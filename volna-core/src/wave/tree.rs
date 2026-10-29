//! Wave rows in pre-order, with group membership and bus ownership validated on edits.

use std::collections::BTreeSet;
use std::ops::{Deref, DerefMut, Range, RangeInclusive};
use std::sync::Arc;

use super::model::WaveRow;
pub use crate::history::Splice;

pub const MAX_DEPTH: u8 = 8;

#[derive(Clone)]
pub struct Entry {
    pub depth: u8,
    pub row: WaveRow,
}

impl Entry {
    #[must_use]
    pub fn new(depth: u8, row: WaveRow) -> Self {
        Self { depth, row }
    }

    #[must_use]
    pub fn is_group(&self) -> bool {
        matches!(self.row, WaveRow::Group(_))
    }

    #[must_use]
    pub fn detached(&self) -> Self {
        Self {
            depth: self.depth,
            row: self.row.detached(),
        }
    }

    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        self.depth == other.depth && self.row.same(&other.row)
    }
}

impl Deref for Entry {
    type Target = WaveRow;
    fn deref(&self) -> &WaveRow {
        &self.row
    }
}

impl DerefMut for Entry {
    fn deref_mut(&mut self) -> &mut WaveRow {
        &mut self.row
    }
}

pub fn validate(items: &[Entry]) -> Result<(), String> {
    validate_entries(items.iter().collect())
}

fn validate_entries(entries: Vec<&Entry>) -> Result<(), String> {
    let mut parents: Vec<&Entry> = Vec::new();
    for (i, e) in entries.into_iter().enumerate() {
        let depth = usize::from(e.depth);
        if e.depth >= MAX_DEPTH {
            return Err(format!("row {i}: groups nested deeper than {MAX_DEPTH}"));
        }
        if depth > parents.len() {
            return Err(format!("row {i}: depth {depth} below a non-container"));
        }
        parents.truncate(depth);
        if let Some(parent) = parents.last()
            && let Some(bus) = parent.signal()
        {
            let valid = bus.bus.is_some_and(|b| b.initialized)
                && e.signal().is_some_and(|s| {
                    s.source == bus.source
                        && s.source_shape == bus.source_shape
                        && s.projection.is_some_and(|p| {
                            p.first == p.last
                                && p.validate(bus.source_shape).is_ok()
                                && bus.projection.is_none_or(|slice| slice.contains(p.first))
                        })
                });
            if !valid {
                return Err(format!("row {i}: a bus can contain only its own bits"));
            }
        }
        if e.collapsed().is_some() {
            parents.push(e);
        }
    }
    Ok(())
}

pub(crate) fn validate_splices(items: &[Entry], splices: &[Splice]) -> Result<(), String> {
    let mut entries: Vec<&Entry> = items.iter().collect();
    for s in splices {
        if s.at > entries.len() || s.remove > entries.len() - s.at {
            return Err(format!(
                "splice {}+{} outside {} rows",
                s.at,
                s.remove,
                entries.len()
            ));
        }
        entries.splice(s.at..s.at + s.remove, s.insert.iter());
    }
    validate_entries(entries)
}

pub fn apply(items: &mut Vec<Entry>, splices: Vec<Splice>) -> Result<Vec<Splice>, String> {
    validate_splices(items, &splices)?;
    let mut inverse = Vec::with_capacity(splices.len());
    for s in splices {
        let inserted = s.insert.len();
        let removed = items
            .splice(s.at..s.at + s.remove, s.insert)
            .map(|e| e.detached())
            .collect();
        inverse.push(Splice {
            at: s.at,
            remove: inserted,
            insert: removed,
        });
    }
    inverse.reverse();
    Ok(inverse)
}

#[must_use]
pub fn subtree_end(items: &[Entry], i: usize) -> usize {
    let depth = items[i].depth;
    i + 1
        + items[i + 1..]
            .iter()
            .take_while(|e| e.depth > depth)
            .count()
}

#[must_use]
pub fn subtree(items: &[Entry], i: usize) -> Range<usize> {
    i..subtree_end(items, i)
}

#[must_use]
pub fn parent(items: &[Entry], i: usize) -> Option<usize> {
    let depth = items.get(i)?.depth;
    (0..i).rev().find(|&j| items[j].depth < depth)
}

#[must_use]
pub fn is_within(items: &[Entry], i: usize, ancestor: usize) -> bool {
    subtree(items, ancestor).contains(&i)
}

