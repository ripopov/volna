# VTR transactions

Status: proposed design; not implemented. [SPEC.md](SPEC.md) describes the
current format. Adoption requires a format version bump and regeneration of
older trace assets. This document specifies the transaction subsystem; the
container, compression, string table, and general hierarchy framing are defined
in SPEC.md. Numeric section kind allocation and the version number must be assigned when this
proposal is integrated into the container specification.

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

Embedding all stage and event data in owners makes transaction inspection
convenient, but forces pipeline views to decode owner transactions to discover
activity at a location. Storing only independent location records makes
pipeline loading convenient, but leaves transaction inspection needing a
search for its children. The format must support both access patterns through
independently queryable located records and explicit owner references.

Readers should select relevant blocks by generator and time, discover mandatory
table columns from declarations, and resolve related records on demand. Writers
must sustain high trace throughput: encode argument values without repeating
their schema, release completed child payloads promptly, and avoid retaining
stage history or checking resource exclusivity. The design must also preserve
unlocated stages and events for producers that need only transaction-local
detail.

## 1. Identity, time, and ownership

A transaction is a runtime record emitted by a generator. Ordinary transactions
represent intervals such as instructions, requests, transfers, or operations.
Located stages represent visits to design locations; located events and logs
represent points in time. Clock transactions represent steady stretches of
clock edges.

Transaction IDs are nonzero `u64` values unique within the file across all
record types. The writer allocates them in creation order, independently of
record timestamps and completion order. A parent is identified by transaction
ID alone. Relations can use ID zero to mean an endpoint outside the file;
zero is never a real transaction or parent ID.

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

A generator is a hierarchy node with a node ID, name, optional parent scope,
kind string, metadata attributes, and ordered argument schema. It may be a
root or a child of an ordinary scope. Its hierarchical path identifies its
source or design location. Declarations are immutable and must precede blocks
that use them. Additional generators may be declared during a run.

| Generator kind | Records | Storage |
|---|---|---|
| Producer-defined, such as `INSTRUCTION` or `TRANSACTION` | Ordinary transactions | TX_BLOCK |
| `CLOCK` | Clock stretches | TX_BLOCK |
| `EVENT` | Located events | EVENT_BLOCK |
| `STAGE` | Located stages | STAGE_BLOCK |
| `LOG` | Log messages | LOG_BLOCK |

The four special kind strings are reserved. Other kinds are producer-defined.
A generator's kind is separate from the per-record span kind of an ordinary
transaction. Scope membership does not imply a record kind or inheritance of
metadata or argument schemas.

Generator metadata consists of keyed typed values describing the source.
Argument declarations describe values supplied by every record. Metadata is
not copied into individual records and is not a default for missing arguments.

### 2.1 Mandatory arguments

Every generator declares an ordered list of `(name, type)` pairs. An empty
list is valid. Names are unique within the schema. Each record supplies exactly
one value of the declared type for every argument, in declaration order.
There are no optional positions or implicit default values.

The argument type set is:

| Tag | Type | Meaning |
|---|---|---|
| 1 | bool | Boolean |
| 2 | i64 | Signed 64-bit integer |
| 3 | u64 | Unsigned 64-bit integer |
| 4 | f64 | IEEE binary64 |
| 5 | str | Interned string ID |
| 6 | bytes | Opaque byte sequence |
| 10 | time | Unsigned value in file time units |
| 12 | pointer | Unsigned address |
| 17 | text | UTF-8 text, deduplicated within a block |

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

Readers can construct mandatory table columns from declarations without
reading records. The guarantee applies per generator. A table combining
several generators must handle differing schemas, such as by showing a union
of columns with empty cells where a generator does not declare a field.

### 2.2 Attributes and labels

Metadata, optional transaction attributes, unlocated record attributes, and
relation attributes are lists of unique string keys and typed values. A writer
rejects duplicate keys within an item. Readers preserve unknown keys.

