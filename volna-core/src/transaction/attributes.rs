//! Bounded, resumable attribute inspection with lossless source identities.

use super::view::{Budget, SectionKey};
use crate::data::text::{Radix, append_exact, format_attribute_radix, is_integer, truncate_ref};
use crate::trace::Traced;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fmt::{self, Write};
use std::sync::Arc;
use volna_trace::data::transactions::{AttributeValue, TrackRef, Transaction, TransactionRef};

pub const PAGE_ROWS: usize = 64;
pub const NAVIGATION_BYTES: usize = 32 * 1024;
pub(super) const SCAN_SCRATCH_BYTES: usize = 32 * 1024;
const NAVIGATION_ENTRY_BYTES: usize = 2 * 1024;
const SCAN_STEPS: usize = 4096;
// Interned references can nest beyond the format’s inline-value depth limit.
const SCAN_DEPTH: usize = 256;
const SHARED_SUMMARIES: usize = 256;
const SEARCH_COMPARISONS: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct AttributeSource {
    track: Traced<u32>,
    generation: u64,
    transaction: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttributeOwner {
    source: AttributeSource,
    record: u64,
}

impl AttributeOwner {
    pub(super) fn new(
        track: Traced<TrackRef>,
        generation: u64,
        transaction: TransactionRef,
    ) -> Self {
        Self {
            source: AttributeSource {
                track: track.map(|t| t.0),
                generation,
                transaction: transaction.0,
            },
            record: transaction.0,
        }
    }
    pub(super) fn with_record(self, record: TransactionRef) -> Self {
        Self {
            record: record.0,
            ..self
        }
    }
    fn key(self) -> String {
        format!(
            "{}:{}/{}/{}",
            self.source.track.trace, self.source.track.item, self.source.transaction, self.record
        )
    }
}

impl fmt::Display for AttributeOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.key(), self.source.generation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttributeId {
    owner: AttributeOwner,
    path: Arc<[usize]>,
}

impl AttributeId {
    pub(super) fn belongs_to(&self, source: AttributeOwner) -> bool {
        self.owner.source == source.source
    }
    fn format_key(&self) -> String {
        // Formatting preferences survive reloads; action identities include the source generation.
        let mut key = self.owner.key();
        for index in &*self.path {
            write!(key, "/{index}").expect("string write");
        }
        key
    }

    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        (self.path.len() > 1).then(|| Self {
            owner: self.owner,
            path: self.path[..self.path.len() - 1].into(),
        })
    }
}

