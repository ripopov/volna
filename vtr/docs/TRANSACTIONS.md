# VTR transactions

Status: proposed; not implemented.

This document specifies the transaction model, encoding, and reader and writer
behavior. [SPEC.md](SPEC.md) defines the current container, compression, string
table, and hierarchy framing. Adoption requires a new format version, assigned
section kind codes, and regeneration of older trace assets.

Every transaction record has a file-wide ID. Events and stages associated with
design locations are stored separately from their owning transactions, grouped
by generator and packed into independently compressed pages. Each owner lists
the blocks containing its children. The recommended writer settings are 64 KiB
pages and Zstandard level 1. Section 12 gives the performance rationale.

## Notation and common types

Rust-style declarations describe the logical data model. They do not prescribe
a programming interface, memory layout, or serialization order. Section 8
defines the byte layout.

`i8`, `i16`, `i32`, and `i64` are signed integers; `u8`, `u16`, `u32`, and
`u64` are unsigned integers. The suffix gives the width in bits. A signed
N-bit integer ranges from `-2^(N-1)` to `2^(N-1) - 1`; an unsigned N-bit
integer ranges from zero to `2^N - 1`. `bool` is Boolean; `f32` and `f64` are
IEEE 754 binary32 and binary64, respectively.
`Vec<T>` is an ordered, variable-length sequence of `T`; `Option<T>` is either
absent or one `T`. `(A, B)` is a pair. Collections may be empty unless a
constraint says otherwise. An `enum` selects one named variant and its payload.

```rust
type TransactionId = u64;
type NodeId = u32;
type GeneratorId = NodeId;
type StringId = u32;
type Timestamp = u64;
type BlockNumber = u64;
type Bytes = Vec<u8>;
type Utf8Text = Vec<u8>;
```

`TransactionId` and `BlockNumber` are nonzero identifiers in separate
namespaces. `NodeId` indexes the container hierarchy; `GeneratorId` identifies
a generator node in that hierarchy. `StringId` indexes the file string table.
Hierarchy and string indexes are zero-based. `Timestamp` uses the file's time
units. `Bytes` is opaque data; `Utf8Text` must contain valid UTF-8.
Record-specific types and their constraints are defined below.

## Problem statement

A trace viewer must efficiently display both transactions and the pipeline
stages they pass through. These require two complementary views of the same
execution:

- A transaction view follows an instruction or other transaction through its
  lifetime, showing its stages, events, and attributes.
- A pipeline view follows a design location over time, showing which
  transactions occupy a stage or produce events there.

For example, selecting an instruction should reveal its fetch, decode, and
execute visits; selecting an execute slot should show visits from all
instructions within the visible time window. Users must be able to navigate
from either view to the corresponding records in the other.

An *owner* is the ordinary transaction to which an event or stage belongs.
A *located* event or stage has its own generator, identifying a design
location. An *unlocated* event or stage is embedded in its owner and is
accessible through that owner.

Located records support direct queries by location and time. Owners list the
blocks containing their located children, limiting the search needed to
reconstruct a transaction's activity. Together, these structures support both
views without duplicating child records.

Readers select blocks by generator and time, derive argument columns from
generator declarations, and resolve related records on demand. Writers encode
arguments without repeating their schema and release completed child payloads
promptly. Producers that need only transaction-local detail can use unlocated
events and stages.

![The same stage visits grouped by instruction above and by pipeline location below. Stage 103 belongs to instruction 42 and occupies execute.slot0 from time 4 to 7 in both views.](diagrams/transaction-views.drawio.svg)

The highlighted visit is one stored record. The transaction view finds it in
the blocks listed by its owner and matches its parent ID; the pipeline view
queries its generator.

## 1. Identity, time, and ownership

A transaction is a runtime record emitted by a generator. Ordinary transactions
represent intervals such as instructions, requests, transfers, or operations.
Located stages represent visits to design locations; located events and logs
represent points in time. Clock transactions represent steady stretches of
clock edges.

Transaction IDs, also called record IDs, are nonzero `u64` values unique within
the file across all record types. The writer allocates strictly increasing IDs
in creation order, independently of timestamps and completion order. IDs never
wrap or get reused; ID exhaustion is a writer error. A parent is identified by
transaction ID alone. Relations can use ID zero to mean an endpoint outside
the file; zero is never a real transaction or parent ID.

Times are unsigned `u64` values in the file's time units. Ordinary intervals
and located stages require `end >= begin`. Occupancy of a positive-duration
stage is `[begin, end)`; zero-duration stages and events are point records.
Timestamps need not be monotonic, including within a block. Record timestamps
are independent of the writer's waveform time.

Hierarchy parentage describes design structure. Transaction parentage describes
runtime ownership. Neither implies the other, and runtime ownership does not
require temporal containment. Parent relationships must be acyclic. Physical
record order does not determine ownership, and references may point forward
in the file.

## 2. Generators and declarations

A generator is a hierarchy node whose path identifies a source or design
location. It may be a root or a child of an ordinary scope. Declarations are
immutable and must precede blocks that use them. Additional generators may
be declared during a run.

```rust
struct Generator {
    id: GeneratorId,
    name: StringId,
    parent_scope: Option<NodeId>,
    kind: StringId,
    metadata: Attributes,
    arguments: Vec<ArgumentDeclaration>,
}
```

`Attributes` is defined in section 2.2. `parent_scope`, when present, must
identify an ordinary scope.

| Generator kind | Records | Storage |
|---|---|---|
| Producer-defined, such as `INSTRUCTION` or `TRANSACTION` | Ordinary transactions | TX_BLOCK |
| `CLOCK` | Clock stretches | TX_BLOCK |
| `EVENT` | Located events | RECORD_BLOCK |
| `STAGE` | Located stages | RECORD_BLOCK |
| `LOG` | Log messages | RECORD_BLOCK |

