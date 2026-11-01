//! Source-aligned time bins for bounded digital waveform sampling.

use crate::wave::marks::{KindReader, Kinds};
use crate::wave::viewport::Viewport;
use std::sync::{Arc, Weak};
use volna_trace::data::SignalHistory;
use volna_trace::remote::memory::{MemoryBudget, Reservation};

// Power-of-two tick bins keep density decisions independent of viewport position.
// Integer boundaries also avoid rounding a transition into an adjacent bin.
pub(super) struct SamplingGrid {
    first: i128,
    step: i128,
    pub(super) pixel: f32,
    pub(super) left: f32,
    pub(super) len: usize,
}

impl SamplingGrid {
    pub(super) fn new(vp: &Viewport, width: f64) -> Self {
        let unit = vp.width() / width;
        // Subtracting panned endpoints can move an exact LOD boundary by a few ulps.
        let uncertainty = 4.0 * f64::EPSILON * vp.start.abs().max(vp.end.abs()) / width;
        let shift = (unit + uncertainty).log2().floor().clamp(0.0, 63.0) as u32;
        let step = 1_i128 << shift;
        let first = (vp.start.floor() as i128).div_euclid(step) * step;
        let relative = (vp.start.floor() as i128 - first) as f64 + (vp.start - vp.start.floor());
        let pixel = step as f64 / unit;
        Self {
            first,
            step,
            pixel: pixel as f32,
            left: (-relative / unit) as f32,
            len: ((relative + vp.width()) / step as f64).ceil().max(0.0) as usize,
        }
    }

    pub(super) fn with_padding(mut self, pixels: f32) -> Self {
        let cells = (pixels / self.pixel).ceil() as usize;
        self.first -= cells as i128 * self.step;
        self.left -= cells as f32 * self.pixel;
        self.len += 2 * cells;
        self
    }

    pub(super) fn index_before(
        &self,
        h: &dyn SignalHistory,
        boundary: usize,
        hint: Option<usize>,
    ) -> Option<usize> {
        let tick = self.first + boundary as i128 * self.step - 1;
        if tick < 0 {
            return None;
        }
        let tick = tick.min(i128::from(u64::MAX)) as u64;
        match hint {
            Some(hint) => h.index_at_hint(tick, hint),
            None => h.index_at(tick),
        }
    }
}

const TILE_BINS: usize = 256;
const MAX_TILES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Column {
    pub end: Option<usize>,
    pub kinds: Kinds,
}

pub(super) struct Samples {
    pub before: Option<usize>,
    pub columns: Vec<Column>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    history: usize,
    step: i128,
    tile: i128,
    bit: bool,
}

struct Tile {
    // Keep the address unavailable for reuse without retaining the signal payload.
    source: Weak<dyn SignalHistory>,
    before: Option<usize>,
    columns: Box<[Column; TILE_BINS]>,
    used: u64,
    _reservation: Reservation,
}

struct Entry {
    key: Key,
    tile: Tile,
}

type Slots = [Option<Entry>; MAX_TILES];

struct TileStore {
    slots: Box<Slots>,
    len: usize,
    _reservation: Reservation,
}

impl TileStore {
    fn new(budget: &MemoryBudget) -> anyhow::Result<Self> {
        let held =
            budget.reserve_object("waveform sample slots", std::mem::size_of::<Slots>() as u64)?;
        Ok(Self {
            slots: Box::new(std::array::from_fn(|_| None)),
            len: 0,
            _reservation: held,
        })
    }

    fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.slots[..self.len]
            .iter()
            .map(|entry| entry.as_ref().expect("occupied sample slot"))
    }

    fn find(&self, key: Key) -> Result<usize, usize> {
        self.slots[..self.len].binary_search_by_key(&key, |entry| {
            entry.as_ref().expect("occupied sample slot").key
        })
    }

    fn insert(&mut self, key: Key, tile: Tile) {
        let index = self.find(key).expect_err("sample tile is new");
        self.slots[index..=self.len].rotate_right(1);
        self.slots[index] = Some(Entry { key, tile });
        self.len += 1;
    }

    fn remove(&mut self, index: usize) {
        self.slots[index..self.len].rotate_left(1);
        self.len -= 1;
        self.slots[self.len] = None;
    }

    fn prune(&mut self) {
        let mut live = 0;
        for index in 0..self.len {
            if self.slots[index]
                .as_ref()
                .expect("occupied sample slot")
                .tile
                .source
                .strong_count()
                > 0
            {
                self.slots.swap(live, index);
                live += 1;
            }
        }
        self.slots[live..self.len].fill_with(|| None);
        self.len = live;
    }
}

#[derive(Default)]
pub(crate) struct DigitalCache {
    tiles: Option<TileStore>,
    clock: u64,
    paint_start: u64,
}