impl fmt::Display for AttributeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.owner)?;
        for index in &*self.path {
            write!(f, "/{index}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttrRow {
    pub id: AttributeId,
    pub path: String,
    pub depth: usize,
    pub children: usize,
    pub expanded: bool,
    pub key: String,
    pub phase: Option<&'static str>,
    pub value: String,
    pub radix: Option<Radix>,
    pub kind: &'static str,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Position {
    next_root: usize,
    frames: Vec<Frame>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Frame {
    index: usize,
    next_child: usize,
    phase: SearchPhase,
    emitted: bool,
    matched: bool,
    include_children: bool,
    search_offset: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum SearchPhase {
    #[default]
    Name,
    Value,
    Children,
}

impl Position {
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.frames.capacity() * std::mem::size_of::<Frame>()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Pages {
    first: usize,
    current: usize,
    starts: VecDeque<Position>,
}

impl Pages {
    fn bytes(&self) -> usize {
        NAVIGATION_ENTRY_BYTES
            + self.starts.capacity() * std::mem::size_of::<Position>()
            + self
                .starts
                .iter()
                .map(|p| p.frames.capacity() * std::mem::size_of::<Frame>())
                .sum::<usize>()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttributePrefs {
    pub expanded: bool,
    pub toggled: BTreeSet<AttributeId>,
    pages: BTreeMap<AttributeOwner, Pages>,
}

impl Default for AttributePrefs {
    fn default() -> Self {
        Self {
            expanded: true,
            toggled: BTreeSet::new(),
            pages: BTreeMap::new(),
        }
    }
}

impl AttributePrefs {
    #[must_use]
    pub fn is_expanded(&self, id: &AttributeId) -> bool {
        self.expanded != self.toggled.contains(id)
    }
    pub(super) fn clear_pages(&mut self) {
        self.pages.clear();
    }
    pub(super) fn navigate(
        &mut self,
        owner: AttributeOwner,
        page: usize,
        tree: &AttributeTree,
    ) -> bool {
        if page == 0 {
            return self.pages.remove(&owner).is_some();
        }
        let pages = self.pages.entry(owner).or_insert_with(|| Pages {
            starts: VecDeque::from([Position::default()]),
            ..Pages::default()
        });
        if page >= pages.first && page < pages.first + pages.starts.len() {
            pages.current = page;
            return true;
        }
        if page != pages.current + 1 {
            return false;
        }
        let Some(next) = &tree.next else { return false };
        pages.starts.truncate(pages.current - pages.first + 1);
        pages.starts.push_back(next.clone());
        pages.current = page;
        while self.pages.values().map(Pages::bytes).sum::<usize>() > NAVIGATION_BYTES {
            if let Some(pages) = self
                .pages
                .values_mut()
                .find(|p| p.starts.len() > 1 && p.current > p.first)
            {
                pages.starts.pop_front();
                pages.first += 1;
            } else if let Some(other) = self.pages.keys().copied().find(|id| *id != owner) {
                self.pages.remove(&other);
            } else {
                break;
            }
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttributeTree {
    pub owner: AttributeOwner,
    pub key: SectionKey,
    pub total: usize,
    pub page: usize,
    pub collapsed: bool,
    pub filtering: bool,
    pub limited: bool,
    pub rows: Vec<AttrRow>,
    previous: Option<usize>,
    next: Option<Position>,
}

impl AttributeTree {
    #[must_use]
    pub fn has_branches(&self) -> bool {
        self.rows.iter().any(|row| row.children > 0)
    }

    #[must_use]
    pub fn has_previous(&self) -> bool {
        self.previous.is_some()
    }
    #[must_use]
    pub fn has_next(&self) -> bool {
        self.next.is_some()
    }
    #[must_use]
    pub fn cut(&self) -> bool {
        self.page > 0 || self.has_next() || self.limited
    }
}

#[derive(Clone, Copy)]
enum Name<'a> {
    Field(&'a str),
    Index(usize),
}
impl<'a> Name<'a> {
    fn text(self) -> std::borrow::Cow<'a, str> {
        match self {
            Self::Field(name) => name.into(),
            Self::Index(i) => format!("[{i}]").into(),
        }
    }
}

#[derive(Clone, Copy)]
struct Node<'a> {
    name: Name<'a>,
    value: &'a AttributeValue,
}

fn child(value: &AttributeValue, i: usize) -> Option<Node<'_>> {
    match value.resolved() {
        AttributeValue::Map(fields) => fields.get(i).map(|(name, value)| Node {
            name: Name::Field(name),
            value,
        }),
        AttributeValue::List(values) => values.get(i).map(|value| Node {
            name: Name::Index(i),
            value,
        }),
        _ => None,
    }
}

fn children(value: &AttributeValue) -> usize {
    match value.resolved() {
        AttributeValue::Map(fields) => fields.len(),
        AttributeValue::List(values) => values.len(),
        _ => 0,
    }
}

fn search(
    text: &str,
    needle: &[char],
    offset: &mut usize,
    comparisons: &mut usize,
) -> Option<bool> {
    if needle.is_empty() {
        return Some(true);
    }
    while *offset < text.len() {
        if *comparisons < needle.len() {
            return None;
        }
        let mut hay = text[*offset..].chars().flat_map(char::to_lowercase);
        let found = needle.iter().all(|expected| {
            *comparisons -= 1;
            hay.next() == Some(*expected)
        });
        if found {
            return Some(true);
        }
        *offset += text[*offset..]
            .chars()
            .next()
            .expect("remaining character")
            .len_utf8();
    }
    Some(false)
}

fn value_kind(value: &AttributeValue) -> &'static str {
    match value.resolved() {
        AttributeValue::Shared(_) => unreachable!("resolved value"),
        AttributeValue::Null => "null",
        AttributeValue::Bool(_) => "bool",
        AttributeValue::I64(_) | AttributeValue::U64(_) => "int",
        AttributeValue::F64(_) => "real",
        AttributeValue::Text(_) => "text",
        AttributeValue::Bytes(_) => "bytes",
        AttributeValue::Logic { .. } => "logic",
        AttributeValue::Time(_) => "time",
        AttributeValue::Enum { .. } => "enum",
        AttributeValue::Pointer(_) => "pointer",
        AttributeValue::Fixed { .. } | AttributeValue::UFixed { .. } => "fixed",
        AttributeValue::List(_) => "list",
        AttributeValue::Map(_) => "map",
    }
}

fn split_phase(key: &str) -> (&str, Option<&'static str>) {
    for phase in ["record", "end"] {
        if let Some(base) = key.strip_suffix(phase)
            && let Some(base) = base.strip_suffix('.')
            && !base.is_empty()
        {
            return (base, Some(phase));
        }
    }
    (key, None)
}

pub(super) fn tree<'a>(
    owner: AttributeOwner,
    key: SectionKey,
    roots: impl Iterator<Item = (usize, &'a str, &'a AttributeValue)> + Clone,
    prefs: &super::ViewPrefs,
    filter: &str,
    budget: &mut Budget,
) -> AttributeTree {
    let pages = prefs.attributes.pages.get(&owner);
    let page = pages.map_or(0, |p| p.current);
    let start = pages.and_then(|p| p.starts.get(page - p.first));
    let mut position = start.cloned().unwrap_or_default();
    let mut tree = AttributeTree {
        owner,
        key,
        total: roots.clone().count(),
        page,
        collapsed: prefs.collapsed.contains(&key),
        filtering: !filter.is_empty(),
        limited: false,
        rows: Vec::new(),
        previous: pages.filter(|p| p.current > p.first).map(|p| p.current - 1),
        next: None,
    };
    if !budget.admit(position.bytes()) {
        tree.limited = true;
        return tree;
    }
    let mut nodes = Vec::new();
    if let Some(first) = position.frames.first() {
        if let Some((_, name, value)) = roots.clone().find(|(i, _, _)| *i == first.index) {
            nodes.push(Node {
                name: Name::Field(name),
                value,
            });
            for frame in position.frames.iter().skip(1) {
                let Some(node) = child(nodes.last().expect("parent").value, frame.index) else {
                    return tree;
                };
                nodes.push(node);
            }
        } else {
            return tree;
        }
    }
    let next_root = position.next_root;
    let mut roots = roots.filter(|(i, _, _)| *i >= next_root);
    // Repeated references must not multiply a search through a nonmatching subtree.
    let mut absent = HashSet::<*const AttributeValue>::new();
    let mut end = filter.len().min(256);
    while !filter.is_char_boundary(end) {
        end -= 1;
    }
    let needle: Vec<_> = filter[..end].chars().flat_map(char::to_lowercase).collect();
    let mut comparisons = SEARCH_COMPARISONS;
    let mut finished = false;
    for _ in 0..SCAN_STEPS {
        if nodes.is_empty() {
            let Some((index, name, value)) = roots.next() else {
                finished = true;
                break;
            };
            position.next_root = index + 1;
            if !budget.slot(&mut position.frames) {
                tree.limited = true;
                break;
            }
            position.frames.push(Frame {
                index,
                ..Frame::default()
            });
            nodes.push(Node {
                name: Name::Field(name),
                value,
            });
        }
        let depth = nodes.len() - 1;
        let node = nodes[depth];
        let name = node.name.text();
        let state = &mut position.frames[depth];
        if state.phase != SearchPhase::Children {
            let mut direct = state.include_children || state.matched || needle.is_empty();
            if !direct && state.phase == SearchPhase::Name {
                let Some(found) =
                    search(&name, &needle, &mut state.search_offset, &mut comparisons)
                else {
                    break;
                };
                direct = found;
                state.phase = SearchPhase::Value;
                state.search_offset = 0;
            }
            if !direct && children(node.value) == 0 {
                let text = match node.value.resolved() {
                    AttributeValue::Text(text) | AttributeValue::Enum { name: text, .. } => {
                        std::borrow::Cow::Borrowed(text.as_str())
                    }
                    value => scalar(value, Radix::Dec, 256).0.into(),
                };
                let Some(found) =
                    search(&text, &needle, &mut state.search_offset, &mut comparisons)
                else {
                    break;
                };
                direct = found;
            }
            if !direct && absent.contains(&std::ptr::from_ref(node.value.resolved())) {
                nodes.pop();
                position.frames.pop();
                continue;
            }
            if direct {
                state.matched = true;
                if let Some(ancestor) = position.frames.iter().position(|frame| !frame.emitted) {
                    if tree.rows.len() == PAGE_ROWS {
                        break;
                    }
                    if !push_row(
                        owner,
                        &nodes[..=ancestor],
                        &position.frames[..=ancestor],
                        prefs,
                        filter,
                        &mut tree.rows,
                        budget,
                    ) {
                        tree.limited = true;
                        break;
                    }
                    position.frames[ancestor].emitted = true;
                    continue;
                }
            }
            let state = &mut position.frames[depth];
            state.phase = SearchPhase::Children;
            state.include_children = direct;
        }
        let id = AttributeId {
            owner,
            path: position
                .frames
                .iter()
                .map(|f| f.index)
                .collect::<Vec<_>>()
                .into(),
        };
        if filter.is_empty() && !prefs.attributes.is_expanded(&id) {
            position.frames[depth].next_child = children(node.value);
        }
        let state = &mut position.frames[depth];
        if let Some(next) = child(node.value, state.next_child) {
            if nodes.len() == SCAN_DEPTH {
                tree.limited = true;
                break;
            }
            let index = state.next_child;
            let include_children = state.include_children;
            if !budget.slot(&mut position.frames) {
                tree.limited = tree.rows.len() < PAGE_ROWS;
                break;
            }
            position.frames[depth].next_child += 1;
            position.frames.push(Frame {
                index,
                include_children,
                ..Frame::default()
            });
            nodes.push(next);
        } else {
            let state = position.frames.pop().expect("visited frame");
            nodes.pop();
            if let Some(parent) = position.frames.last_mut() {
                parent.matched |= state.matched;
            }
            if !state.matched && absent.len() < SHARED_SUMMARIES {
                absent.insert(std::ptr::from_ref(node.value.resolved()));
            }
        }
    }
    if !finished && !tree.limited {
        tree.next = Some(position);
    }
    tree
}

fn push_row(
    owner: AttributeOwner,
    nodes: &[Node<'_>],
    frames: &[Frame],
    prefs: &super::ViewPrefs,
    filter: &str,
    rows: &mut Vec<AttrRow>,
    budget: &mut Budget,
) -> bool {
    let id_bytes = frames.len() * std::mem::size_of::<usize>() + 2 * std::mem::size_of::<usize>();
    if !budget.admit(id_bytes) || !budget.slot(rows) {
        return false;
    }
    let id = AttributeId {
        owner,
        path: frames.iter().map(|f| f.index).collect::<Vec<_>>().into(),
    };
    let node = nodes.last().expect("row node");
    let name = node.name.text();
    let radix = is_integer(node.value).then(|| match nodes[0].name {
        Name::Field(root) if nodes.len() == 1 => prefs.radix_of(root),
        _ if prefs.radix.is_empty() => Radix::Dec,
        _ => prefs.radix_of(&id.format_key()),
    });
    let allowance = budget.remaining / (PAGE_ROWS - rows.len());
    let mut row_budget = Budget::new(allowance);
    let mut path = String::new();
    for (i, node) in nodes.iter().enumerate() {
        match node.name {
            Name::Field(name) => {
                if i > 0 {
                    crate::data::text::push_limited(&mut path, ".", 256);
                }
                crate::data::text::push_limited(&mut path, name, 256);
            }
            Name::Index(index) => {
                crate::data::text::push_limited(&mut path, &format!("[{index}]"), 256);
            }
        }
    }
    let (name, phase) = if nodes.len() == 1 {
        split_phase(&name)
    } else {
        (name.as_ref(), None)
    };
    let key = row_budget.text(name);
    let path = row_budget.text(&path);
    let (value, truncated) = scalar(
        node.value,
        radix.unwrap_or_default(),
        row_budget.remaining.min(4096),
    );
    row_budget.remaining = row_budget.remaining.saturating_sub(value.len());
    budget.remaining -= allowance - row_budget.remaining;
    budget.truncated |= row_budget.truncated || truncated;
    rows.push(AttrRow {
        expanded: !filter.is_empty() || prefs.attributes.is_expanded(&id),
        id,
        path,
        depth: nodes.len() - 1,
        children: children(node.value),
        key,
        phase,
        value,
        radix,
        kind: value_kind(node.value),
        truncated,
    });
    true
}

pub(super) struct Target<'a> {
    pub root: &'a str,
    pub value: &'a AttributeValue,
}

pub(super) fn resolve<'a>(tx: &'a Transaction, id: &AttributeId) -> Option<Target<'a>> {
    let index = *id.path.first()?;
    let (root, mut value) = if id.owner.record == tx.id.0 {
        let attr = tx.attributes.get(index)?;
        (attr.key.as_str(), &attr.value)
    } else if let Some(stage) = tx.stages.iter().find(|s| s.id.0 == id.owner.record) {
        let (key, value) = stage.attributes.get(index)?;
        (key.as_str(), value)
    } else {
        let (key, value) = tx
            .events
            .iter()
            .find(|s| s.id.0 == id.owner.record)?
            .attributes
            .get(index)?;
        (key.as_str(), value)
    };
    for &index in id.path.iter().skip(1) {
        value = child(value, index)?.value;
    }
    Some(Target { root, value })
}

pub(super) fn radix_key<'a>(target: &Target<'a>, id: &AttributeId) -> std::borrow::Cow<'a, str> {
    if id.path.len() == 1 {
        target.root.into()
    } else {
        id.format_key().into()
    }
}