#[must_use]
pub fn visible(items: &[Entry]) -> Arc<[u32]> {
    let mut out = Vec::with_capacity(items.len());
    let mut hide_below: Option<u8> = None;
    for (i, e) in items.iter().enumerate() {
        if hide_below.is_some_and(|d| e.depth > d) {
            continue;
        }
        hide_below = None;
        out.push(i as u32);
        if e.collapsed() == Some(true) {
            hide_below = Some(e.depth);
        }
    }
    out.into()
}

#[must_use]
pub fn roots(items: &[Entry], sel: &BTreeSet<usize>) -> Vec<usize> {
    let mut out = Vec::new();
    let mut end = 0;
    for &i in sel.iter().filter(|&&i| i < items.len()) {
        if i < end {
            continue;
        }
        out.push(i);
        end = subtree_end(items, i);
    }
    out
}

#[must_use]
pub fn blocks(items: &[Entry], sel: &BTreeSet<usize>) -> Vec<Range<usize>> {
    roots(items, sel)
        .into_iter()
        .map(|i| subtree(items, i))
        .collect()
}

pub fn leaves(items: &[Entry], i: usize) -> impl Iterator<Item = usize> + '_ {
    let mut skip = i;
    subtree(items, i).filter(move |&j| {
        if j < skip {
            return false;
        }
        if items[j].signal().is_some_and(|s| s.bus.is_some()) {
            skip = subtree_end(items, j);
        }
        !items[j].is_group()
    })
}

#[must_use]
pub fn selected_leaves(items: &[Entry], sel: &BTreeSet<usize>) -> Vec<usize> {
    roots(items, sel)
        .into_iter()
        .flat_map(|i| leaves(items, i))
        .collect()
}

#[must_use]
pub fn gap_depths(items: &[Entry], visible: &[u32], gap: usize) -> RangeInclusive<u8> {
    let above = gap.checked_sub(1).and_then(|g| visible.get(g));
    let below = visible.get(gap);
    let max = above.map_or(0, |&a| {
        let e = &items[a as usize];
        let open = matches!(&e.row, WaveRow::Group(g) if !g.collapsed);
        e.depth + u8::from(open)
    });
    let min = below.map_or(0, |&b| items[b as usize].depth);
    min..=max.min(MAX_DEPTH - 1).max(min)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub at: usize,
    pub depth: u8,
}

impl Place {
    #[must_use]
    pub fn gap(items: &[Entry], visible: &[u32], gap: usize, depth: u8) -> Self {
        Self {
            at: visible.get(gap).map_or(items.len(), |&i| i as usize),
            depth,
        }
    }

    #[must_use]
    pub fn into(items: &[Entry], group: usize) -> Self {
        Self {
            at: subtree_end(items, group),
            depth: items[group].depth + 1,
        }
    }
}

#[derive(Clone)]
pub struct Moved {
    pub splices: Vec<Splice>,
    pub roots: BTreeSet<usize>,
    pub first: usize,
}

#[must_use]
pub fn plan_move(items: &[Entry], sel: &BTreeSet<usize>, to: Place) -> Option<Moved> {
    let blocks = blocks(items, sel);
    if blocks.is_empty() || to.at > items.len() {
        return None;
    }
    if blocks.iter().any(|b| to.at > b.start && to.at < b.end) {
        return None;
    }
    let mut moved = Vec::new();
    let mut rest = Vec::new();
    let mut next = blocks.iter().peekable();
    let mut current: Option<&Range<usize>> = None;
    for (i, e) in items.iter().enumerate() {
        if current.is_none_or(|b| i >= b.end)
            && let Some(b) = next.next_if(|b| b.start == i)
        {
            current = Some(b);
        }
        match current.filter(|b| b.contains(&i)) {
            Some(b) => {
                let depth =
                    u32::from(e.depth) - u32::from(items[b.start].depth) + u32::from(to.depth);
                if depth >= u32::from(MAX_DEPTH) {
                    return None;
                }
                moved.push((i, depth as u8));
            }
            None => rest.push((i, e.depth)),
        }
    }
    let first = rest.iter().filter(|(i, _)| *i < to.at).count();
    let mut order = rest[..first].to_vec();
    order.extend_from_slice(&moved);
    order.extend_from_slice(&rest[first..]);
    if order
        .iter()
        .enumerate()
        .all(|(k, &(i, d))| d == items[k].depth && (i == k || items[i].row.same(&items[k].row)))
    {
        return None;
    }
    let mut limit = 0u8;
    for &(i, d) in &order {
        if d > limit {
            return None;
        }
        limit = d + u8::from(items[i].collapsed().is_some());
    }
    let mut roots = BTreeSet::new();
    let mut k = first;
    for b in &blocks {
        roots.insert(k);
        k += b.len();
    }
    let mut splices = removal_of(&blocks);
    splices.push(Splice {
        at: first,
        remove: 0,
        insert: moved
            .iter()
            .map(|&(i, depth)| Entry {
                depth,
                row: items[i].row.clone(),
            })
            .collect(),
    });
    validate_splices(items, &splices).ok()?;
    Some(Moved {
        splices,
        roots,
        first,
    })
}

