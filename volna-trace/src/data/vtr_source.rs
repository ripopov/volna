//! [`Session`] over a VTR file in this process: memory-mapped from a path natively, parsed
//! from an in-memory image on wasm.

use std::sync::Arc;

use anyhow::Context as _;
use vtr::{NodeData, Reader, SignalKind, SignalValue, trace::SignalData};

use super::history::SignalHistory;
use super::source::{Generator, Hierarchy, SignalRef, TraceInfo};
use super::transactions::TrackRef;
use super::value::{Bit, SignalShape, WaveValue};
use crate::session::Session;

pub(super) struct LocalReader {
    reader: Reader,
    // Shared hierarchy handles retain this owner; close the mapping before removing its files.
    _owner: Option<Box<dyn Send + Sync>>,
}

impl std::ops::Deref for LocalReader {
    type Target = Reader;
    fn deref(&self) -> &Reader {
        &self.reader
    }
}

pub struct LocalSession {
    pub(super) reader: Arc<LocalReader>,
    pub(super) tracks: Vec<super::transactions::Track>,
    info: TraceInfo,
    hierarchy: Hierarchy,
    source_bytes: u64,
    activity: Option<super::activity::ActivitySource>,
    temporary: bool,
}

impl LocalSession {
    #[cfg(not(target_family = "wasm"))]
    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        let source_bytes = std::fs::metadata(path)?.len();
        let reader = Reader::open(path).with_context(|| format!("open {}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let activity = vtr::activity::Identity::of(&reader)
            .ok()
            .map(|id| super::activity::ActivitySource::new(path, id));
        let mut session = Self::from_reader(name, reader, source_bytes)?;
        session.activity = activity;
        Ok(session)
    }

    /// Retains owned temporary files until the last mapped reader closes. Activity
    /// sidecars stay beside this VTR rather than entering the persistent cache.
    #[cfg(not(target_family = "wasm"))]
    pub fn open_temporary(
        path: &std::path::Path,
        name: String,
        owner: impl Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        let source_bytes = std::fs::metadata(path)?.len();
        let reader = Reader::open(path)?;
        let activity = vtr::activity::Identity::of(&reader)
            .ok()
            .map(|id| super::activity::ActivitySource::new_temporary(path, id));
        let mut session =
            Self::from_owned_reader(name, reader, source_bytes, Some(Box::new(owner)))?;
        session.activity = activity;
        session.temporary = true;
        Ok(session)
    }

    pub fn from_bytes(name: impl Into<String>, bytes: Vec<u8>) -> anyhow::Result<Self> {
        let source_bytes = bytes.capacity() as u64;
        let reader = Reader::from_bytes(bytes).context("parse VTR image")?;
        Self::from_reader(name.into(), reader, source_bytes)
    }

    fn from_reader(name: String, reader: Reader, source_bytes: u64) -> anyhow::Result<Self> {
        Self::from_owned_reader(name, reader, source_bytes, None)
    }

    fn from_owned_reader(
        name: String,
        reader: Reader,
        source_bytes: u64,
        owner: Option<Box<dyn Send + Sync>>,
    ) -> anyhow::Result<Self> {
        let reader = Arc::new(LocalReader {
            reader,
            _owner: owner,
        });
        let hierarchy = build_hierarchy(Arc::clone(&reader));
        let file_text = |name: &str| {
            reader.metadata().attributes.iter().find_map(|attr| {
                let (key, value) = (attr.key, &attr.value);
                if reader.string(key).expect("metadata key") != name {
                    return None;
                }
                match value {
                    vtr::Value::Str(value) => {
                        Some(reader.string(*value).expect("metadata text").to_owned())
                    }
                    vtr::Value::Text(value) => Some(value.clone()),
                    _ => None,
                }
            })
        };
        let info = TraceInfo {
            name,
            design_id: file_text("design.vdb_id"),
            timescale: reader.metadata().timescale,
            time_range: reader.time_range()?.map_or((0, 0), |w| (w.first, w.last)),
            signal_count: reader.signal_count() as usize,
            change_count: None,
            time_unit: file_text("time.unit").filter(|unit| !unit.is_empty()),
        };
        Ok(LocalSession {
            tracks: super::vtr_transactions::tracks(&reader),
            reader,
            info,
            hierarchy,
            source_bytes,
            activity: None,
            temporary: false,
        })
    }
}

fn declaration_bytes(reader: &Reader) -> u64 {
    fn attributes(values: &vtr::Attributes) -> usize {
        values.capacity() * std::mem::size_of::<vtr::Attribute>()
            + values
                .iter()
                .map(|attr| owned_value(&attr.value))
                .sum::<usize>()
    }
    fn owned_value(value: &vtr::Value) -> usize {
        match value {
            vtr::Value::Text(text) => text.capacity(),
            vtr::Value::Bytes(bytes)
            | vtr::Value::Bits { data: bytes, .. }
            | vtr::Value::Logic { data: bytes, .. }
            | vtr::Value::Logic9 { data: bytes, .. } => bytes.capacity(),
            vtr::Value::List(values) => {
                values.capacity() * std::mem::size_of::<vtr::Value>()
                    + values.iter().map(owned_value).sum::<usize>()
            }
            vtr::Value::Map(values) => attributes(values),
            _ => 0,
        }
    }
    let declarations = reader.declarations();
    let nodes = declarations.nodes.capacity() * std::mem::size_of::<vtr::Node>()
        + declarations
            .nodes
            .iter()
            .map(|node| {
                attributes(&node.attributes)
                    + match &node.data {
                        NodeData::EnumTable { entries } => {
                            entries.capacity() * std::mem::size_of::<(u32, u32)>()
                        }
                        _ => 0,
                    }
            })
            .sum::<usize>();
    let strings = declarations.strings.capacity() * std::mem::size_of::<String>()
        + declarations
            .strings
            .iter()
            .map(String::capacity)
            .sum::<usize>();
    let types = (0..reader.types().len())
        .map(|id| {
            if id == vtr::schema::Primitive::Enum.id() as usize {
                // The enum tag reserves a table slot but is not a usable scalar datatype.
                return std::mem::size_of::<vtr::schema::Datatype>();
            }
            let datatype = reader.types().get(id as u32).expect("declared datatype");
            std::mem::size_of::<vtr::schema::Datatype>()
                + datatype.name.capacity()
                + match &datatype.kind {
                    vtr::schema::TypeKind::Struct(fields) => {
                        fields.capacity() * std::mem::size_of::<vtr::schema::Field>()
                            + fields
                                .iter()
                                .map(|field| field.name.capacity())
                                .sum::<usize>()
                    }
                    vtr::schema::TypeKind::Enum { values, .. } => {
                        values.capacity() * std::mem::size_of::<vtr::schema::Enumerator>()
                            + values
                                .iter()
                                .map(|value| value.name.capacity())
                                .sum::<usize>()
                    }
                    _ => 0,
                }
        })
        .sum::<usize>();
    (nodes + strings + types + declarations.signals.capacity() * std::mem::size_of::<SignalKind>())
        as u64
}

fn shape_of(kind: SignalKind, var_type: vtr::VarType) -> SignalShape {
    if var_type == vtr::VarType::Event {
        return SignalShape::Event;
    }
    match kind {
        SignalKind::Bits { width: 1, .. } => SignalShape::Bit,
        SignalKind::Bits { width, .. } => SignalShape::Vector {
            width: width.max(2),
        },
        SignalKind::Real => SignalShape::Real,
        SignalKind::VarLen => SignalShape::Text,
    }
}

struct ReaderHierarchy {
    reader: Arc<LocalReader>,
    scopes: Vec<vtr::NodeId>,
    vars: Vec<vtr::NodeId>,
    links: super::hierarchy_columns::Links,
    root_owner: Option<usize>,
    top: Option<usize>,
}
impl std::fmt::Debug for ReaderHierarchy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderHierarchy")
            .field("scopes", &self.scopes.len())
            .field("vars", &self.vars.len())
            .finish_non_exhaustive()
    }
}
impl ReaderHierarchy {
    fn node(&self, id: usize) -> vtr::NodeId {
        self.scopes[id - usize::from(self.top.is_some_and(|top| id > top))]
    }
    fn scope_id(&self, node: vtr::NodeId) -> Option<usize> {
        self.scopes
            .binary_search(&node)
            .ok()
            .map(|id| id + usize::from(self.top.is_some_and(|top| id >= top)))
    }
    fn owner(&self, node: &vtr::Node) -> usize {
        node.parent
            .and_then(|parent| self.scope_id(parent))
            .or(self.root_owner)
            .expect("declared member scope")
    }
}
impl super::source::HierarchySource for ReaderHierarchy {
    fn scope_count(&self) -> usize {
        self.scopes.len() + usize::from(self.top.is_some())
    }
    fn var_count(&self) -> usize {
        self.vars.len()
    }
    fn roots(&self) -> super::source::Ids<'_> {
        super::source::Ids::from_slice(&self.links.roots)
    }
    fn scope(&self, id: usize) -> super::source::ScopeView<'_> {
        let (name, kind, component, parent) = if Some(id) == self.top {
            ("(top)", "module", "", None)
        } else {
            let node = self.reader.node(self.node(id)).expect("declared scope");
            let NodeData::Scope {
                scope_type,
                component,
            } = node.data
            else {
                unreachable!()
            };
            (
                self.reader.string(node.name).expect("scope name"),
                vtr::ScopeType::from_code(scope_type).name(),
                self.reader.string(component).expect("scope component"),
                node.parent.and_then(|parent| self.scope_id(parent)),
            )
        };
        super::source::ScopeView {
            name,
            kind,
            component,
            parent,
            children: self.links.children(id),
            vars: self.links.vars(id),
            generators: self.links.generators(id),
        }
    }
    fn var_name(&self, id: usize) -> &str {
        self.reader.name(self.vars[id]).expect("variable name")
    }
    fn var(&self, id: usize) -> super::source::VariableView<'_> {
        let node = self.reader.node(self.vars[id]).expect("declared variable");
        let NodeData::Signal {
            var_type,
            direction,
            signal,
            ..
        } = node.data
        else {
            unreachable!()
        };
        let var_type = vtr::VarType::from_code(var_type);
        super::source::VariableView {
            name: self.reader.string(node.name).expect("variable name"),
            scope: self.owner(node),
            shape: shape_of(
                self.reader.signal_kind(signal).expect("signal kind"),
                var_type,
            ),
            var_type: var_type.name(),
            direction: vtr::Direction::from_u8(direction).into(),
            signal: SignalRef(signal.0),
            enum_table: node.attributes.iter().find_map(|attr| {
                if self.reader.string(attr.key).expect("attribute key") == "enum_table" {
                    match attr.value {
                        vtr::Value::U32(id) => Some(id),
                        vtr::Value::U64(id) => u32::try_from(id).ok(),
                        _ => None,
                    }
                } else {
                    None
                }
            }),
        }
    }
    fn resident_bytes(&self) -> u64 {
        ((self.scopes.capacity() + self.vars.capacity()) * std::mem::size_of::<vtr::NodeId>())
            as u64
            + self.links.resident_bytes()
    }
}