The four special kind strings are reserved. Other kinds are producer-defined.
A generator's kind is separate from the per-record span kind of an ordinary
transaction. Scope membership does not imply a record kind or inheritance of
metadata or argument schemas.

Generator metadata consists of keyed typed values describing the source.
Argument declarations describe values supplied by every record. Metadata is
not copied into individual records and is not a default for missing arguments.

### 2.1 Mandatory arguments

```rust
type ArgumentType = u8;

struct ArgumentDeclaration {
    name: StringId,
    value_type: ArgumentType,
}
```

Every generator declares an ordered list of arguments. An empty list is
valid. Names are unique within the schema. Each record supplies exactly
one value of the declared type for every argument, in declaration order.
There are no optional positions or implicit default values.

`ArgumentType` accepts only the tags below. Values use the corresponding
`Value` variant from section 2.2.

| Tag | Type | Value variant | Meaning |
|---|---|---|---|
| 1 | bool | `Bool` | Boolean |
| 2 | i8 | `I8` | Signed 8-bit integer |
| 3 | i16 | `I16` | Signed 16-bit integer |
| 4 | i32 | `I32` | Signed 32-bit integer |
| 5 | i64 | `I64` | Signed 64-bit integer |
| 6 | u8 | `U8` | Unsigned 8-bit integer |
| 7 | u16 | `U16` | Unsigned 16-bit integer |
| 8 | u32 | `U32` | Unsigned 32-bit integer |
| 9 | u64 | `U64` | Unsigned 64-bit integer |
| 10 | f32 | `F32` | IEEE binary32 |
| 11 | f64 | `F64` | IEEE binary64 |
| 12 | str | `Str` | Interned string ID |
| 13 | text | `Text` | UTF-8 text, deduplicated within one column set (section 8.1) |
| 14 | bytes | `Bytes` | Opaque byte sequence |
| 18 | time | `Time` | Unsigned value in file time units |
| 20 | pointer | `Pointer` | Unsigned address |

The generator-specific hierarchy payload is `varint kind_string_id`, then
`varint argument_count`, then that many `{varint name_string_id, u8 type_tag}`
pairs. The enclosing hierarchy record supplies its ID, name, parent and metadata.
Names and kind strings use the file string table. A schema change requires a
new generator declaration with a distinct node ID.

Arguments are exposed as named attributes by readers. Ordinary transactions
can also carry optional keyed attributes; these must not duplicate a declared
argument name. Located events, located stages, and logs carry only their
schema-defined arguments. Their encoded records omit argument counts, keys,
and type tags.

Readers can determine each generator's argument columns from its declaration
without reading records. Tables combining generators must account for their
different schemas.

### 2.2 Attributes and labels

Metadata, optional transaction attributes, unlocated record attributes, and
relation attributes are lists of unique string keys and typed values. A writer
rejects duplicate keys within an item. Readers preserve unknown keys.

```rust
type Attributes = Vec<(StringId, Value)>;

enum Value {
    Null,
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    Str(StringId),
    Text(Utf8Text),
    Bytes(Bytes),
    Bits { width: u64, packed: Bytes },
    Logic { width: u64, packed: Bytes },
    Logic9 { width: u64, packed: Bytes },
    Time(u64),
    Enum { value: i64, literal: StringId },
    Pointer(u64),
    Fixed { raw: i64, scale: i64 },
    Ufixed { raw: u64, scale: i64 },
    List(Vec<Value>),
    Map(Attributes),
}
```

`Time` holds a timestamp or duration in file time units, according to the
field's meaning. `Pointer` holds an unsigned address. `Bits`, `Logic`, and
`Logic9` hold packed signal states; `width` counts signal bits. Their packed
lengths and the wire tags for all variants are defined below.

| Tag | Value | Payload after tag |
|---|---|---|
| 0 | null | None |
| 1 | bool | Byte 0 or 1 |
| 2 | i8 | Signed varint |
| 3 | i16 | Signed varint |
| 4 | i32 | Signed varint |
| 5 | i64 | Signed varint |
| 6 | u8 | Unsigned varint |
| 7 | u16 | Unsigned varint |
| 8 | u32 | Unsigned varint |
| 9 | u64 | Unsigned varint |
| 10 | f32 | 4 bytes, little-endian IEEE binary32 |
| 11 | f64 | 8 bytes, little-endian IEEE binary64 |
| 12 | str | Varint string ID |
| 13 | text | Length-prefixed UTF-8 bytes |
| 14 | bytes | Length-prefixed bytes |
| 15 | bits | Varint width, then `ceil(width/8)` packed bytes |
| 16 | logic | Varint width, then `ceil(width/4)` packed bytes |
| 17 | logic9 | Varint width, then `ceil(width/2)` packed bytes |
| 18 | time | Unsigned varint |
| 19 | enum | Signed varint value, varint literal string ID |
| 20 | pointer | Unsigned varint |
| 21 | fixed | Signed varint raw, signed varint scale; value is `raw * 2^-scale` |
| 22 | ufixed | Unsigned varint raw, signed varint scale |
| 23 | list | Varint count, then tagged values |
| 24 | map | Varint count, then string-ID keys and tagged values |

Tags group signed integers, unsigned integers, and floating-point values into
contiguous ranges, with increasing width within each family. Argument schemas
and tagged attributes use the same assignments.

Integer types preserve both signedness and width. Signed values use the
64-bit ZigZag encoding after sign extension; unsigned values use unsigned
varints. Writers and readers reject values outside the tagged or declared
type's range; narrowing must not truncate or wrap. Integer width determines
the permitted range, not a fixed payload length. These rules also apply to
values nested in lists and maps.

Floating-point payloads preserve their IEEE 754 bit patterns, including signed
zero, infinities, and NaN payloads. An `f32` value occupies four bytes and is
not widened to `f64`.

Packed signal-state codes follow the container's value definitions. A
length-prefixed byte sequence is a varint byte length followed by those bytes.
Maps have unique keys. Unknown type tags are errors.