impl DigitalCache {
    pub(crate) fn prune(&mut self) {
        if let Some(tiles) = &mut self.tiles {
            tiles.prune();
            if tiles.len == 0 {
                self.tiles = None;
            }
        }
    }

    pub(super) fn begin_paint_pass(&mut self) {
        self.prune();
        self.paint_start = self.clock;
    }

    fn evict(&mut self) -> bool {
        let Some(tiles) = &mut self.tiles else {
            return false;
        };
        let index = tiles
            .entries()
            .enumerate()
            // Preserve the current panel paint's working set when it exceeds cache capacity.
            .filter(|(_, entry)| entry.tile.used <= self.paint_start)
            .min_by_key(|(_, entry)| entry.tile.used)
            .map(|(index, _)| index);
        if let Some(index) = index {
            tiles.remove(index);
            true
        } else {
            false
        }
    }

    pub(super) fn sample(
        &mut self,
        history: &Arc<dyn SignalHistory>,
        grid: &SamplingGrid,
        bit: bool,
        budget: &MemoryBudget,
    ) -> Option<Samples> {
        if grid.len == 0 || grid.step == 1 {
            return None;
        }
        let before = grid.index_before(history.as_ref(), 0, None);
        let end = grid.index_before(history.as_ref(), grid.len, Some(before.unwrap_or(0)));
        if super::changes_between(before, end) <= grid.len {
            return None;
        }
        if self.tiles.is_none() {
            self.tiles = Some(TileStore::new(budget).ok()?);
        }
        let mut samples = Samples {
            before: None,
            columns: Vec::with_capacity(grid.len),
        };
        let first = grid.first.div_euclid(grid.step);
        let mut offset = 0;
        while offset < grid.len {
            let bin = first + offset as i128;
            let key = Key {
                history: Arc::as_ptr(history).cast::<()>() as usize,
                step: grid.step,
                tile: bin.div_euclid(TILE_BINS as i128),
                bit,
            };
            if self.tiles.as_ref()?.find(key).is_err() {
                if self.tiles.as_ref()?.len >= MAX_TILES && !self.evict() {
                    return None;
                }
                let bytes = std::mem::size_of::<[Column; TILE_BINS]>() as u64;
                let held = loop {
                    match budget.reserve_object("waveform samples", bytes) {
                        Ok(held) => break held,
                        Err(_) if self.evict() => {}
                        Err(_) => return None,
                    }
                };
                let tile_grid = SamplingGrid {
                    first: key.tile * TILE_BINS as i128 * grid.step,
                    step: grid.step,
                    pixel: grid.pixel,
                    left: 0.0,
                    len: TILE_BINS,
                };
                let before = tile_grid.index_before(history.as_ref(), 0, None);
                let mut index = before;
                let mut kinds = KindReader::new(history.as_ref(), bit);
                let columns = Box::new(std::array::from_fn::<_, TILE_BINS, _>(|x| {
                    let end =
                        tile_grid.index_before(history.as_ref(), x + 1, Some(index.unwrap_or(0)));
                    let state = if end == index {
                        kinds.state(index)
                    } else {
                        kinds.column(index, end.expect("advancing bin ends on a change"))
                    };
                    index = end;
                    Column { end, kinds: state }
                }));
                self.tiles.as_mut()?.insert(
                    key,
                    Tile {
                        source: Arc::downgrade(history),
                        before,
                        columns,
                        used: self.clock,
                        _reservation: held,
                    },
                );
            }
            self.clock = self.clock.wrapping_add(1);
            let tiles = self.tiles.as_mut()?;
            let index = tiles.find(key).expect("requested tile is resident");
            let tile = &mut tiles.slots[index]
                .as_mut()
                .expect("occupied sample slot")
                .tile;
            tile.used = self.clock;
            let start = bin.rem_euclid(TILE_BINS as i128) as usize;
            let count = (TILE_BINS - start).min(grid.len - offset);
            if offset == 0 {
                samples.before = if start == 0 {
                    tile.before
                } else {
                    tile.columns[start - 1].end
                };
            }
            samples
                .columns
                .extend_from_slice(&tile.columns[start..start + count]);
            offset += count;
        }
        Some(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_trace::data::history::VecHistory;
    use volna_trace::data::{SignalShape, WaveValue};

    struct Counted {
        reads: std::sync::atomic::AtomicUsize,
    }

    impl SignalHistory for Counted {
        fn shape(&self) -> SignalShape {
            SignalShape::Bit
        }
        fn len(&self) -> usize {
            1 << 30
        }
        fn time(&self, i: usize) -> u64 {
            self.reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            i as u64
        }
        fn value(&self, i: Option<usize>) -> WaveValue {
            WaveValue::Bits(
                if i.unwrap_or(0).is_multiple_of(2) {
                    "0"
                } else {
                    "x"
                }
                .into(),
            )
        }
        fn bit(&self, i: Option<usize>) -> volna_trace::data::Bit {
            if i.unwrap_or(0).is_multiple_of(2) {
                volna_trace::data::Bit::Zero
            } else {
                volna_trace::data::Bit::X
            }
        }
    }

    #[test]
    fn overlapping_pans_reuse_bins_and_eviction_releases_budget_without_retaining_history() {
        use std::sync::atomic::Ordering;
        let source = Arc::new(Counted { reads: 0.into() });
        let history: Arc<dyn SignalHistory> = source.clone();
        let budget = MemoryBudget::new(16 << 20);
        let mut cache = DigitalCache::default();
        let grid = SamplingGrid::new(
            &Viewport {
                start: 100_000.0,
                end: 104_096.0,
            },
            512.0,
        );
        let first = cache.sample(&history, &grid, true, &budget).unwrap();
        let cold = source.reads.swap(0, Ordering::Relaxed);
        let again = cache.sample(&history, &grid, true, &budget).unwrap();
        let warm = source.reads.swap(0, Ordering::Relaxed);
        assert_eq!(first.before, again.before);
        assert_eq!(first.columns, again.columns);
        assert!(cold > 10 * warm, "{cold} cold reads vs {warm} warm reads");
        let grid = SamplingGrid::new(
            &Viewport {
                start: 100_008.0,
                end: 104_104.0,
            },
            512.0,
        );
        let panned = cache.sample(&history, &grid, true, &budget).unwrap();
        assert_eq!(
            first.columns[1..],
            panned.columns[..panned.columns.len() - 1]
        );
        assert!(source.reads.swap(0, Ordering::Relaxed) < cold / 10);
        for i in 0..MAX_TILES + 4 {
            cache.begin_paint_pass();
            let start = (i * TILE_BINS * 512) as f64;
            let grid = SamplingGrid::new(
                &Viewport {
                    start,
                    end: start + 512.0,
                },
                1.0,
            );
            cache.sample(&history, &grid, true, &budget).unwrap();
        }
        assert_eq!(cache.tiles.as_ref().unwrap().len, MAX_TILES);
        cache.begin_paint_pass();
        let keys: Vec<_> = cache
            .tiles
            .as_ref()
            .unwrap()
            .entries()
            .map(|entry| entry.key)
            .collect();
        for key in &keys {
            let start = (key.tile * TILE_BINS as i128 * key.step) as f64;
            let grid = SamplingGrid::new(
                &Viewport {
                    start,
                    end: start + key.step as f64,
                },
                1.0,
            );
            cache.sample(&history, &grid, true, &budget).unwrap();
        }
        let start = ((MAX_TILES + 100) * TILE_BINS * 512) as f64;
        let grid = SamplingGrid::new(
            &Viewport {
                start,
                end: start + 512.0,
            },
            1.0,
        );
        assert!(cache.sample(&history, &grid, true, &budget).is_none());
        assert!(
            keys.iter()
                .all(|key| cache.tiles.as_ref().unwrap().find(*key).is_ok())
        );
        assert!(budget.used() < 8 << 20);
        assert_eq!(Arc::strong_count(&source), 2);
        drop(history);
        drop(source);
        cache.prune();
        assert!(cache.tiles.is_none());
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn refusal_does_not_publish_partial_samples_or_leak_admission() {
        let history: Arc<dyn SignalHistory> = Arc::new(Counted { reads: 0.into() });
        let budget = MemoryBudget::new(0);
        let mut cache = DigitalCache::default();
        let grid = SamplingGrid::new(
            &Viewport {
                start: 0.0,
                end: 4096.0,
            },
            512.0,
        );
        assert!(cache.sample(&history, &grid, true, &budget).is_none());
        assert_eq!(budget.used(), 0);
        budget.set_limit(16 << 20);
        assert!(cache.sample(&history, &grid, true, &budget).is_some());
        assert!(budget.used() > 0);
        budget.set_object_limit(1);
        cache.begin_paint_pass();
        let other = SamplingGrid::new(
            &Viewport {
                start: 200_000.0,
                end: 204_096.0,
            },
            512.0,
        );
        assert!(cache.sample(&history, &other, true, &budget).is_none());
        assert_eq!(budget.used(), std::mem::size_of::<Slots>() as u64);
        assert_eq!(cache.tiles.as_ref().unwrap().len, 0);
        cache.prune();
        assert_eq!(budget.used(), 0);
        assert!(cache.tiles.is_none());
    }

    #[test]
    fn storage_admission_tracks_slots_and_tiles_and_pruning_preserves_the_paint_pass() {
        let history: Arc<dyn SignalHistory> = Arc::new(Counted { reads: 0.into() });
        let slots = std::mem::size_of::<Slots>() as u64;
        let tile = std::mem::size_of::<[Column; TILE_BINS]>() as u64;
        let budget = MemoryBudget::new(slots - 1);
        let mut cache = DigitalCache::default();
        let grid = SamplingGrid::new(
            &Viewport {
                start: 0.0,
                end: 512.0,
            },
            1.0,
        );
        assert!(cache.sample(&history, &grid, true, &budget).is_none());
        assert!(cache.tiles.is_none());
        assert_eq!(budget.used(), 0);

        budget.set_limit(slots);
        assert!(cache.sample(&history, &grid, true, &budget).is_none());
        assert_eq!(budget.used(), slots);
        assert_eq!(cache.tiles.as_ref().unwrap().len, 0);
        cache.prune();
        assert_eq!(budget.used(), 0);

        budget.set_limit(slots + tile);
        cache.begin_paint_pass();
        assert!(cache.sample(&history, &grid, true, &budget).is_some());
        assert_eq!(budget.used(), slots + tile);
        let first_key = cache.tiles.as_ref().unwrap().entries().next().unwrap().key;
        let start = (TILE_BINS * 512) as f64;
        let other = SamplingGrid::new(
            &Viewport {
                start,
                end: start + 512.0,
            },
            1.0,
        );
        cache.prune();
        assert!(cache.sample(&history, &other, true, &budget).is_none());
        assert!(cache.tiles.as_ref().unwrap().find(first_key).is_ok());
        assert_eq!(budget.used(), slots + tile);

        cache.begin_paint_pass();
        let samples = cache.sample(&history, &other, true, &budget).unwrap();
        assert_eq!(samples.before, Some(start as usize - 1));
        assert!(cache.tiles.as_ref().unwrap().find(first_key).is_err());
        assert_eq!(budget.used(), slots + tile);
        drop(cache);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn pruning_compacts_dead_entries_without_invalidating_live_lookups() {
        use std::sync::atomic::Ordering;
        let source = Arc::new(Counted { reads: 0.into() });
        let live: Arc<dyn SignalHistory> = source.clone();
        let dead: Arc<dyn SignalHistory> = Arc::new(Counted { reads: 0.into() });
        let budget = MemoryBudget::new(1 << 20);
        let mut cache = DigitalCache::default();
        let grid = SamplingGrid::new(
            &Viewport {
                start: 0.0,
                end: 4096.0,
            },
            512.0,
        );
        let expected = cache.sample(&live, &grid, true, &budget).unwrap();
        cache.sample(&dead, &grid, true, &budget).unwrap();
        let allocated = budget.used();
        drop(dead);
        cache.prune();
        assert!(budget.used() < allocated);
        source.reads.store(0, Ordering::Relaxed);
        let actual = cache.sample(&live, &grid, true, &budget).unwrap();
        assert_eq!(actual.before, expected.before);
        assert_eq!(actual.columns, expected.columns);
        assert!(source.reads.load(Ordering::Relaxed) < 100);
        drop(live);
        drop(source);
        cache.prune();
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn late_bin_boundaries_are_exact() {
        let origin = 1_u64 << 60;
        let history = VecHistory {
            shape: SignalShape::Bit,
            times: vec![origin + 31, origin + 32, origin + 33],
            values: vec![
                WaveValue::Bits("1".into()),
                WaveValue::Bits("0".into()),
                WaveValue::Bits("1".into()),
            ],
            initial: WaveValue::Unavailable,
        };
        let vp = Viewport {
            start: origin as f64,
            end: origin as f64 + 4096.0,
        };
        let grid = SamplingGrid::new(&vp, 512.0);
        assert_eq!(grid.index_before(&history, 4, None), Some(0));
        assert_eq!(grid.index_before(&history, 5, None), Some(2));
        let panned = SamplingGrid::new(
            &Viewport {
                start: vp.start + 256.0,
                end: vp.end + 256.0,
            },
            512.0,
        );
        assert_eq!(panned.first, grid.first + 256);
        assert_eq!(panned.step, grid.step);
    }

    #[test]
    fn sampling_work_stays_bounded_by_view_width() {
        for (start, span) in [
            (-123.25, 4005.0),
            (0.0, 0.25),
            (0.0, 1e12),
            ((1_u64 << 60) as f64, 4096.0),
        ] {
            let grid = SamplingGrid::new(
                &Viewport {
                    start,
                    end: start + span,
                },
                500.25,
            );
            assert!(grid.len <= 2 * 501 + 1, "{start}, {span}");
            assert_eq!(grid.first.rem_euclid(grid.step), 0);
            assert!(grid.left <= 0.0);
            assert!(grid.left + grid.len as f32 * grid.pixel >= 500.25 - 0.001);
        }
    }
}