fn build_hierarchy(reader: Arc<LocalReader>) -> Hierarchy {
    use super::hierarchy_columns::{Links, NONE};
    use super::source::HierarchySource as _;
    let mut scopes = Vec::new();
    let mut vars = Vec::new();
    let mut generator_nodes = Vec::new();
    for (id, node) in reader.declarations().nodes.iter().enumerate() {
        let target = match node.data {
            NodeData::Scope { .. } => &mut scopes,
            NodeData::Signal { .. } => &mut vars,
            NodeData::Generator { .. } => &mut generator_nodes,
            NodeData::EnumTable { .. } | NodeData::Callsite { .. } => continue,
        };
        target.push(vtr::NodeId(id as u32));
    }
    let first_root_member = vars
        .iter()
        .chain(&generator_nodes)
        .copied()
        .filter(|id| reader.node(*id).expect("declared node").parent.is_none())
        .min();
    let existing_top = scopes.iter().position(|&id| {
        first_root_member.is_some_and(|first| id < first)
            && reader.node(id).expect("declared scope").parent.is_none()
            && reader.name(id).expect("scope name") == "(top)"
    });
    let top = first_root_member
        .filter(|_| existing_top.is_none())
        .map(|first| scopes.partition_point(|id| *id < first));
    scopes.shrink_to_fit();
    vars.shrink_to_fit();
    let mut source = ReaderHierarchy {
        reader,
        scopes,
        vars,
        links: Links::default(),
        root_owner: existing_top.or(top),
        top,
    };
    let generators: Vec<_> = generator_nodes
        .into_iter()
        .map(|id| {
            let node = source.reader.node(id).expect("declared generator");
            let NodeData::Generator { kind, .. } = node.data else {
                unreachable!()
            };
            Generator {
                name: source
                    .reader
                    .string(node.name)
                    .expect("generator name")
                    .into(),
                scope: source.owner(node),
                root: node.parent.is_none(),
                kind: source.reader.string(kind).expect("generator kind").into(),
                track: TrackRef(id.0),
                attributes: super::vtr_transactions::attributes(&source.reader, &node.attributes),
            }
        })
        .collect();
    let parents: Vec<_> = (0..source.scope_count())
        .map(|id| {
            if Some(id) == source.top {
                NONE
            } else {
                source
                    .reader
                    .node(source.node(id))
                    .expect("declared scope")
                    .parent
                    .and_then(|parent| source.scope_id(parent))
                    .map_or(NONE, |id| id as u32)
            }
        })
        .collect();
    source.links = Links::build(
        &parents,
        source
            .vars
            .iter()
            .map(|&id| source.owner(source.reader.node(id).expect("declared variable"))),
        &generators,
    );
    Hierarchy {
        source: Arc::new(source),
        generators: Arc::new(generators),
    }
}