Keys beginning with `vtr.` and `log.` are reserved. Imported keys must not be
rewritten into reserved namespaces. Producer-specific keys should use a tool
prefix. The reserved `vtr.label` attribute or declared argument must have type
`str` or `text`; it supplies an instance label. Otherwise a transaction uses
its generator name, an unlocated event or stage uses its own name, and a
relation uses its kind. Labels carry no color, layout, or ordering semantics.

## 3. Ordinary transactions

```rust
type TransactionStatus = u8;
type SpanKind = u8;

struct Transaction {
    id: TransactionId,
    generator: GeneratorId,
    begin: Timestamp,
    end: Timestamp,
    status: TransactionStatus,
    span_kind: SpanKind,
    parent: Option<TransactionId>,
    arguments: Vec<Value>,
    attributes: Attributes,
    events: Vec<UnlocatedEvent>,
    stages: Vec<UnlocatedStage>,
    child_blocks: Vec<BlockNumber>,
}
```

`TransactionStatus` accepts only the following codes:

| Status | Code | Meaning |
|---|---|---|
| unset | 0 | No status supplied |
| ok | 1 | Completed successfully |
| error | 2 | Completed with an error |
| aborted | 3 | Cancelled, squashed, or flushed |
| open | 4 | Unfinished when the file closed |

`SpanKind` accepts `unspecified` (0), `internal` (1), `server` (2), `client` (3),
`producer` (4), and `consumer` (5). They classify an individual operation and
are independent of the generator's kind string. Child and optional attribute
lists may be empty; the argument list must match the generator's schema.
Ordinary parentage does not require an inverse child list. Located child block
locators cover only EVENT and STAGE records.

### 3.1 Unlocated events and stages

```rust
struct UnlocatedEvent {
    time: Timestamp,
    name: StringId,
    attributes: Attributes,
}

struct UnlocatedStage {
    name: StringId,
    lane: StringId,
    begin: Timestamp,
    end: Option<Timestamp>,
    attributes: Attributes,
}
```

Each unlocated event or stage belongs to one ordinary transaction. Names and
lanes are interned strings. A lane groups stage intervals within a transaction;
it is not a hierarchy node or an independently queryable generator.

These records have no independent transaction ID. They cannot be relation
endpoints or runtime parents. Their order preserves insertion order rather
than timestamp order. Repeated names, repeated lanes, overlapping intervals,
and records outside the owner's time interval are permitted.

An absent stage end means open through the owner's end. Its effective end is
`max(stage.begin, owner.end)`. Explicit ends must be at least the stage begin.
Readers preserve the distinction between an absent end and an explicit end.

### 3.2 Located child block locators

An owner stores the sorted, distinct block numbers of every RECORD_BLOCK
containing at least one of its located EVENT or STAGE children. Block numbers
identify sections within the file as defined in section 8; they are not
transaction IDs or byte offsets. The list contains no TX_BLOCK locators and
no blocks without a matching child. LOG records do not contribute locators,
even when their parent is this owner.

To expand an owner, the reader selects EVENT and STAGE segments in the listed
blocks, then selects records whose parent equals the owner's ID. LOG segments
are excluded. Every located child must be covered by its owner's list.
Multiple visits to one generator remain distinct records with
distinct global IDs. Sorting the selected children by ID reconstructs their
creation order, which is also their insertion order within the owner. A reader
can expose separate event and stage lists by generator kind.

A located child's owner is an ordinary transaction; LOG, EVENT, STAGE, and
CLOCK records do not own located children. Each located child is created after
its owner, so its parent ID is smaller than its own ID. This does not impose
timestamp order or temporal containment.

For example, owner 42 may list blocks `[12, 15]`. Block 12 contains stage 103
with generator 18 and parent 42; block 15 contains event 104 with generator 19
and parent 42. Either block may also contain logs or other owners' records;
these are excluded from the result. Neither the generator IDs nor the
individual child IDs are repeated in owner 42.

Locators identify blocks, not individual pages or rows. Section 8.4 defines
how readers retrieve an owner's children.

## 4. Located events and stages

```rust
struct LocatedEvent {
    id: TransactionId,
    generator: GeneratorId,
    time: Timestamp,
    parent: TransactionId,
    arguments: Vec<Value>,
}

struct LocatedStage {
    id: TransactionId,
    generator: GeneratorId,
    begin: Timestamp,
    end: Timestamp,
    parent: TransactionId,
    arguments: Vec<Value>,
}
```

`LocatedEvent` uses an EVENT generator; `LocatedStage` uses a STAGE generator.
Both require an ordinary transaction as their parent. Arguments follow the
generator's declared schema.

Each generator represents a specific design location. Repeated activity at
that location produces separate records. A general transaction interface
exposes EVENT with equal begin and end, and both types with status `unset`
and span kind `unspecified`. These implied fields are not stored. Unlocated
and located records may coexist within one owner.

STAGE records always store a finite end. Ending an owner closes any active
located stages at `max(stage.begin, owner.end)`. This does not encode whether
that end was explicit or imposed by owner finalization. Producers needing
that distinction must declare an argument for it.

### 4.1 Resource occupancy

Producers are responsible for exclusivity when a STAGE generator represents
one resource slot. Simultaneous independent slots use separate generators.
Overlaps do not invalidate the file and are not rejected by the writer or
reader. Neither retains completed stage history solely to check exclusivity.

The viewer detects overlapping positive-duration intervals on a located stage
track and renders the conflicting portions as `XXX`, retaining every record
for inspection. Zero-duration stages do not occupy an interval. Coincident
EVENT records are valid and must remain individually accessible.

## 5. Logs

A LOG generator represents a logging call site. Its name is the message format
string. Its argument schema supplies argument names and types in placeholder
order. Producers without argument names use `"0"`, `"1"`, and so on.