fn removal_of(blocks: &[Range<usize>]) -> Vec<Splice> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for b in blocks {
        match runs.last_mut() {
            Some(run) if run.end == b.start => run.end = b.end,
            _ => runs.push(b.clone()),
        }
    }
    runs.into_iter()
        .rev()
        .map(|b| Splice {
            at: b.start,
            remove: b.len(),
            insert: Vec::new(),
        })
        .collect()
}

#[must_use]
pub fn group(
    items: &[Entry],
    sel: &BTreeSet<usize>,
    group: WaveRow,
) -> Option<(Vec<Splice>, usize)> {
    let blocks = blocks(items, sel);
    let first = blocks.first()?.start;
    let depth = items[first].depth;
    let too_deep = blocks.iter().any(|b| {
        items[b.clone()]
            .iter()
            .any(|e| e.depth - items[b.start].depth + depth + 1 >= MAX_DEPTH)
    });
    if too_deep {
        return None;
    }
    let mut inserted = vec![Entry::new(depth, group)];
    for b in &blocks {
        let base = items[b.start].depth;
        inserted.extend(items[b.clone()].iter().map(|e| Entry {
            depth: e.depth - base + depth + 1,
            row: e.row.clone(),
        }));
    }
    let mut splices = removal_of(&blocks);
    splices.push(Splice {
        at: first,
        remove: 0,
        insert: inserted,
    });
    validate_splices(items, &splices).ok()?;
    Some((splices, first))
}

#[must_use]
pub fn ungroup(items: &[Entry], groups: &BTreeSet<usize>) -> (Vec<Splice>, BTreeSet<usize>) {
    let groups: BTreeSet<usize> = groups
        .iter()
        .copied()
        .filter(|&g| items.get(g).is_some_and(Entry::is_group))
        .collect();
    let mut splices = Vec::new();
    let mut children = BTreeSet::new();
    let mut shift = 0;
    for span in blocks(items, &groups) {
        let spans: Vec<(usize, u8, Range<usize>)> = groups
            .range(span.clone())
            .map(|&g| (g, items[g].depth, subtree(items, g)))
            .collect();
        let at = span.start - shift;
        let mut insert = Vec::with_capacity(span.len());
        for i in span.clone() {
            if spans.iter().any(|(g, ..)| *g == i) {
                continue;
            }
            let mut lift = 0;
            for (_, depth, _) in spans.iter().filter(|(g, _, s)| i > *g && s.contains(&i)) {
                lift += 1;
                if items[i].depth == depth + 1 {
                    children.insert(at + insert.len());
                }
            }
            let mut row = items[i].row.clone();
            if row.tint().is_none()
                && let Some(p) = parent(items, i).filter(|p| groups.contains(p))
            {
                let mut at = Some(p);
                while let Some(g) = at.filter(|g| groups.contains(g)) {
                    if let Some(t) = items[g].row.tint() {
                        row.set_tint(Some(t));
                        break;
                    }
                    at = parent(items, g);
                }
            }
            insert.push(Entry {
                depth: items[i].depth - lift,
                row,
            });
        }
        shift += span.len() - insert.len();
        splices.push(Splice {
            at,
            remove: span.len(),
            insert,
        });
    }
    (splices, children)
}

#[must_use]
pub fn of_trace(items: &[Entry], trace: crate::trace::TraceId) -> BTreeSet<usize> {
    let of = |j: usize| items[j].row.trace() == Some(trace);
    (0..items.len())
        .filter(|&i| {
            if !items[i].is_group() {
                return of(i);
            }
            let mut leaves = leaves(items, i).peekable();
            leaves.peek().is_some() && leaves.all(of)
        })
        .collect()
}

#[must_use]
pub fn removal(items: &[Entry], sel: &BTreeSet<usize>) -> Vec<Splice> {
    removal_of(&blocks(items, sel))
}

#[must_use]
pub fn extract(items: &[Entry], sel: &BTreeSet<usize>) -> Vec<Entry> {
    blocks(items, sel)
        .into_iter()
        .flat_map(|b| {
            let base = items[b.start].depth;
            items[b].iter().map(move |e| Entry {
                depth: e.depth - base,
                row: e.row.clone(),
            })
        })
        .collect()
}

