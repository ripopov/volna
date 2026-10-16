//! Trace metadata and hierarchy model shared by every session.

use super::transactions::{Attributes, TrackRef};
use super::value::SignalShape;

pub type ScopeId = usize;
pub type VarId = usize;
pub type GeneratorId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Member {
    Var(VarId),
    Generator(GeneratorId),
}

impl Member {
    #[must_use]
    pub fn var(self) -> Option<VarId> {
        if let Self::Var(id) = self {
            Some(id)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Generator {
    pub name: String,
    pub scope: ScopeId,
    pub root: bool,
    pub kind: String,
    pub track: TrackRef,
    pub attributes: Attributes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup<T> {
    Found(T),
    Missing,
    Ambiguous,
}

fn unique<T>(mut matches: impl Iterator<Item = T>) -> Lookup<T> {
    match (matches.next(), matches.next()) {
        (None, _) => Lookup::Missing,
        (Some(id), None) => Lookup::Found(id),
        _ => Lookup::Ambiguous,
    }
}

#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct SignalRef(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Direction {
    None,
    Input,
    Output,
    InOut,
}

impl From<vtr::Direction> for Direction {
    fn from(direction: vtr::Direction) -> Self {
        match direction {
            vtr::Direction::Input => Self::Input,
            vtr::Direction::Output => Self::Output,
            vtr::Direction::InOut => Self::InOut,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Scope {
    pub name: String,
    pub kind: String,
    pub parent: Option<ScopeId>,
    pub children: Vec<ScopeId>,
    pub vars: Vec<VarId>,
    pub component: String,
    pub generators: Vec<GeneratorId>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Variable {
    pub name: String,
    pub scope: ScopeId,
    pub shape: SignalShape,
    pub var_type: String,
    pub direction: Direction,
    pub signal: SignalRef,
    pub enum_table: Option<u32>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct HierarchyBuilder {
    pub scopes: Vec<Scope>,
    pub roots: Vec<ScopeId>,
    pub vars: Vec<Variable>,
    pub generators: Vec<Generator>,
}

impl HierarchyBuilder {
    pub fn push_scope(&mut self, name: String, kind: String, parent: Option<ScopeId>) -> ScopeId {
        let id = self.scopes.len();
        self.scopes.push(Scope {
            name,
            kind,
            parent,
            children: Vec::new(),
            vars: Vec::new(),
            component: String::new(),
            generators: Vec::new(),
        });
        match parent {
            Some(p) => self.scopes[p].children.push(id),
            None => self.roots.push(id),
        }
        id
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TraceInfo {
    pub name: String,
    pub design_id: Option<String>,
    pub timescale: i8,
    pub time_range: (u64, u64),
    pub signal_count: usize,
    pub change_count: Option<u64>,
    pub time_unit: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct ScopeView<'a> {
    pub name: &'a str,
    pub kind: &'a str,
    pub component: &'a str,
    pub parent: Option<ScopeId>,
    pub children: Ids<'a>,
    pub vars: Ids<'a>,
    pub generators: Ids<'a>,
}

#[derive(Clone, Copy, Debug)]
pub struct VariableView<'a> {
    pub name: &'a str,
    pub scope: ScopeId,
    pub shape: SignalShape,
    pub var_type: &'a str,
    pub direction: Direction,
    pub signal: SignalRef,
    pub enum_table: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct Ids<'a> {
    storage: IdStorage<'a>,
    gap: Option<usize>,
}
#[derive(Clone, Copy, Debug)]
enum IdStorage<'a> {
    Slice(&'a [u32]),
    Reader(vtr::hierarchy_index::IndexedNodes<'a>),
}
impl Default for Ids<'_> {
    fn default() -> Self {
        Self::from_slice(&[])
    }
}
impl<'a> Ids<'a> {
    #[must_use]
    pub fn from_slice(ids: &'a [u32]) -> Self {
        Self {
            storage: IdStorage::Slice(ids),
            gap: None,
        }
    }
    pub(crate) fn reader(ids: vtr::hierarchy_index::IndexedNodes<'a>) -> Self {
        Self {
            storage: IdStorage::Reader(ids),
            gap: None,
        }
    }
    pub(crate) fn shifted(mut self, gap: Option<usize>) -> Self {
        self.gap = gap;
        self
    }
    #[must_use]
    pub fn len(self) -> usize {
        match self.storage {
            IdStorage::Slice(ids) => ids.len(),
            IdStorage::Reader(ids) => ids.len(),
        }
    }
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.iter().next().is_none()
    }
    #[must_use]
    pub fn iter(self) -> IdsIter<'a> {
        IdsIter {
            storage: match self.storage {
                IdStorage::Slice(ids) => IterStorage::Slice(ids.iter()),
                IdStorage::Reader(ids) => IterStorage::Reader(ids.iter()),
            },
            gap: self.gap,
        }
    }
    #[must_use]
    pub fn first(self) -> Option<usize> {
        self.iter().next()
    }
}
impl<'a> IntoIterator for Ids<'a> {
    type Item = usize;
    type IntoIter = IdsIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
pub struct IdsIter<'a> {
    storage: IterStorage<'a>,
    gap: Option<usize>,
}
enum IterStorage<'a> {
    Slice(std::slice::Iter<'a, u32>),
    Reader(vtr::hierarchy_index::IndexedIter<'a>),
}
impl Iterator for IdsIter<'_> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        let id = match &mut self.storage {
            IterStorage::Slice(ids) => ids.next().map(|&id| id as usize),
            IterStorage::Reader(ids) => ids.next(),
        }?;
        Some(id + usize::from(self.gap.is_some_and(|gap| id >= gap)))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.storage {
            IterStorage::Slice(ids) => ids.size_hint(),
            IterStorage::Reader(ids) => ids.size_hint(),
        }
    }
}
impl DoubleEndedIterator for IdsIter<'_> {
    fn next_back(&mut self) -> Option<usize> {
        let id = match &mut self.storage {
            IterStorage::Slice(ids) => ids.next_back().map(|&id| id as usize),
            IterStorage::Reader(ids) => ids.next_back(),
        }?;
        Some(id + usize::from(self.gap.is_some_and(|gap| id >= gap)))
    }
}

pub(crate) trait HierarchySource: Send + Sync + std::fmt::Debug {
    fn scope_count(&self) -> usize;
    fn var_count(&self) -> usize;
    fn roots(&self) -> Ids<'_>;
    fn scope(&self, id: ScopeId) -> ScopeView<'_>;
    fn var(&self, id: VarId) -> VariableView<'_>;
    fn resident_bytes(&self) -> u64;
    fn signal(&self, id: VarId) -> SignalRef {
        self.var(id).signal
    }
    fn var_name(&self, id: VarId) -> &str {
        self.var(id).name
    }
}

#[derive(Clone, Debug)]
pub struct Hierarchy {
    pub(crate) source: std::sync::Arc<dyn HierarchySource>,
    pub(crate) generators: std::sync::Arc<Vec<Generator>>,
}
impl Default for Hierarchy {
    fn default() -> Self {
        HierarchyBuilder::default().finish()
    }
}
impl Hierarchy {
    #[must_use]
    pub fn generators(&self) -> &[Generator] {
        &self.generators
    }
    #[must_use]
    pub fn scope_count(&self) -> usize {
        self.source.scope_count()
    }
    #[must_use]
    pub fn var_count(&self) -> usize {
        self.source.var_count()
    }
    #[must_use]
    pub fn signal(&self, id: VarId) -> SignalRef {
        self.source.signal(id)
    }
    #[must_use]
    pub fn roots(&self) -> Ids<'_> {
        self.source.roots()
    }
    #[must_use]
    pub fn scope(&self, id: ScopeId) -> ScopeView<'_> {
        self.source.scope(id)
    }
    #[must_use]
    pub fn var(&self, id: VarId) -> VariableView<'_> {
        self.source.var(id)
    }
    #[must_use]
    pub fn var_name(&self, id: VarId) -> &str {
        self.source.var_name(id)
    }
    #[must_use]
    pub fn get_scope(&self, id: ScopeId) -> Option<ScopeView<'_>> {
        (id < self.scope_count()).then(|| self.scope(id))
    }
    #[must_use]
    pub fn get_var(&self, id: VarId) -> Option<VariableView<'_>> {
        (id < self.var_count()).then(|| self.var(id))
    }
    pub fn scopes(&self) -> impl ExactSizeIterator<Item = ScopeView<'_>> {
        (0..self.scope_count()).map(|id| self.scope(id))
    }
    pub fn vars(&self) -> impl ExactSizeIterator<Item = VariableView<'_>> {
        (0..self.var_count()).map(|id| self.var(id))
    }
    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        self.source.resident_bytes()
    }
    #[must_use]
    pub fn to_builder(&self) -> HierarchyBuilder {
        HierarchyBuilder {
            scopes: self
                .scopes()
                .map(|s| Scope {
                    name: s.name.into(),
                    kind: s.kind.into(),
                    component: s.component.into(),
                    parent: s.parent,
                    children: s.children.iter().collect(),
                    vars: s.vars.iter().collect(),
                    generators: s.generators.iter().collect(),
                })
                .collect(),
            vars: self
                .vars()
                .map(|v| Variable {
                    name: v.name.into(),
                    scope: v.scope,
                    shape: v.shape,
                    var_type: v.var_type.into(),
                    direction: v.direction,
                    signal: v.signal,
                    enum_table: v.enum_table,
                })
                .collect(),
            roots: self.roots().iter().collect(),
            generators: self.generators.as_ref().clone(),
        }
    }
    pub fn find_generator(&self, path: &[impl AsRef<str>]) -> Lookup<GeneratorId> {
        let Some((name, scope)) = path.split_last() else {
            return Lookup::Missing;
        };
        if scope.is_empty() {
            return unique(
                self.generators
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| g.root && g.name == name.as_ref())
                    .map(|(id, _)| id),
            );
        }
        match self.find_scope(scope) {
            Lookup::Found(id) => {
                unique(self.scope(id).generators.iter().filter(|&g| {
                    !self.generators[g].root && self.generators[g].name == name.as_ref()
                }))
            }
            Lookup::Missing => Lookup::Missing,
            Lookup::Ambiguous => Lookup::Ambiguous,
        }
    }

    #[must_use]
    pub fn member_name(&self, member: Member) -> &str {
        match member {
            Member::Var(id) => self.var_name(id),
            Member::Generator(id) => &self.generators[id].name,
        }
    }

    #[must_use]
    pub fn member_path(&self, member: Member) -> String {
        match member {
            Member::Var(id) => self.full_name(id),
            Member::Generator(id) => {
                let g = &self.generators[id];
                if g.root {
                    g.name.clone()
                } else {
                    format!("{}.{}", self.scope_path(g.scope).join("."), g.name)
                }
            }
        }
    }

    #[must_use]
    pub fn member_track(&self, member: Member) -> Option<TrackRef> {
        match member {
            Member::Generator(id) => Some(self.generators.get(id)?.track),
            Member::Var(_) => None,
        }
    }

    #[must_use]
    pub fn is_log(&self, member: Member) -> bool {
        matches!(member, Member::Generator(id) if self.generators[id].kind == "LOG")
    }

    #[must_use]
    pub fn has_vars(&self, scope: ScopeId) -> bool {
        if self.get_scope(scope).is_none() {
            return false;
        }
        let mut pending = vec![scope];
        while let Some(id) = pending.pop() {
            let scope = self.scope(id);
            if !scope.vars.is_empty() {
                return true;
            }
            pending.extend(scope.children.iter());
        }
        false
    }

    pub fn find_scope(&self, path: &[impl AsRef<str>]) -> Lookup<ScopeId> {
        let mut children = self.roots();
        let mut result = Lookup::Missing;
        for name in path {
            result = unique(
                children
                    .iter()
                    .filter(|&id| self.scope(id).name == name.as_ref()),
            );
            match result {
                Lookup::Found(id) => children = self.scope(id).children,
                _ => return result,
            }
        }
        result
    }

    pub fn find_var(&self, path: &[impl AsRef<str>], nth: Option<usize>) -> Lookup<VarId> {
        let Some((name, scope)) = path.split_last() else {
            return Lookup::Missing;
        };
        let scope = match self.find_scope(scope) {
            Lookup::Found(id) => id,
            Lookup::Missing => return Lookup::Missing,
            Lookup::Ambiguous => return Lookup::Ambiguous,
        };
        let mut matches = self
            .scope(scope)
            .vars
            .iter()
            .filter(|&id| self.var_name(id) == name.as_ref());
        match nth {
            Some(n) => matches.nth(n).map_or(Lookup::Missing, Lookup::Found),
            None => unique(matches),
        }
    }

    pub fn var_path(&self, var: VarId) -> (Vec<String>, Option<usize>) {
        let v = self.var(var);
        let mut path: Vec<_> = self
            .scope_path(v.scope)
            .into_iter()
            .map(str::to_owned)
            .collect();
        path.push(v.name.to_owned());
        let (mut before, mut total, mut seen) = (0, 0, false);
        for id in self.scope(v.scope).vars {
            seen |= id == var;
            if self.var_name(id) == v.name {
                total += 1;
                before += usize::from(!seen);
            }
        }
        let nth = (total > 1).then_some(before);
        (path, nth)
    }

    #[must_use]
    pub fn scope_path(&self, mut id: ScopeId) -> Vec<&str> {
        let mut parts = vec![self.scope(id).name];
        while let Some(p) = self.scope(id).parent {
            parts.push(self.scope(p).name);
            id = p;
        }
        parts.reverse();
        parts
    }

    #[must_use]
    pub fn full_name(&self, var: VarId) -> String {
        let v = self.var(var);
        let mut s = self.scope_path(v.scope).join(".");
        if !s.is_empty() {
            s.push('.');
        }
        s.push_str(v.name);
        s
    }
}