| Metadata | Type | Meaning |
|---|---|---|
| `log.severity` | u64 | Trace 0, debug 1, info 2, warn 3, error 4, fatal 5; default 2 |
| `log.file` | str | Optional source file |
| `log.line` | u64 | Optional source line |
| `log.func` | str | Optional source function |

Other severity values rank numerically. Source metadata identifies the log
call site. Design-source mappings and presentation remain outside VTR.

```rust
struct Log {
    id: TransactionId,
    generator: GeneratorId,
    time: Timestamp,
    parent: Option<TransactionId>,
    arguments: Vec<Value>,
}
```

`generator` identifies a LOG generator, and arguments follow its declared
schema. A parent identifies the operation during which the message was
produced. Logs have zero duration, implied status `unset`, and implied span
kind `unspecified`. They can participate in relations and be resolved through
global ID lookup.
They have no embedded events, stages, or located child block locators.

Format strings support `{}` placeholders, explicit indexes such as `{2}`,
format specifications such as `{:#010x}` and `{:>8.3}`, and escaped braces
`{{` and `}}`. Argument count is determined by the schema, not by parsing the
format. A missing placeholder argument renders as `{?}`. Readers expose the
typed named arguments independently of message formatting.

Text arguments are deduplicated within each generator segment. Use `text` for
dynamic or repeated message strings, `str` for strings explicitly interned in
the file string table, and `bytes` for opaque payloads.

### 5.1 Run ending

The root scope `vtr.run` is reserved for run-ending logs. An abnormal ending
is described by one record from a FATAL LOG generator under this scope, at
the current writer time, as the last log created, with the greatest log ID:

| Generator format string | Ordered arguments | Meaning |
|---|---|---|
| `run stopped by signal {}` | signal: i64 | Stop request followed by normal close |
| `run exited with status {}` | status: i64 | Program exited without closing |
| `run crashed by signal {} (code {}, address {}, thread {}, sealed {})` | signal: i64, code: i64, address: pointer, thread: u64, sealed: bool | Fatal signal; sealed means only data already handed to the encoder was retained |
| `run recovered by scanning, {} bytes dropped` | dropped: u64 | Recovery rewrote the file |
| `run sealed after a panic in the writer` | Empty | Writer failed mid-call and retained encoded data |

Signal numbers belong to the producing platform. A reader reports the record
with the greatest ID under `vtr.run`. Without such a record, a file with a final
directory indicates normal close; a file recovered without a directory has an
unknown ending and is reported as recovered.

## 6. Relations

```rust
type RelationEndpoint = u64;

struct Relation {
    kind: StringId,
    source: RelationEndpoint,
    destination: RelationEndpoint,
    attributes: Attributes,
}
```

A relation is a directed edge. A `RelationEndpoint` is either zero, meaning
outside the file, or a nonzero `TransactionId` referring to any of the globally
identified record types. Relations have no separate
transaction ID or timestamp and are stored in TX_BLOCK independently of
ordinary transaction records.

Kinds are producer-defined, such as dependency or causality classifications.
Structural ownership uses parent fields. A producer may additionally emit a
`parent_of` relation, but readers do not require it to interpret parentage.
Relations may refer to records stored later in the file.

## 7. Clocks

A CLOCK generator records steady stretches of rising edges using the
`Transaction` structure from section 3. Its schema is empty. A stretch begins
at its first edge and ends at its last edge. For multiple edges, `vtr.period`
is a positive `time` attribute and `end - begin` must be an exact multiple of the period.
A single-edge stretch has equal begin and end and omits the period.

A stretch has no parent, events, stages, or located child block locators. Its
span kind is `unspecified` and its status is `unset`, or `open` if still
running at file close. Stretches of the same clock never share an edge:
each begins after the preceding stretch's last edge.

A stretch with period `p` has edges `begin + k*p`, for
`k = 0 .. (end - begin)/p`. The first recorded edge is cycle zero; numbering
continues across stretches in chronological order. At time `t`, the cycle
is the number of the last edge at or before `t`, or absent before the first
edge. Cycle numbers are computed rather than stored.

The generator metadata `vtr.clock`, of type `str`, associates any generator
with a clock by its hierarchical path joined with `.`. An absent association
means no declared clock. Clock generators are normally named after the clock
net and placed in its scope to permit path-based waveform matching.

Writers should place a clock's stretches in blocks containing only that
generator. At close, an active stretch ends at its last recorded edge at or
before the writer's current time.

## 8. Storage

The container supplies section framing, CRC, compression codecs, directory
entries, and string and hierarchy declarations. All referenced strings and
generators must be declared before their block. RECORD_BLOCK stores located
events, located stages, and logs in a paged layout. TX_BLOCK holds ordinary
transactions, clocks, and relations in one compressed column set.

Both block kinds carry a nonzero `u64 block_number`, unique across RECORD_BLOCK
and TX_BLOCK within the file. The writer assigns increasing numbers when
records enter a block, before background encoding. Numbers never wrap or
get reused and need not match physical section order. Readers map them to
section offsets using block headers and the container directory, or a section
scan during recovery. The container directory-entry layout is unchanged.

A *segment* is a sequence of records from one generator, with its own column
set and delta state. A *page* is an independently compressed blob containing
one or more complete segments. A generator may span several segments and
pages within a block. Segment headers select generators and time ranges before
decompression; segment byte offsets select columns after their page is decoded.

### 8.1 Column sets and scalar encodings

Fixed-width integers are little-endian. `varint` means unsigned base-128;
`svarint` means ZigZag followed by varint. A column set starts with
`varint column_count`, then that many varint column byte lengths, followed by
the concatenated column streams. All columns must be consumed exactly.
Unknown columns or invalid lengths are errors for this format version.