| Tag | Value | Payload after tag |
|---|---|---|
| 0 | null | None |
| 1 | bool | Byte 0 or 1 |
| 2 | i64 | Signed varint |
| 3 | u64 | Unsigned varint |
| 4 | f64 | 8 bytes, little-endian IEEE binary64 |
| 5 | str | Varint string ID |
| 6 | bytes | Length-prefixed bytes |
| 7 | bits | Varint width, then `ceil(width/8)` packed bytes |
| 8 | logic | Varint width, then `ceil(width/4)` packed bytes |
| 9 | logic9 | Varint width, then `ceil(width/2)` packed bytes |
| 10 | time | Unsigned varint |
| 11 | enum | Signed varint value, varint literal string ID |
| 12 | pointer | Unsigned varint |
| 13 | fixed | Signed varint raw, signed varint scale; value is `raw * 2^-scale` |
| 14 | ufixed | Unsigned varint raw, signed varint scale |
| 15 | list | Varint count, then tagged values |
| 16 | map | Varint count, then string-ID keys and tagged values |
| 17 | text | Length-prefixed UTF-8 bytes |

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

An ordinary transaction contains:

```text
transaction ID
generator ID
begin timestamp
end timestamp
status
span kind
optional parent transaction ID
argument values, in declared order
optional keyed attributes
unlocated events: [(timestamp, name, attributes), ...]
unlocated stages: [(name, lane, begin, optional end, attributes), ...]
located events: [(generator ID, transaction ID), ...]
located stages: [(generator ID, transaction ID), ...]
```

| Status | Code | Meaning |
|---|---|---|
| unset | 0 | No status supplied |
| ok | 1 | Completed successfully |
| error | 2 | Completed with an error |
| aborted | 3 | Cancelled, squashed, or flushed |
| open | 4 | Unfinished when the file closed |

Span kinds are `unspecified` (0), `internal` (1), `server` (2), `client` (3),
`producer` (4), and `consumer` (5). They classify an individual operation and
are independent of the generator's kind string. All lists may be empty.
Ordinary parentage does not require an inverse child list; the located child
lists specifically enumerate EVENT and STAGE records.

### 3.1 Unlocated events and stages

An unlocated event belongs to one ordinary transaction and contains a time,
name, and keyed attributes. An unlocated stage contains a name, lane name,
begin, optional end, and keyed attributes. Names and lanes are interned
strings. A lane groups stage intervals within a transaction; it is not a
hierarchy node or an independently queryable generator.

These records have no independent transaction ID. They cannot be relation
endpoints or runtime parents. Their order preserves insertion order rather
than timestamp order. Repeated names, repeated lanes, overlapping intervals,
and records outside the owner's time interval are permitted.

An absent stage end means open through the owner's end. Its effective end is
`max(stage.begin, owner.end)`. Explicit ends must be at least the stage begin.
Readers preserve the distinction between an absent end and an explicit end.

### 3.2 Located child references

Each located child reference includes both its generator ID and transaction
ID. The generator must have the corresponding EVENT or STAGE kind. The
referenced child's parent must equal the owner's ID. Each child occurs exactly
once in the appropriate owner list. Multiple visits to one generator are
allowed and have distinct transaction IDs. List order is insertion order.

A located child's owner is an ordinary transaction; LOG, EVENT, STAGE, and
CLOCK records do not own located children. This keeps specialized records
limited to their stated fields. The generator ID in a reference lets a reader
select candidate blocks without first resolving the child by global ID.

## 4. Located events and stages

An EVENT record contains:

```text
transaction ID
generator ID
timestamp
mandatory parent transaction ID
argument values, in declared order
```

A STAGE record contains:

```text
transaction ID
generator ID
begin timestamp
end timestamp
mandatory parent transaction ID
argument values, in declared order
```

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

A log record contains its transaction ID, generator ID, timestamp, optional
parent transaction ID, and argument values in declaration order. A parent
identifies the operation during which the message was produced. Logs have
zero duration, implied status `unset`, and implied span kind `unspecified`.
They can participate in relations and be resolved through global ID lookup.
They have no embedded events, stages, or child reference lists.