pub(super) fn source_path(
    tx: &Transaction,
    id: &AttributeId,
    limit: usize,
) -> anyhow::Result<String> {
    let root = AttributeId {
        owner: id.owner,
        path: vec![id.path[0]].into(),
    };
    let target =
        resolve(tx, &root).ok_or_else(|| anyhow::anyhow!("Attribute is no longer available."))?;
    let mut path = String::new();
    append_exact(&mut path, target.root, limit)?;
    let mut value = target.value;
    for &index in id.path.iter().skip(1) {
        let node = child(value, index)
            .ok_or_else(|| anyhow::anyhow!("Attribute is no longer available."))?;
        match node.name {
            Name::Field(name) => {
                append_exact(&mut path, ".", limit)?;
                append_exact(&mut path, name, limit)?;
            }
            Name::Index(index) => append_exact(&mut path, &format!("[{index}]"), limit)?,
        }
        value = node.value;
    }
    Ok(path)
}

pub(super) fn scalar(value: &AttributeValue, radix: Radix, limit: usize) -> (String, bool) {
    let (text, required) = match value.resolved() {
        AttributeValue::Map(fields) => (format!("{} fields", fields.len()), 0),
        AttributeValue::List(values) => (format!("{} items", values.len()), 0),
        AttributeValue::Text(text) => {
            let text = if text.is_empty() { "\"\"" } else { text };
            return (truncate_ref(text, limit), text.len() > limit);
        }
        AttributeValue::Bytes(bytes) => {
            let mut text = String::new();
            for (i, byte) in bytes.iter().take(limit.saturating_add(1) / 3).enumerate() {
                if i > 0 {
                    text.push(' ');
                }
                write!(text, "{byte:02x}").expect("string write");
            }
            let required = bytes.len().saturating_mul(3).saturating_sub(1);
            if bytes.is_empty() {
                text.push_str("[]");
            }
            (text, required)
        }
        AttributeValue::Logic {
            width,
            states,
            data,
        } => {
            let logic = volna_trace::data::value_view::LogicView::packed_lsb(*width, *states, data);
            let text = (0..logic.width.min(limit))
                .map(|i| char::from(logic.bit(i)))
                .collect();
            (text, logic.width)
        }
        value => {
            let text = format_attribute_radix(
                value,
                radix,
                if matches!(value, AttributeValue::Enum { .. }) {
                    limit
                } else {
                    usize::MAX
                },
            );
            let required = crate::data::text::attribute_value_min_bytes(value).max(text.len());
            (text, required)
        }
    };
    let cut = text.len() > limit || required > limit;
    (truncate_ref(&text, limit), cut)
}