Delta state resets to zero at each segment, or at the beginning of a TX_BLOCK.
Signed ID, parent, and time differences use wrapping 64-bit subtraction,
interpreted as signed, with inverse wrapping addition on decode. This permits
the full unsigned domain and negative differences. Duration addition must not
overflow: `begin + duration` must fit in `u64`.

An optional parent is encoded as varint zero for absent, otherwise
`zigzag(delta)`, where `delta` is the wrapping difference `id - parent`,
interpreted as signed. A present parent cannot equal the record's ID because
self-parentage is a cycle. The nonzero difference distinguishes a parent
reference from absence. Decoding a nonzero value subtracts its inverse ZigZag
value from the record ID with wrapping arithmetic; the resolved parent must be
nonzero. Writers reject a present zero or self parent before encoding.
This encoding applies to ordinary transactions
and logs. Located events and stages use the mandatory parent delta column in
section 8.2.

Unsigned scalar varints are limited to 64 bits, except the unlocated stage-end
sentinel in section 8.3, which can require 65 bits. Writers use the shortest
base-128 representation. Only signed delta and offset reconstruction wraps.

Each column set has these five argument streams. Records consume arguments
in declaration order, selecting streams by the declared type:

| Stream | Contents |
|---|---|
| numeric | bool as byte; i8/i16/i32/i64 as svarint; u8/u16/u32/u64, time, pointer as varint |
| floating | f32 as 4 little-endian bytes; f64 as 8 little-endian bytes |
| text | Varint index into this column set's text dictionary |
| dictionary | Varint entry count, then length-prefixed UTF-8 entries |
| miscellaneous | str as varint string ID; bytes as length-prefixed bytes |

Dictionary entries are distinct text values in first-occurrence order. Empty
dictionaries encode a zero count. A dictionary belongs to one segment or one
TX_BLOCK; no page depends on another page's dictionary or delta state.
Argument counts, keys, and type tags come from the generator declaration and
are omitted from records.

### 8.2 RECORD_BLOCK

A RECORD_BLOCK may contain any combination of EVENT, STAGE, and LOG generators.
Each segment contains records from one generator, whose declared kind selects
the time, parent, and duration encodings. No per-record kind tag is stored.

The section payload has the following layout:

```text
u64 block_number
u64 record_count
u64 min_id, max_id
u64 t_min, t_max
u32 segment_count, page_count

segment_count * {
    u32 generator_node_id
    u32 page_index
    u64 record_count
    u64 raw_offset, raw_length
    u64 min_id, max_id
    u64 t_min, t_max
}

page_count * {
    u64 blob_offset, blob_length
    u64 raw_length
}

page blobs
```

Blocks, segments, and pages are nonempty. Only EVENT, STAGE, and LOG generators
are permitted. The segment table gives complete generator membership; no
per-record generator column is stored. Segment ID bounds describe its records.
Segment time bounds are minimum and maximum timestamps for LOG and EVENT, or
minimum begin and maximum end for STAGE. Block bounds are the extrema over all
segments, including when kinds are mixed. The block record count is the sum of
segment record counts.

Page indexes are zero-based. Each page blob uses SPEC.md section 3.1 framing
and is independently decompressible. Blob offsets are relative to the start
of the page-blob area. Pages occur in index order, with no gaps or overlaps;
their lengths consume the remainder of the section. The page table's raw
length must agree with both the blob framing and the decompressed length.

Segment offsets refer to decompressed bytes in their page. The segment table
is ordered by page index and then raw offset. Segments partition each page
without gaps or overlaps, and each segment contains exactly one column set.
Repeated entries for a generator are allowed. Their table order defines its
record sequence within the block. Counts, offsets, and length arithmetic must
fit their encoded fields and remain within the containing page or section.

Every segment uses nine columns. The generator kind determines their contents:

| Index | Column | Encoding |
|---|---|---|
| 0 | time | LOG/EVENT: svarint timestamp delta; STAGE: svarint `begin - previous_end` |
| 1 | id | Svarint record ID delta |
| 2 | parent | LOG: optional-parent encoding; EVENT/STAGE: svarint parent ID delta |
| 3 | duration | STAGE: varint `end - begin`; LOG/EVENT: empty column |
| 4–8 | arguments | Numeric, floating, text, dictionary, miscellaneous |

For a STAGE segment, `previous_end` initially equals zero and advances to the
decoded end of each record. Negative gaps encode overlaps and out-of-order
times without a special case. Mandatory parent deltas are relative to the
previous parent in that segment, initially zero. A zero delta repeats that
parent; it never means absent. The reconstructed parent must be nonzero and
smaller than the child ID.

The recommended writer policy targets 64 KiB of decompressed bytes per page,
including column framing and text dictionaries. Split large generator groups
between records into independently encoded segments, and pack smaller segments
together up to that target. A record cannot span segments or pages. A record
whose column set alone exceeds the target occupies a page by itself; the
target is not a format limit. Readers accept other page sizes. The writer must
account for framing and dictionary bytes when deciding whether a page fits.

Pages may contain segments of different kinds. Writers may close a page or
block early to separate logs from events and stages when selective reading
benefits. Readers must accept both mixed and single-kind pages and blocks.

Zstandard level 1 is recommended. Each blob may use any container-supported
codec and settings. Store uncompressed bytes when compression does not reduce
the page size. Section 12 compares compression and throughput.

### 8.3 TX_BLOCK

The section payload begins with:

```text
u64 block_number
u64 transaction_count, relation_count
u64 min_id, max_id                 ordinary and clock IDs; zero if none
u64 t_min, t_max                   smallest begin, largest end; zero if none
u64 rel_min_from, rel_max_from, rel_min_to, rel_max_to
u32 generator_count
u32 compressed_blob_length
generator_count * u32 generator_node_id
compressed blob
```

The generator list is unique, sorted by node ID, and names the ordinary
transactions and clocks physically present. Located child blocks do not add
their generators to this list. Relation endpoint bounds cover all stored
endpoints, including zero; all four bounds are zero when no relations exist.
A relation-only block has no generators or transactions. A block with neither
transactions nor relations is not emitted.