Format strings support `{}` placeholders, explicit indexes such as `{2}`,
format specifications such as `{:#010x}` and `{:>8.3}`, and escaped braces
`{{` and `}}`. Argument count is determined by the schema, not by parsing the
format. A missing placeholder argument renders as `{?}`. Readers expose the
typed named arguments independently of message formatting.

Text arguments are deduplicated within each block. Use `text` for dynamic or
repeated message strings, `str` for strings explicitly interned in the file
string table, and `bytes` for opaque payloads.

### 5.1 Run ending

The root scope `vtr.run` is reserved for run-ending logs. An abnormal ending
is described by one record from a FATAL LOG generator under this scope, at
the current writer time, as the last log record:

| Generator format string | Ordered arguments | Meaning |
|---|---|---|
| `run stopped by signal {}` | signal: i64 | Stop request followed by normal close |
| `run exited with status {}` | status: i64 | Program exited without closing |
| `run crashed by signal {} (code {}, address {}, thread {}, sealed {})` | signal: i64, code: i64, address: pointer, thread: u64, sealed: bool | Fatal signal; sealed means only data already handed to the encoder was retained |
| `run recovered by scanning, {} bytes dropped` | dropped: u64 | Recovery rewrote the file |
| `run sealed after a panic in the writer` | Empty | Writer failed mid-call and retained encoded data |

Signal numbers belong to the producing platform. A reader reports the last
record under `vtr.run`. Without such a record, a file with a final directory
indicates normal close; a file recovered without a directory has an unknown
ending and is reported as recovered.

## 6. Relations

A relation contains a kind string, source transaction ID, destination
transaction ID, and keyed attributes. It is a directed edge. Either endpoint
may be zero to mean outside the file; every nonzero endpoint refers to any
of the globally identified record types. Relations have no separate
transaction ID or timestamp and are stored in TX_BLOCK independently of
ordinary transaction records.

Kinds are producer-defined, such as dependency or causality classifications.
Structural ownership uses parent fields. A producer may additionally emit a
`parent_of` relation, but readers do not require it to interpret parentage.
Relations may refer to records stored later in the file.

## 7. Clocks

A CLOCK generator records steady stretches of rising edges as ordinary
transaction records. Its schema is empty. A stretch begins at its first edge
and ends at its last edge. For multiple edges, `vtr.period` is a positive
`time` attribute and `end - begin` must be an exact multiple of the period.
A single-edge stretch has equal begin and end and omits the period.

A stretch has no parent, events, stages, or located child references. Its
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
before the writer's current time, rather than inventing a new edge.

## 8. Storage

Records are distributed across column streams in compressed blocks. They are
not contiguous on-disk rows. The container supplies section framing, CRC,
compression codec, directory entries, and string and hierarchy declarations.
All referenced strings and generators must be declared before their block.

Fixed-width integers are little-endian. `varint` means unsigned base-128;
`svarint` means ZigZag followed by varint. Delta columns reset at each block
and use zero as their initial previous value. ID and time deltas use wrapping
64-bit subtraction interpreted as signed, with inverse wrapping addition on
decode, so the full unsigned domain remains representable.

Optional parents use a varint: zero for absent, otherwise
`(zigzag(id - parent) << 1) | 1`. This packed expression is evaluated as an
integer without truncation and can require 65 bits for full-range IDs.
Decoders must accept that width in this column. EVENT and STAGE require a
nonzero encoded parent and a nonzero resolved parent ID.

Every decompressed block starts with `varint column_count`, followed by that
many varint column byte lengths, then the concatenated column streams. Counts
and declarations determine how records consume columns. All columns must be
consumed exactly. Unknown columns or invalid lengths are errors for this
format version.

### 8.1 Argument columns

Every block type has these five argument streams. Records consume arguments
in declaration order, selecting streams by the declared type:

| Stream | Contents |
|---|---|
| numeric | bool as byte; i64 as svarint; u64, time, pointer as varint |
| floating | f64 as 8 little-endian bytes |
| text | Varint index into the block dictionary |
| dictionary | Varint entry count, then length-prefixed UTF-8 entries |
| miscellaneous | str as varint string ID; bytes as length-prefixed bytes |