impl Session for LocalSession {
    fn resident_bytes(&self) -> u64 {
        self.source_bytes
            + declaration_bytes(&self.reader)
            + self.hierarchy.resident_bytes()
            + self
                .activity
                .as_ref()
                .map_or(0, super::activity::ActivitySource::resident_bytes)
    }
    fn activity(&self) -> Option<Arc<crate::data::ActivityIndex>> {
        self.activity
            .as_ref()
            .and_then(super::activity::ActivitySource::index)
    }
    fn activity_build_info(&self) -> Option<super::ActivityBuildInfo> {
        self.activity
            .as_ref()
            .map(super::activity::ActivitySource::info)
    }
    fn activity_identity(&self) -> Option<vtr::activity::Identity> {
        self.activity.as_ref().map(|a| a.identity)
    }
    fn activity_image(&self, cache_dir: Option<&std::path::Path>) -> anyhow::Result<Vec<u8>> {
        #[cfg(not(target_family = "wasm"))]
        {
            self.activity
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("this trace has no activity source"))?
                .image(cache_dir.filter(|_| !self.temporary))
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = cache_dir;
            anyhow::bail!("byte-backed traces have no sidecar")
        }
    }
    fn build_activity(
        &self,
        options: &vtr::activity::BuildOptions,
        budget: &crate::remote::memory::MemoryBudget,
        cache_dir: Option<&std::path::Path>,
    ) -> anyhow::Result<()> {
        let source = self
            .activity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this trace cannot build an activity index"))?;
        source.build(
            options,
            budget,
            cache_dir.filter(|_| !self.temporary),
            self.info.signal_count,
            |w| vtr::activity::build(&self.reader, w, options),
        )
    }
    fn load_track(
        &self,
        track: super::transactions::TrackRef,
    ) -> anyhow::Result<super::loaded_tracks::LoadedTrack> {
        super::vtr_transactions::load_track(self, track)
    }
    fn tracks(&self) -> &[super::transactions::Track] {
        &self.tracks
    }
    fn capabilities(&self) -> crate::session::Capabilities {
        crate::session::Capabilities {
            waveforms: true,
            transactions: true,
            relations: true,
        }
    }
    fn format(&self) -> Option<&'static str> {
        Some("VTR")
    }
    fn info(&self) -> &TraceInfo {
        &self.info
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    fn load_signal(&self, signal: SignalRef) -> anyhow::Result<Arc<dyn SignalHistory>> {
        let data = self
            .reader
            .load_signal(vtr::SignalId(signal.0))
            .with_context(|| format!("load signal {}", signal.0))?;
        Ok(Arc::new(VtrHistory {
            shape: shape_of(
                data.kind(),
                self.reader
                    .declarations()
                    .nodes
                    .iter()
                    .find_map(|node| match node.data {
                        NodeData::Signal {
                            signal: id,
                            var_type,
                            alias: false,
                            ..
                        } if id.0 == signal.0 => Some(vtr::VarType::from_code(var_type)),
                        _ => None,
                    })
                    .context("signal declaration")?,
            ),
            data,
        }))
    }
}