The decompressed blob contains one column set with 31 columns:

| Index | Column | Encoding |
|---|---|---|
| 0 | id | Svarint transaction ID delta |
| 1 | generator | Varint block-local generator index |
| 2 | begin | Svarint begin delta |
| 3 | duration | Varint `end - begin` |
| 4 | status/kind | Byte `(span_kind << 4) \| status` |
| 5 | parent | Optional-parent encoding from section 8.1 |
| 6 | attribute count | Varint count per keyed attribute list |
| 7 | attribute key | Varint string ID |
| 8 | attribute tag | Byte value tag, upper three bits zero |
| 9 | attribute numeric | bool byte; signed/unsigned varints according to type |
| 10 | attribute floating | f32 as 4 little-endian bytes; f64 as 8 little-endian bytes |
| 11 | attribute string | Varint string ID for str or enum literal |
| 12 | attribute miscellaneous | Payload for tags 13–17 and 23–24 |
| 13 | unlocated event count | Varint per transaction |
| 14 | unlocated event time | Svarint offset from owner begin |
| 15 | unlocated event name | Varint string ID |
| 16 | unlocated stage count | Varint per transaction |
| 17 | unlocated stage name | Varint string ID |
| 18 | unlocated stage lane | Varint string ID |
| 19 | unlocated stage begin | Svarint offset from owner begin |
| 20 | unlocated stage end | Varint 0 for open, otherwise `end - begin + 1` |
| 21 | relation kind | Varint string ID |
| 22 | relation from | Svarint source ID delta |
| 23 | relation to | Svarint destination offset from source ID |
| 24–28 | arguments | Numeric, floating, text, dictionary, miscellaneous |
| 29 | located child block count | Varint per transaction |
| 30 | located child block number | Varint first number, then positive deltas within that owner's list |

The located block list is sorted and distinct. Its delta state resets for each
owner, independently of other columns. Zero first numbers and zero subsequent
deltas are invalid; reconstruction must not overflow. CLOCK records have a
zero block count. No individual located child ID or generator ID is stored in
the owner.

Keyed attribute lists are consumed for each transaction, then each of its
unlocated events, then each of its unlocated stages; relation attribute lists
follow all transactions. Arguments are consumed once per transaction using
its generator schema. Embedded child lists retain insertion order.

Attribute numeric payloads follow the value table. An enum consumes its
signed numeric value and its literal from the string stream. Fixed and ufixed
consume raw and scale from the numeric stream. Floating-point attributes share
the floating stream; each value's tag determines whether it consumes four or
eight bytes. Argument floating widths are determined by the generator schema.
Lists and maps in the miscellaneous stream retain nested tags. Attribute text is inline, while
schema argument text uses the dictionary. The stage-end sentinel arithmetic
uses sufficient width for `u64::MAX + 1`, without wrapping to the open marker.

TX_BLOCK uses the same recommended Zstandard level-1 policy as paged blocks.
Its owner and relation columns remain in one blob; page packing and stable
generator grouping apply only to RECORD_BLOCK.

### 8.4 Order and queries

The writer assigns ordinary transactions and stages to blocks on finalization;
events and logs are assigned on emission. RECORD_BLOCK has one assignment
sequence shared by EVENT, STAGE, and LOG; TX_BLOCK has a separate sequence for
ordinary transactions, clocks, and relations. Blocks contain consecutive
portions of their sequence, with block numbers increasing along that sequence.
Background encoders may complete out of order. Declarations must still
precede their uses in the physical file, and readers must not infer
record order from file offsets.

Within each paged block, stable grouping preserves assignment order for each
generator. Segment splitting also preserves that order. Across generators,
physical order has no semantic meaning and does not recover stage completion
order. Global IDs recover creation order, including log and event production
order. TX_BLOCK preserves transaction finalization order within its sequence.
Neither creation nor finalization order implies timestamp order.

For a closed query window `[a, b]`, where `a <= b`, candidate blocks and segments
satisfy `t_min <= b` and `t_max >= a`. A positive-duration stage intersects the
window when `begin <= b` and `end > a`; a point matches when
`a <= time <= b`. Ordinary transaction queries use `begin <= b` and `end >= a`.
Clock queries select stretches by these bounds and derive edges from the
period. Embedded event and stage times do not expand TX_BLOCK bounds.

A located-track query selects matching generator segments by their headers,
decompresses their pages once each, and decodes only the selected segments.
It then filters individual records by time. Pages may contain unrelated
segments, which incur decompression but not record-decoding work. A scope query
resolves descendant generators from the hierarchy. Log severity queries filter
generators by their declared metadata. Owner payloads are not required to load
located intervals, points, or their declared arguments.

Expanding an owner uses its block locators to select EVENT and STAGE segments,
decompresses their pages once each, and selects children by parent ID. LOG
segments are skipped even if their records have the same parent. Pages holding
only LOG segments need no decompression for this query; mixed pages may still
require decompressing log data. Owner time bounds cannot filter children
because ownership does not require temporal containment. A reader may
build a transient parent-to-record index while decoding a block and reuse it
for later expansions. The format does not contain an owner-to-page index;
uncached owner expansion may read every page containing EVENT or STAGE segments
in each referenced block.

Global ID lookup filters block ID ranges and, for paged blocks, segment ID
ranges. Relation lookup uses TX_BLOCK endpoint ranges. Ranges can overlap,
including when a long-lived owner finishes alongside recently created owners.
Header ranges select candidates rather than providing an exact ID-to-row map.
Readers should batch parent and label lookups by candidate block and cache
decoded owners.

Readers build header-based block-number and generator/segment indexes. They
cache decoded pages and records on demand. Section CRC verification still
covers the entire section payload and may require reading the other compressed
pages; independent pages bound decompression, not the I/O needed for full
section integrity verification.