Dictionary entries are distinct text values in first-occurrence order. Empty
dictionaries still encode a zero count. No argument count, key, or type tag
appears per record. The metadata needed to decode a record is its generator
schema, not its owner's payload.

### 8.2 LOG_BLOCK, EVENT_BLOCK, and STAGE_BLOCK

These section payloads begin with:

```text
u64 record_count
u64 min_id, max_id                 zero for an empty block
u64 t_min, t_max                   zero for an empty block
u32 generator_count
u32 compressed_blob_length
generator_count * u32 generator_node_id
compressed blob
```

The generator list is unique and ordered by first occurrence. It names exactly
the generators represented by records. A block contains only records of its
own kind. Time bounds are the smallest timestamp and largest timestamp for
LOG and EVENT, or smallest begin and largest end for STAGE.

LOG and EVENT have nine columns:

| Index | Column | Encoding |
|---|---|---|
| 0 | generator | Varint block-local generator index |
| 1 | time | Svarint timestamp delta |
| 2 | id | Svarint ID delta |
| 3 | parent | Packed optional-parent encoding; mandatory for EVENT |
| 4–8 | arguments | Numeric, floating, text, dictionary, miscellaneous |

STAGE has ten columns: indexes 0–8 are identical, with column 1 holding begin
time deltas, and column 9 holds varint durations `end - begin`.

### 8.3 TX_BLOCK

The section payload begins with:

```text
u64 transaction_count, relation_count
u64 min_id, max_id                 ordinary transaction IDs; zero if none
u64 t_min, t_max                   smallest begin, largest end; zero if none
u64 rel_min_from, rel_max_from, rel_min_to, rel_max_to
u32 generator_count
u32 compressed_blob_length
generator_count * u32 generator_node_id
compressed blob
```

The generator list is unique, sorted by node ID, and names the ordinary
transactions and clocks physically present. Located child references do not
add their generators to this list. Relation endpoint bounds cover all stored
endpoints, including zero; all four bounds are zero when no relations exist.

TX_BLOCK has 35 columns:

| Index | Column | Encoding |
|---|---|---|
| 0 | id | Svarint transaction ID delta |
| 1 | generator | Varint node ID |
| 2 | begin | Svarint begin delta |
| 3 | duration | Varint `end - begin` |
| 4 | status/kind | Byte `(span_kind << 4) | status` |
| 5 | parent | Packed optional-parent encoding |
| 6 | attribute count | Varint count per keyed attribute list |
| 7 | attribute key | Varint string ID |
| 8 | attribute tag | Byte value tag, upper three bits zero |
| 9 | attribute numeric | bool byte; signed/unsigned varints according to type |
| 10 | attribute f64 | 8 little-endian bytes |
| 11 | attribute string | Varint string ID for str or enum literal |
| 12 | attribute miscellaneous | Payload for tags 6–9 and 15–17 |
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
| 29 | located event count | Varint per transaction |
| 30 | located event generator | Varint node ID per reference |
| 31 | located event ID | Varint transaction ID per reference |
| 32 | located stage count | Varint per transaction |
| 33 | located stage generator | Varint node ID per reference |
| 34 | located stage ID | Varint transaction ID per reference |

Keyed attribute lists are consumed for each transaction, then each of its
unlocated events, then each of its unlocated stages; relation attribute lists
follow all transactions. Arguments are consumed once per transaction using
its generator schema. Child references consume their lists in insertion order.

Attribute numeric payloads follow the value table. An enum consumes its
signed numeric value and its literal from the string stream. Fixed and ufixed
consume raw and scale from the numeric stream. Lists and maps in the
miscellaneous stream retain nested tags. Attribute text is inline, while
schema argument text uses the dictionary. The stage-end sentinel arithmetic
uses sufficient width for `u64::MAX + 1`, without wrapping to the open marker.

### 8.4 Block order and queries

Blocks can appear in any order relative to other transaction block kinds and
signal blocks. Ordinary transactions and stages are appended on completion;
events and logs preserve production order. Parallel encoders must preserve
that order within each output sequence. Block ID and time ranges may overlap.
Chronological consumers must sort; they cannot assume file order is time order.