struct VtrHistory {
    shape: SignalShape,
    data: SignalData,
}

fn to_wave_value(v: SignalValue<'_>) -> WaveValue {
    match v {
        SignalValue::Bits { .. } => WaveValue::Bits(v.to_ascii()),
        SignalValue::Real(r) => WaveValue::Real(r),
        SignalValue::VarLen(bytes) => WaveValue::Bytes(bytes.to_vec()),
    }
}

fn first_bit(v: SignalValue<'_>) -> Bit {
    match v {
        SignalValue::Bits { states, data, .. } => {
            let code = vtr::signal::get_code(data, states, 0);
            match code {
                0 => Bit::Zero,
                1 => Bit::One,
                2 | 4 | 5 => Bit::X,
                3 => Bit::Z,
                8 => Bit::DontCare,
                _ => Bit::Other,
            }
        }
        _ => Bit::Other,
    }
}

impl SignalHistory for VtrHistory {
    fn value_view(&self, i: Option<usize>) -> super::value_view::ValueView<'_> {
        use super::value_view::{LogicView, ValueView};
        match i.map_or_else(|| self.data.initial(), |i| self.data.get(i)) {
            SignalValue::Bits {
                width,
                states,
                data,
            } => ValueView::Logic(LogicView::packed_lsb(u64::from(width), states, data)),
            SignalValue::Real(value) => ValueView::Real(value),
            SignalValue::VarLen(bytes) => ValueView::Bytes(bytes.into()),
        }
    }
    fn resident_bytes(&self) -> u64 {
        fn owned_bytes(value: &vtr::OwnedSignalValue) -> usize {
            match value {
                vtr::OwnedSignalValue::Bits { data, .. } | vtr::OwnedSignalValue::VarLen(data) => {
                    data.capacity()
                }
                vtr::OwnedSignalValue::Real(_) => 0,
            }
        }
        (self.data.times.capacity() * std::mem::size_of::<u64>()
            + self.data.values.capacity() * std::mem::size_of::<vtr::OwnedSignalValue>()
            + owned_bytes(&self.data.initial)
            + self.data.values.iter().map(owned_bytes).sum::<usize>()) as u64
    }

    fn shape(&self) -> SignalShape {
        self.shape
    }
    fn always_normal(&self) -> bool {
        !matches!(self.data.kind(), vtr::SignalKind::Bits { states, .. } if states > 2)
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    fn time(&self, i: usize) -> u64 {
        self.data.times()[i]
    }
    fn index_at(&self, t: u64) -> Option<usize> {
        self.data
            .times
            .partition_point(|time| *time <= t)
            .checked_sub(1)
    }
    fn value(&self, i: Option<usize>) -> WaveValue {
        match i {
            None => to_wave_value(self.data.initial()),
            Some(i) => to_wave_value(self.data.get(i)),
        }
    }
    fn bit(&self, i: Option<usize>) -> Bit {
        match i {
            None => first_bit(self.data.initial()),
            Some(i) => first_bit(self.data.get(i)),
        }
    }
}