pub(super) fn copy_value(value: &AttributeValue, limit: usize) -> anyhow::Result<String> {
    if let AttributeValue::Text(text) = value.resolved() {
        let mut out = String::new();
        append_exact(&mut out, text, limit)?;
        return Ok(out);
    }

    enum Work<'a> {
        Value(&'a AttributeValue),
        List(&'a [AttributeValue]),
        Map(&'a [(String, AttributeValue)]),
        Token(&'static str),
    }
    let mut text = String::new();
    let mut work = vec![Work::Value(value)];
    while let Some(item) = work.pop() {
        match item {
            Work::Token(token) => append_exact(&mut text, token, limit)?,
            Work::Value(value) => match value.resolved() {
                AttributeValue::Map(fields) => {
                    append_exact(&mut text, "{", limit)?;
                    work.push(Work::Token("}"));
                    work.push(Work::Map(fields));
                }
                AttributeValue::List(values) => {
                    append_exact(&mut text, "[", limit)?;
                    work.push(Work::Token("]"));
                    work.push(Work::List(values));
                }
                AttributeValue::Text(value) => {
                    anyhow::ensure!(
                        value.len() <= limit,
                        "Attribute exceeds the clipboard limit."
                    );
                    append_exact(&mut text, &serde_json::to_string(value)?, limit)?;
                }
                value => {
                    let (value, cut) = scalar(value, Radix::Dec, limit);
                    anyhow::ensure!(!cut, "Attribute exceeds the clipboard limit.");
                    append_exact(&mut text, &value, limit)?;
                }
            },
            Work::List(values) => {
                if let Some((first, rest)) = values.split_first() {
                    if !rest.is_empty() {
                        work.push(Work::List(rest));
                        work.push(Work::Token(", "));
                    }
                    work.push(Work::Value(first));
                }
            }
            Work::Map(fields) => {
                if let Some(((key, value), rest)) = fields.split_first() {
                    anyhow::ensure!(key.len() <= limit, "Attribute exceeds the clipboard limit.");
                    append_exact(&mut text, &serde_json::to_string(key)?, limit)?;
                    append_exact(&mut text, ": ", limit)?;
                    if !rest.is_empty() {
                        work.push(Work::Map(rest));
                        work.push(Work::Token(", "));
                    }
                    work.push(Work::Value(value));
                }
            }
        }
    }
    Ok(text)
}

pub(super) fn copy_attributes(
    attributes: &[(String, AttributeValue)],
    limit: usize,
) -> anyhow::Result<String> {
    let mut text = String::new();
    for (i, (key, value)) in attributes.iter().enumerate() {
        if i > 0 {
            append_exact(&mut text, " · ", limit)?;
        }
        append_exact(&mut text, key, limit)?;
        append_exact(&mut text, "=", limit)?;
        let value = copy_value(value, limit.saturating_sub(text.len()))?;
        append_exact(&mut text, &value, limit)?;
    }
    Ok(text)
}
