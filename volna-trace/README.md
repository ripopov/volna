# Volna trace loading

`volna-trace` opens immutable VTR and FST recordings and exposes a common API
for local and remote access. It provides hierarchy metadata, complete signal
histories, transaction tracks, activity sidecars, memory admission, and a
cooperative remote client. It has no GUI toolkit or server-process dependency.

Use [`vtr`](../vtr/README.md) to write a recording or query its format-specific
features. Use `volna-trace` when a consumer needs the same objects from a local
file, an in-memory image, or [`volna-server`](../volna-server/README.md).

## Read a local recording

From the repository root, build and browse the Rust API:

```sh
cargo build --locked -p volna-trace
cargo doc --locked -p volna-trace --no-deps --open
```

The crate requires Rust 1.97.1 or newer. The workspace also needs a C compiler
for VTR's Zstandard dependency.

`OpenSpec::Path` detects the format from the file header, rather than its
extension. Opening and local queries block; a host should run them on its
worker executor. Opening a local recording does not start a server.

```rust
use volna_trace::OpenSpec;

let session = OpenSpec::Path("recording.vtr".into()).open()?;
println!("{} variables", session.hierarchy().var_count());

if let Some(variable) = session.hierarchy().vars().next() {
    let history = session.load_signal(variable.signal)?;
    if let Some(index) = history.index_at(100) {
        println!("value at tick 100: {:?}", history.value(Some(index)));
    }
}
Ok::<(), anyhow::Error>(())
```

Times are integer ticks in the recording's time base. `session.info().timescale`
is the decimal exponent of seconds per tick. `index_at(t)` finds the last
entry at or before `t`; it returns `None` before the first entry. Histories
retain changes at the same timestamp and preserve real values, byte strings,
and all supported logic states.

For an image already in memory, use `OpenSpec::Bytes { name, bytes }`. The name
labels the recording; its bytes determine the format. Native path access uses
a memory-mapped VTR reader or a buffered FST reader. Keep files unchanged while
sessions or objects borrowing their storage exist.

## Metadata and complete objects

`Session` separates resident metadata from potentially expensive loads:

| Operation | Result |
| --- | --- |
| `info()` and `hierarchy()` | Recording time range, timescale, scopes, and variable declarations |
| `capabilities()` and `tracks()` | Supported operations and the resident transaction catalog |
| `load_signal()` and `load_signals()` | Complete immutable change histories; batched results follow request order |
| `load_track()` | Complete records and incident relations for a stream or generator, including empty generators |
| `activity()` | A shared activity index, if one was found or built |

Aliases refer to the same signal identity. A batched load can share their
history storage. An empty transaction catalog differs from an unsupported
operation: VTR supports transactions even in a waveform-only recording; FST
does not support transaction or relation loading.

An activity sidecar helps identify signals that are quiet or changing in a
time window. Its classification can leave signals undecided; use
`resolve_activity()` for an exact answer. Path-backed sessions can build an
index with `build_activity()`, a memory budget, and VTR's `BuildOptions`.
`BuildControl` provides progress and cancellation. Byte images and recovered
recordings have no stable activity build source. See the
[activity sidecar specification](../vtr/docs/SPEC.md#activity-index-sidecar-version-1).

## Remote loading

`remote::client::RemoteClient` queues raw loads and consumes protocol responses.
The host launches or connects to the server, transports packets, and schedules
decoding. The library supplies correlation, validation, memory admission, and
complete immutable results.

A host follows this cycle:

1. Create a `RemoteClient` with a shared `MemoryBudget` and an object limit;
   its constructor prepares the open request.
2. Send packets returned by `take_command()` using the transport encoder.
3. Pass decoded response packets to `accept()`.
4. On `ClientStep::Yield`, schedule `step()` for a later turn. On `Ack`, send
   the returned acknowledgement. On `Complete`, send its acknowledgement and
   deliver the `LoadResult` to the caller.
5. Queue signal, track, or activity work after a session has opened. On a
   rejected or empty submission, deliver the immediate completion returned by
   `submit()`. On a disconnect, call `disconnect()` to fail pending work and
   release its storage.

Only an explicit, validated object end makes a result available. Caller tags
and generations identify completions without entering the wire protocol.
`OpenSpec::Remote` describes an asynchronous open; calling its blocking
`open()` method returns an error. Remote sessions likewise route loads through
`RemoteClient`, rather than their blocking local query methods.

The [server guide](../volna-server/README.md) describes process hosting and
protocol version 6. Local and remote objects have the same data contract;
transport carries runtime facts. Viewport state, search, value translation,
transaction layout, VDB design semantics, and annotations belong to consumers.

## Memory and format limits

`MemoryBudget` accounts for retained objects and construction allowances.
Reservations follow shared owners and release when the last owner is dropped.
`account_local_session()` applies the same admission owner to local metadata
and loads. Limits can change while the budget is live; lowering a limit
refuses new reservations without evicting existing objects.

These limits do not bound total process memory, reader caches, or every
allocation made before admission. Complete metadata and selected histories or
tracks must fit in memory. Remote queries load complete objects; there are no
window-query or live-recording endpoints. FST recordings with nonzero time
offsets or dump-off regions are rejected explicitly. In a single-threaded WASM
host, local byte-image decoding can block; remote decoding yields cooperatively.

## Verification

```sh
cargo test --locked -p volna-trace --all-targets
cargo test --locked -p volna-trace --doc
cargo clippy --locked -p volna-trace --all-targets --all-features -- -D warnings
```

Tests cover local VTR/FST agreement, aliases, transaction semantics, compact
hierarchy transfer, corrupted and incomplete responses, cooperative decoding,
and memory ownership. All binary fixtures are committed under
[`tests/fixtures`](tests/fixtures/README.md); no simulator or submodule is needed.
The `testing` feature exposes procedural recordings for downstream headless tests.