## 9. Writer lifecycle

The writer declares a generator and its schema before emitting records.
Creating an ordinary transaction allocates its ID and opens an owner handle.
Creating a located child requires an active owner and allocates its global ID.
Events and logs supply complete arguments when created and are submitted
immediately; stages can be submitted once their ends and arguments are known.
Required arguments must be supplied before finalization, including at file
close. Missing arguments are a writer error; close must not fabricate values
or emit an incomplete argument list.

The submission path assigns each finalized record to a block before handing
the block to a background encoder. One ordered stream of bounded buffers
serves EVENT, STAGE, and LOG records in RECORD_BLOCK; another serves TX_BLOCK.
A block's first record reserves its block number. For a located child,
assignment also records that number in the active owner. A record must
never migrate to another numbered block during grouping, page packing, or
compression: its owner may already have been serialized with that locator.

The writer maintains one block list for each active owner. RECORD_BLOCK
numbers increase along the shared submission stream, so a located EVENT or
STAGE child appends a number only when it differs from that owner's last
entry. Logs do not update the list. The list is already sorted and distinct
at owner finalization, without a merge, per-child hash set, or scan of
completed records.

Ending an owner closes remaining active stages at
`max(stage.begin, owner.end)` and submits them to numbered blocks before
serializing the owner. It then fixes the owner's child block list. No children
may be attached after finalization. The owner need not wait for child page
encoding or physical I/O; forward block references are valid. The owner retains
no completed child payload after submission. Buffer and queue ownership keeps
that payload alive until encoding finishes.

The recommended RECORD_BLOCK buffer limits are 65,536 finalized records and
4 MiB of buffered record payload; reaching either limit flushes the block.
An oversized record is accepted alone. Byte accounting must include
variable-length arguments.
Bounded encoder queues apply backpressure when producers outrun encoding.
TX_BLOCK uses a separate byte budget, also 4 MiB, so large embedded
details and relations also count toward its memory budget.

Encoding gathers records into generator buckets in first-occurrence order,
preserving order within each bucket. This linear pass requires no ID or
timestamp sorting. The encoder splits buckets into segments and packs pages
as specified in section 8.2. Grouping and encoding state, including argument
dictionaries and pending pages, is discarded after the block is encoded.

At file close, unfinished ordinary transactions receive status `open` and end
`max(begin, current_waveform_time)`. Their active located stages close at that
effective owner end, clamped to their begins. Unlocated stage ends may remain
absent and are interpreted through the owner's end. Explicit writer end calls
clamp an end earlier than its begin; malformed on-disk negative durations are
not valid. Clock finalization follows the last-edge rule in section 7.

The writer checks schema arity and types, key uniqueness, generator kind,
active-owner validity, and local duration constraints. It does not retain
completed records to check cross-record properties such as stage exclusivity.

Memory consists of declarations, active records, distinct block locators of
active owners, bounded record/encoding buffers, and bounded queued work.
Per-owner locator memory grows with the number of touched blocks. In the worst
case of one child per block it still grows with child count. Large individual
records and active owners' embedded details can exceed the normal buffer
targets; memory is not bounded independently of active owner size.

## 10. Reader validation and recovery

Readers reject invalid encodings, undeclared generators, mismatched record
and generator kinds, unsupported schema types, malformed counts, invalid
status/kind codes, and out-of-range indexes. Page and segment byte ranges must
partition their containers as specified. Loaded column sets must consume every
column exactly, and decoded record counts and bounds must agree with headers.
Lazy queries validate the data they load; full validation additionally checks
unloaded pages and aggregate block bounds.

Globally unique record IDs and block numbers, acyclic ownership, relation
endpoint existence, and owner-locator consistency are complete-file invariants.
For each owner, its locator list must equal the set of RECORD_BLOCKs containing
EVENT or STAGE records with that parent. LOG records are excluded from this
invariant. Full validation may check these separately; loading one track must
not trigger a whole-file ownership scan.

Forward references are valid. A complete, normally finalized file must resolve
every nonzero relation endpoint, parent, and located block locator. A growing,
sealed, or recovered file may lack referenced records or blocks whose sections
were never written, even if recovery or sealing appended a final directory.
Readers preserve available records and expose missing owners, endpoints, and
child blocks. A missing child block makes that owner's child set incomplete;
its missing child IDs and count cannot be inferred from the locator alone.

Recovery retains complete container sections with valid integrity checks.
Pages do not introduce partial-section recovery: an incomplete block is dropped
as a whole. Recovery preserves surviving block numbers and does not renumber
them when rebuilding the directory, so existing locators remain meaningful.
Records or blocks appended during recovery must use identifiers above all
surviving or referenced values in their respective namespaces, including
unresolved references to missing data.

Readers expose arguments under their declared names, preserve optional
attributes, distinguish embedded records from located records, and retain all
records involved in resource overlaps. Cached decoded results remain valid
for callers while held, even if the reader evicts its own cache entry.

## 11. Performance and validation criteria

Performance evaluation covers writer CPU and memory, compressed storage, and
reader latency for both track queries and transaction inspection.

Before adoption, establish reproducible benchmarks covering the complete
writer, reader, and container, including owner payloads, argument dictionaries,
checksums, directory/header costs, and caches. Measure writer throughput and
peak memory, complete scans, uncached track loading, owner expansion, parent
label lookup, and repeated navigation. Report decoded-cache state separately
from filesystem-cache state and actual disk or remote I/O.

Exercise ordinary transactions with and without schemas, mixed generators,
skewed generator traffic, large arguments, out-of-order timestamps, long-lived
owners, repeated visits, overlaps, coincident points, open records, logs,
clocks, relations, and forward references. Include blocks and pages mixing
EVENT, STAGE, and LOG generators, and verify that a log whose parent is an
owner is excluded from that owner's located children. Verify segment splits reset all
delta and dictionary state; one page decodes independently; owner finalization
assigns every child block before serialization; and asynchronous completion
does not change locators or per-generator order. Test truncation after an
owner block but before its referenced child blocks, and the reverse order.

