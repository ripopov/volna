//! # VTR: Versatile Trace Record
//!
//! An open trace store for hardware simulation: signal waveforms, transaction
//! streams, the elaborated design hierarchy and relations between
//! transactions in one random-access file.
//!
//! * [`Writer`] — streaming writer usable from a running simulator
//!   ([`writer`] has the ordering rules and value semantics).
//! * [`Reader`] — random-access, memory-mapped reader, `Send + Sync`
//!   ([`reader`] has the query guide and cost model).
//!
//! The data model:
//! * **Hierarchy** ([`hierarchy`]): a forest of scopes, variables, streams,
//!   generators and enum tables with typed [`Value`] attributes. A variable
//!   names a signal ([`SignalId`]); aliases share one. [`census`] counts
//!   the distinct signals below each scope.
//! * **Waveforms** ([`signal`]): value changes of 2/4/9-state bit vectors,
//!   reals and variable-length byte strings.
//! * **Transactions** ([`txblock`]): intervals of a generator with
//!   attributes, events, pipeline stages, a parent and a status, plus typed
//!   relations between them. Log records ([`logblock`])
//!   and clock stretches ([`clock`]) are transactions too.
//!
//! Strings are interned ([`StrId`]); every fallible call returns
//! [`Result`]. There is no global state: all settings are in
//! [`WriterOptions`] and [`ReadOptions`]. `block`, `container`, `varint` and
//! `xform` are format internals, public for inspection tools. The file format
//! is specified in `vtr/docs/SPEC.md` in the repository.
//!
//! VTR contains runtime data and identity only. Design-source mappings and
//! presentation belong in a separate VDB layer; this crate does not depend
//! on VDB, a viewer, or a simulator integration. Log-site source provenance
//! is the sole exception.
//!
//! # Writing and reading a waveform
//!
//! ```rust
#![doc = include_str!("../examples/waveform.rs")]
//! ```
//!
//! # Transactions
//!
//! ```rust
#![doc = include_str!("../examples/transactions.rs")]
//! ```

pub mod activity;
pub mod block;
pub mod census;
pub mod clock;
pub mod codec;
pub mod container;
pub mod ending;
pub mod error;
pub mod hierarchy;
pub mod hierarchy_index;
pub mod logblock;
pub mod logfmt;
pub mod reader;
pub mod sections;
pub mod signal;
pub mod strings;
pub mod txblock;
pub mod value;
pub mod varint;
pub mod writer;
pub mod xform;

pub use census::{Census, Contributions, ScopeSizes};
pub use clock::STREAM_KIND as CLOCK_STREAM_KIND;
pub use clock::{ClockId, ClockInfo, ClockTimeline, CycleAt, Stretch};
pub use codec::{Codec, Compression};
pub use ending::Ending;
pub use error::{Error, Result};
pub use hierarchy::{
    Direction, Hierarchy, Node, NodeData, NodeId, NodeKind, ScopeType, SignalId, SignalKind,
    VarType,
};
pub use logblock::STREAM_KIND as LOG_STREAM_KIND;
pub use logblock::{LogArg, LogArgType, LogRecord, LogSite, LogSiteId, LogSiteSpec, Severity};
pub use reader::{LogQuery, ReadOptions, Reader, SignalData, TxQuery};
pub use sections::{Blackout, FileType, Meta, Timescale};
pub use signal::{Logic, LogicStates, OwnedSignalValue, SignalValue};
pub use strings::StrId;
pub use txblock::{Relation, Transaction, TxAttr, TxEvent, TxId, TxKind, TxStage, TxStatus};
pub use value::Value;
pub use writer::{recover, CrashState, Sealer, Writer, WriterBuilder, WriterOptions, WriterStats};