Time-window selection uses conservative block bounds, followed by per-record
checks. Generator selection uses complete block membership lists. A scope
query resolves descendant generators from the hierarchy. Log severity queries
can skip blocks whose generators are all below the threshold. ID queries use
block ID ranges; relations use their source and destination ranges.

Header filters do not provide direct record offsets. A candidate block may
contain unrelated records requiring decoding. Readers should build
header-based generator-to-block indexes, batch referenced IDs by candidate
block, and cache decoded blocks for repeated navigation. Loading a located
track must not require decoding owners. Selecting its owner may still require
a global ID-range search because the parent stores no generator ID.

## 9. Writer lifecycle

The writer declares a generator and its schema before emitting records.
Creating an ordinary transaction allocates its ID and opens an owner handle.
Creating a located child requires an active owner, allocates the child's ID,
and appends its reference to that owner. Events and logs can be encoded
immediately; stages can be encoded as soon as their ends and arguments are
known. Required arguments must be supplied before a record is finalized,
including finalization at file close. A missing argument is a writer error;
close must not fabricate default values or emit an incomplete argument list.

Ending an owner finalizes its child set, closes any remaining located stages,
and makes the owner eligible for serialization. No children may be attached
after finalization. Completed child payloads may be released after encoding;
only their reference pairs remain with the owner. Output buffers may flush
owners and children in either order.

At file close, unfinished ordinary transactions receive status `open` and end
`max(begin, current_waveform_time)`. Their active located stages close at that
effective owner end, clamped to their begins. Unlocated stage ends may remain
absent and are interpreted through the owner's end. Explicit writer end calls
clamp an end earlier than its begin; malformed on-disk negative durations are
not valid. Clock finalization follows the last-edge rule in section 7.

The writer checks schema arity and types, key uniqueness, generator kind,
active-owner validity, and local duration constraints. Its handle-based
interface can maintain inverse child references without a history scan.
Producer-supplied cross-record invariants are not grounds for retaining all
completed records. In particular, the writer never retains completed stage
intervals to enforce exclusivity.

Memory consists of declarations, active records, reference lists of active
owners, and bounded encoding buffers. A long-lived owner can accumulate many
child references even after their payloads have been released. Memory is not
bounded independently of active owner size, but does not grow with the total
completed history of a located generator.

## 10. Reader validation and recovery

Readers reject invalid encodings, undeclared generators, mismatched record
and generator kinds, unsupported schema types, malformed counts, invalid
status/kind codes, and out-of-range indexes. Block counts and bounds must
agree with decoded contents. Parent and child-reference consistency, globally
unique IDs, acyclic ownership, and relation endpoint existence are complete-file
invariants. Full validation may check them separately from ordinary lazy
queries; loading a track must not trigger a whole-file ownership scan.

Forward references are valid. A complete finalized file must resolve every
nonzero relation endpoint, parent, and located child reference. A growing or
recovered file may lack referenced records whose blocks were never written.
Readers expose unresolved references as missing and preserve available records;
they do not fabricate children or owners. Recovery uses complete container
sections and their integrity checks.

Readers expose arguments under their declared names, preserve optional
attributes, distinguish embedded records from located references, and retain
all records involved in resource overlaps. Cached decoded results must remain
valid for callers while held, even if the reader evicts its own cache entry.

## 11. Performance and validation criteria

The format supports independent transaction and pipeline views. Declared
schemas remove repeated field keys and tags; independently stored located
records avoid owner decoding for track loads; explicit references avoid
searching all children when expanding an owner. Costs include IDs and parents
for located records, reference pairs on owners, and candidate-block decoding
for navigation.

Measure writer throughput and peak memory, cold track loading, owner expansion,
and repeated navigation with caching. Exercise ordinary transactions with and
without schemas, mixed generators, out-of-order times, long-lived owners,
repeated stage visits, overlaps, coincident points, open records, logs, clocks,
relations, forward references, and incomplete-file recovery. Block sizes and
generator mixing must be evaluated on representative traces before adding
finer persistent indexes or per-generator output buffers.