#[must_use]
pub fn insertion(to: Place, rows: Vec<Entry>) -> Option<(Splice, BTreeSet<usize>)> {
    if rows.iter().any(|e| e.depth + to.depth >= MAX_DEPTH) {
        return None;
    }
    let roots = rows
        .iter()
        .enumerate()
        .filter(|(_, e)| e.depth == 0)
        .map(|(k, _)| to.at + k)
        .collect();
    let insert = rows
        .into_iter()
        .map(|e| Entry {
            depth: e.depth + to.depth,
            row: e.row,
        })
        .collect();
    Some((
        Splice {
            at: to.at,
            remove: 0,
            insert,
        },
        roots,
    ))
}

pub fn set_collapsed(items: &mut [Entry], i: usize, collapsed: bool, deep: bool) -> bool {
    let end = if deep { subtree_end(items, i) } else { i + 1 };
    let mut changed = false;
    for e in &mut items[i..end] {
        changed |= e.row.set_collapsed(collapsed);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wave::model::{ClockRow, GroupRow};

    fn leaf(depth: u8, name: &str) -> Entry {
        Entry::new(
            depth,
            WaveRow::Clock(ClockRow::new(crate::trace::Traced::new(
                crate::trace::TraceId::A,
                name.to_owned(),
            ))),
        )
    }

    fn group(depth: u8, name: &str, collapsed: bool) -> Entry {
        let mut g = GroupRow::new(name);
        g.collapsed = collapsed;
        Entry::new(depth, WaveRow::Group(g))
    }

    fn soc() -> Vec<Entry> {
        vec![
            leaf(0, "clk"),
            group(0, "AXI", false),
            group(1, "Write", false),
            leaf(2, "aw"),
            leaf(2, "w"),
            group(1, "Read", true),
            leaf(2, "ar"),
            leaf(2, "r"),
            leaf(0, "irq"),
        ]
    }

    fn set(rows: &[usize]) -> BTreeSet<usize> {
        rows.iter().copied().collect()
    }

    fn names(items: &[Entry]) -> Vec<(u8, &str)> {
        items.iter().map(|e| (e.depth, e.name())).collect()
    }

    fn round_trip(items: &[Entry], splices: Vec<Splice>) -> Vec<Entry> {
        let mut edited = items.to_vec();
        let inverse = apply(&mut edited, splices).expect("valid splices");
        assert!(validate(&edited).is_ok());
        let mut undone = edited.clone();
        let redo = apply(&mut undone, inverse).expect("valid inverse");
        assert_eq!(names(&undone), names(items));
        assert!(
            undone
                .iter()
                .zip(items)
                .all(|(a, b)| a.same(b) && a.group() == b.group())
        );
        let mut redone = undone;
        apply(&mut redone, redo).expect("valid redo");
        assert_eq!(names(&redone), names(&edited));
        edited
    }

    #[test]
    fn the_invariant_rejects_orphans_and_depth() {
        assert!(validate(&soc()).is_ok());
        assert!(
            validate(&[leaf(1, "a")]).is_err(),
            "first row below the top"
        );
        assert!(
            validate(&[leaf(0, "a"), leaf(1, "b")]).is_err(),
            "child of a signal"
        );
        let deep: Vec<Entry> = (0..MAX_DEPTH).map(|d| group(d, "g", false)).collect();
        assert!(validate(&deep).is_ok());
        let mut deeper = deep;
        deeper.push(leaf(MAX_DEPTH, "x"));
        assert!(validate(&deeper).is_err());
    }

    #[test]
    fn subtrees_parents_leaves_and_visible_rows() {
        let items = soc();
        assert_eq!(subtree(&items, 1), 1..8);
        assert_eq!(subtree(&items, 3), 3..4);
        assert_eq!(parent(&items, 4), Some(2));
        assert_eq!(parent(&items, 2), Some(1));
        assert_eq!(parent(&items, 1), None);
        assert_eq!(leaves(&items, 1).collect::<Vec<_>>(), [3, 4, 6, 7]);
        assert_eq!(&*visible(&items), &[0, 1, 2, 3, 4, 5, 8]);
        assert_eq!(roots(&items, &set(&[2, 3, 8])), [2, 8]);
        assert_eq!(selected_leaves(&items, &set(&[2, 3, 8])), [3, 4, 8]);
    }

    #[test]
    fn gaps_offer_the_levels_between_their_neighbours() {
        let items = soc();
        let v = visible(&items);
        assert_eq!(gap_depths(&items, &v, 0), 0..=0);
        assert_eq!(gap_depths(&items, &v, 2), 1..=1);
        assert_eq!(gap_depths(&items, &v, 5), 1..=2);
        assert_eq!(gap_depths(&items, &v, 6), 0..=1);
        assert_eq!(gap_depths(&items, &v, 7), 0..=0);
    }

    #[test]
    fn moves_refuse_cycles_depth_and_no_ops() {
        let items = soc();
        let v = visible(&items);
        assert!(plan_move(&items, &set(&[1]), Place { at: 3, depth: 2 }).is_none());
        assert!(plan_move(&items, &set(&[0]), Place { at: 0, depth: 0 }).is_none());
        let to = Place::into(&items, 5);
        assert_eq!(to, Place { at: 8, depth: 2 });
        let moved = plan_move(&items, &set(&[8]), to).unwrap();
        assert_eq!(moved.roots, set(&[8]));
        let m = round_trip(&items, moved.splices);
        assert_eq!(m[8].name(), "irq");
        assert_eq!(m[8].depth, 2);
        assert_eq!(parent(&m, 8), Some(5));
        let to = Place::gap(&items, &v, 6, 1);
        let moved = plan_move(&items, &set(&[0]), to).unwrap();
        let m = round_trip(&items, moved.splices);
        assert_eq!(m[7].name(), "clk");
        assert_eq!(parent(&m, 7), Some(0));
        assert_eq!(moved_roots(&m), ["AXI", "irq"]);
        let moved = plan_move(&items, &set(&[0, 8]), Place { at: 3, depth: 2 }).unwrap();
        let m = round_trip(&items, moved.splices);
        assert_eq!(
            names(&m)[..5],
            [(0, "AXI"), (1, "Write"), (2, "clk"), (2, "irq"), (2, "aw")]
        );
        assert_eq!(moved.roots, set(&[2, 3]));
        let deep: Vec<Entry> = (0..MAX_DEPTH)
            .map(|d| group(d, "g", false))
            .chain([leaf(0, "x")])
            .collect();
        let to = Place {
            at: MAX_DEPTH as usize,
            depth: MAX_DEPTH,
        };
        assert!(plan_move(&deep, &set(&[0]), to).is_none());
    }

    fn moved_roots(items: &[Entry]) -> Vec<&str> {
        items
            .iter()
            .filter(|e| e.depth == 0)
            .map(|e| e.name())
            .collect()
    }

    #[test]
    fn group_ungroup_insert_and_fold() {
        let items = soc();
        let (splices, g) =
            super::group(&items, &set(&[4, 8]), WaveRow::Group(GroupRow::new("new"))).unwrap();
        assert_eq!(g, 4);
        let items = round_trip(&items, splices);
        assert_eq!(names(&items)[4..7], [(2, "new"), (3, "w"), (3, "irq")]);
        let (splices, children) = ungroup(&items, &set(&[4]));
        assert_eq!(children, set(&[4, 5]));
        let items = round_trip(&items, splices);
        assert_eq!(items[5].name(), "irq");
        assert_eq!(items[5].depth, 2);

        let items = soc();
        let (splices, children) = ungroup(&items, &set(&[1, 5]));
        let flat = round_trip(&items, splices);
        assert_eq!(
            names(&flat),
            [
                (0, "clk"),
                (0, "Write"),
                (1, "aw"),
                (1, "w"),
                (0, "ar"),
                (0, "r"),
                (0, "irq")
            ]
        );
        assert_eq!(children, set(&[1, 4, 5]));

        let items = soc();
        let copied = extract(&items, &set(&[5]));
        assert_eq!(
            copied.iter().map(|e| e.depth).collect::<Vec<_>>(),
            [0, 1, 1]
        );
        let (splice, roots) = insertion(Place { at: 9, depth: 0 }, copied).unwrap();
        assert_eq!(roots, set(&[9]));
        let mut items = round_trip(&items, vec![splice]);
        assert!(set_collapsed(&mut items, 1, true, true));
        assert!(
            items[1..8]
                .iter()
                .all(|e| e.group().is_none_or(|g| g.collapsed))
        );
        let items = round_trip(&items, removal(&items, &set(&[1, 3])));
        assert_eq!(
            items.iter().map(|e| e.name()).collect::<Vec<_>>(),
            ["clk", "irq", "Read", "ar", "r"]
        );
    }

    #[test]
    fn invalid_splices_change_nothing() {
        let mut items = soc();
        let before = names(&items).len();
        let outside = Splice {
            at: 8,
            remove: 2,
            insert: Vec::new(),
        };
        assert!(apply(&mut items, vec![outside]).is_err());
        let orphan = Splice {
            at: 1,
            remove: 1,
            insert: Vec::new(),
        };
        assert!(apply(&mut items, vec![orphan]).is_err());
        assert_eq!(names(&items).len(), before);
    }
}
