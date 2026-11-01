//! Manual release-mode measurements of digital sampling and display-list construction.

use super::*;
use crate::scene::{MonoMeasure, Prim};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;
use volna_trace::data::history::VecHistory;
use volna_trace::remote::memory::MemoryBudget;

const ROWS: usize = 40;
const FRAMES: usize = 120;
const WIDTH: f32 = 1600.0;

struct NormalHistory(Arc<dyn SignalHistory>);

impl SignalHistory for NormalHistory {
    fn shape(&self) -> SignalShape {
        self.0.shape()
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn time(&self, i: usize) -> u64 {
        self.0.time(i)
    }
    fn value(&self, i: Option<usize>) -> WaveValue {
        self.0.value(i)
    }
    fn value_view(&self, i: Option<usize>) -> volna_trace::data::value_view::ValueView<'_> {
        self.0.value_view(i)
    }
    fn bit(&self, i: Option<usize>) -> Bit {
        self.0.bit(i)
    }
    fn always_normal(&self) -> bool {
        true
    }
}

fn measure(mut frame: impl FnMut(usize)) -> (f64, f64) {
    for i in 0..10 {
        frame(i);
    }
    let mut times: Vec<_> = (0..FRAMES)
        .map(|i| {
            let start = Instant::now();
            frame(i);
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    (times[FRAMES / 2], times[FRAMES * 99 / 100])
}

fn profile(name: &str, history: &Arc<dyn SignalHistory>, span: f64) {
    let source = history.clone();
    let history = history.as_ref();
    let theme = Theme::volna(true);
    let translators = crate::data::Translators::builtin();
    let translator = translators.get("hex").unwrap();
    let mut text = TextCache::default();
    let mut scene = Scene::default();
    let viewport = |i: usize| {
        let start = 1000.0 + i as f64 * span / f64::from(WIDTH) * 0.7;
        Viewport {
            start,
            end: start + span,
        }
    };
    // Separate identities prevent repeated rows from sharing sampling tiles in this measurement.
    let rows: Vec<Arc<dyn SignalHistory>> = (0..ROWS)
        .map(|_| {
            struct Row(Arc<dyn SignalHistory>);
            impl SignalHistory for Row {
                fn shape(&self) -> SignalShape {
                    self.0.shape()
                }
                fn len(&self) -> usize {
                    self.0.len()
                }
                fn time(&self, i: usize) -> u64 {
                    self.0.time(i)
                }
                fn index_at(&self, t: u64) -> Option<usize> {
                    self.0.index_at(t)
                }
                fn index_at_hint(&self, t: u64, hint: usize) -> Option<usize> {
                    self.0.index_at_hint(t, hint)
                }

                fn value(&self, i: Option<usize>) -> WaveValue {
                    self.0.value(i)
                }
                fn bit(&self, i: Option<usize>) -> Bit {
                    self.0.bit(i)
                }
                fn value_kind(&self, i: Option<usize>) -> ValueKind {
                    self.0.value_kind(i)
                }
                fn always_normal(&self) -> bool {
                    self.0.always_normal()
                }
            }
            Arc::new(Row(source.clone())) as Arc<dyn SignalHistory>
        })
        .collect();
    let sampling = measure(|i| {
        let grid = SamplingGrid::new(&viewport(i), f64::from(WIDTH));
        for _ in 0..ROWS {
            let mut before = grid.index_before(history, 0, None);
            let mut kinds = marks::KindReader::new(history, history.shape() == SignalShape::Bit);
            for x in 0..grid.len {
                let last = grid.index_before(history, x + 1, Some(before.unwrap_or(0)));
                let count = changes_between(before, last);
                if count > 0 {
                    black_box(kinds.column(before, last.unwrap()));
                } else {
                    black_box(kinds.state(before));
                }
                before = last;
            }
        }
    });
    let painting = measure(|i| {
        scene.clear();
        for (row, history) in rows.iter().enumerate() {
            let history = history.as_ref();
            let area = Rect::from_xywh(0.0, row as f32 * 24.0, WIDTH, 24.0);
            if history.shape() == SignalShape::Bit {
                paint_bit_row(history, &viewport(i), area, &theme, &mut scene);
            } else {
                let mut p = TextPainter {
                    theme: &theme,
                    text: &mut text,
                    measure: &mut MonoMeasure,
                    scene: &mut scene,
                };
                paint_bus_row(
                    history,
                    translator.as_ref(),
                    &viewport(i),
                    area,
                    area,
                    8.0,
                    &mut p,
                );
            }
        }
        black_box(&scene);
    });
    let mut cache = DigitalCache::default();
    let budget = MemoryBudget::new(16 << 20);
    let mut cached_frame = |i| {
        cache.begin_paint_pass();
        scene.clear();
        for (row, history) in rows.iter().enumerate() {
            let area = Rect::from_xywh(0.0, row as f32 * 24.0, WIDTH, 24.0);
            let vp = viewport(i);
            if history.shape() == SignalShape::Bit {
                let grid = SamplingGrid::new(&vp, f64::from(WIDTH));
                let samples = cache.sample(history, &grid, true, &budget);
                paint_bits_sampled(
                    DigitalRow {
                        history: history.as_ref(),
                        samples: samples.as_ref(),
                    },
                    &vp,
                    area,
                    &theme,
                    &mut scene,
                    true,
                    0..WIDTH as usize,
                );
            } else {
                let grid = bus_grid(&vp, f64::from(WIDTH), &theme);
                let samples = cache.sample(history, &grid, false, &budget);
                let mut p = TextPainter {
                    theme: &theme,
                    text: &mut text,
                    measure: &mut MonoMeasure,
                    scene: &mut scene,
                };
                paint_bus_row_sampled(
                    DigitalRow {
                        history: history.as_ref(),
                        samples: samples.as_ref(),
                    },
                    translator.as_ref(),
                    &vp,
                    area,
                    area,
                    8.0,
                    &mut p,
                );
            }
        }
        black_box(&scene);
    };
    let start = Instant::now();
    cached_frame(0);
    let cold_ms = start.elapsed().as_secs_f64() * 1000.0;
    let cached = measure(cached_frame);
    eprintln!(
        "{name}: cached median/p99={:.3}/{:.3} ms; cold={cold_ms:.3} ms; cache bytes={}",
        cached.0,
        cached.1,
        budget.used()
    );
    let fills = scene
        .prims
        .iter()
        .filter(|p| matches!(p, Prim::SubpixelFill { .. }))
        .count();
    let lines: usize = scene
        .prims
        .iter()
        .map(|p| match p {
            Prim::Lines { segments, .. } => segments.len(),
            _ => 0,
        })
        .sum();
    eprintln!(
        "{name}: sampling median/p99={:.3}/{:.3} ms; paint median/p99={:.3}/{:.3} ms; prims={} fills={fills} line_segments={lines}",
        sampling.0,
        sampling.1,
        painting.0,
        painting.1,
        scene.prims.len()
    );
}

#[test]
#[ignore = "Manual profiling: cargo test -p volna-core --release profile_digital_pan -- --ignored --nocapture"]
fn profile_digital_pan() {
    eprintln!(
        "{ROWS} rows, {WIDTH} logical pixels, {FRAMES} fractional-pan frames; resident history reused across rows; excludes layout, GPUI and GPU"
    );
    for (name, shape, mixed, span) in [
        ("bit sparse", SignalShape::Bit, false, 1600.0),
        ("bit dense", SignalShape::Bit, false, 1_000_000.0),
        (
            "bus dense",
            SignalShape::Vector { width: 32 },
            false,
            1_000_000.0,
        ),
        (
            "bus mixed dense",
            SignalShape::Vector { width: 32 },
            true,
            1_000_000.0,
        ),
    ] {
        let width = shape.width() as usize;
        let history = VecHistory {
            shape,
            times: (0..262_144_u64)
                .map(|i| i * 8 + (i.wrapping_mul(7919) % 8))
                .collect(),
            values: (0..262_144)
                .map(|i| {
                    WaveValue::Bits(
                        if mixed && i % 13 == 0 {
                            "x"
                        } else if i % 2 == 0 {
                            "0"
                        } else {
                            "1"
                        }
                        .repeat(width),
                    )
                })
                .collect(),
            initial: WaveValue::Bits("0".repeat(width)),
        };
        let history: Arc<dyn SignalHistory> = Arc::new(history);
        profile(name, &history, span);
        if shape != SignalShape::Bit {
            let packed = volna_trace::remote::history::PackedHistory::from_history_with_limit(
                history.as_ref(),
                u64::MAX,
            )
            .unwrap();
            let packed: Arc<dyn SignalHistory> = Arc::new(packed);
            profile(&format!("{name} packed"), &packed, span);
            if !mixed {
                let normal: Arc<dyn SignalHistory> = Arc::new(NormalHistory(history.clone()));
                profile("bus dense normal metadata", &normal, span);
            }
        }
    }
    let session = volna_trace::session::OpenSpec::Path(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../volna/examples/picorv32.vtr"),
    )
    .open()
    .unwrap();
    for name in ["mem_axi_arready", "mem_axi_rdata", "trace_data"] {
        let var = session
            .hierarchy()
            .vars()
            .find(|v| v.name.starts_with(name))
            .unwrap();
        let history = session.load_signal(var.signal).unwrap();
        profile(name, &history, 4_005_000.0);
    }
}