Persistent owner-to-page and exact ID indexes require evidence that faster
uncached owner expansion and label lookup justify their write, memory, and
storage costs.

## 12. Performance rationale

Prototype measurements support the recommended layout and writer settings.
The prototypes use different headers from section 8 and omit full container
framing, integrity checks, and directory costs. Results describe these workloads
and do not establish production throughput. The stage-only comparisons do not
measure the effect of batching events, stages, and logs in the same block.

### 12.1 Workloads and method

The synthetic model contains 200,000 instructions from a 4-wide out-of-order
pipeline, with seven located stages per instruction and 58 STAGE generators.
Execute latency and issue wait vary. Owners carry PC and opcode arguments;
stages carry no arguments. IDs follow creation order, stages enter blocks on
completion, and blocks hold at most 65,536 records. This model is not a
cycle-accurate resource-occupancy simulation.

The Kanata comparison extracts 631,118 stages from the first 100,000
instructions of a qsort trace. Generators correspond to nine distinct
`(thread, lane, stage name)` tuples; these are overlapping logical locations,
not reconstructed physical slots. Sixteen active stages close at the sample
cutoff. Owner payloads, labels, and relations are excluded from native timings.

Native results use an optimized Rust prototype, VTR's compression and varint
code, one pinned core of an Intel Core Ultra 7 265K, and Zstandard level 1.
Each reported timing is the median of five passes after a warm-up. Write time
includes grouping, optional dictionary construction and sorting, serialization,
and compression. Reads include decompression and record reconstruction with
compressed input in memory, reusable buffers, and no decoded-record cache.
All layouts passed record round trips and sampled query-result comparisons.

Track latency is the average per query within the median pass over 128
generator/block selections. A query reads one generator's records from one
located block. Full scans read all stages. Neither measures the public writer
interface, disk I/O, or owner/label resolution.

### 12.2 Stage storage and speed

Synthetic results, with compressed storage reported per stage:

| Layout | Bytes/stage | Write, million stages/s | Full read, million stages/s | Track query, microseconds |
|---|---:|---:|---:|---:|
| Mixed generators in one frame | 2.381 | 32.79 | 141.46 | 402.60 |
| Stable generator groups, one frame | 1.845 | 40.99 | 167.93 | 124.56 |
| Groups with an owner-ID dictionary | 1.626 | 26.08 | 174.57 | 133.46 |
| Owner dictionary and creation-ID sorting | 1.577 | 24.95 | 184.71 | 129.45 |
| **Stable groups, 64 KiB pages** | **2.001** | **37.38** | **154.47** | **35.59** |
| Pages with an owner-ID dictionary | 1.740 | 25.24 | 164.87 | 50.94 |
| One frame per generator | 2.276 | 34.64 | 127.29 | 9.85 |

Compared with one grouped frame, 64 KiB pages use about 8% more space, retain
91% of write throughput, and reduce track-query latency by a factor of 3.5.
Owner dictionaries save more space but substantially reduce write throughput.
Sorting by ID adds another encoding pass with little benefit for track queries.

Kanata stage-only results:

| Layout | Bytes/stage | Write, million stages/s | Full read, million stages/s | Track query, microseconds |
|---|---:|---:|---:|---:|
| Mixed generators in one frame | 0.632 | 53.39 | 191.42 | 298.23 |
| Stable generator groups, one frame | 0.487 | 44.37 | 222.71 | 114.51 |
| Groups with an owner-ID dictionary | 0.455 | 23.67 | 193.65 | 137.06 |
| **Stable groups, 64 KiB pages** | **0.558** | **39.79** | **205.66** | **42.99** |

On this more compressible trace, grouping slows writing relative to mixed
records. Pages still reduce storage and track-query latency. An owner
dictionary nearly halves grouped write throughput for a smaller storage saving
than on synthetic data. Direct parent deltas offer a better balance here.

### 12.3 Owners, codecs, and deferred alternatives

A separate Python encoding estimate for the synthetic owners gave 3.05 bytes
per stage for owners containing individual `(generator ID, child ID)` pairs,
and 0.45 for owners containing distinct child block locators. These values
include owner fields and arguments divided across seven stages. They do not
measure owner write throughput or the complete paged format. The locator
choice removes substantial repeated data without adding per-child dictionary
construction or changing global record identity.

Locators do not eliminate owner-expansion work. The native synthetic queries
that scan one located block for a parent's children averaged 339 microseconds
with one grouped frame, 363 with pages, and 451 with one frame per generator.
Those timings start with the correct child block already identified and omit
owner loading. Per-generator frames favor a narrow track query at the expense
of compression and some broader reads; bounded pages provide the chosen
compromise.

For synthetic pages, Zstandard level 3 reduced stage storage from 2.001 to
1.971 bytes but reduced write throughput from 37.38 to 33.42 million stages/s.
LZ4 used 2.632 bytes/stage, wrote 41.18 million stages/s, and reduced track
latency to 16.97 microseconds. On Kanata, LZ4 pages used 0.951 bytes/stage and
reduced track latency to 36.97 microseconds. These results support Zstandard
level 1 as the recommended setting, with LZ4 available when throughput takes
priority over storage size.

The encoding omits owner-ID dictionaries, extra ID sorting, and adaptive
integer bit-packing. The tested bit-packing variant reduced raw size but did
not improve compressed size on both workloads. Encoding times in units of a
shared time quantum also saved little after compression. These transforms
need further evidence to justify their encoding cost and format complexity.

Global IDs remain on every located record. An owner-local child ordinal could
replace them and still support relations if relation endpoints were redesigned
as compound identities. That would change the common identity model and lose
global creation order unless stored separately. The available size estimates
do not justify this added complexity.
