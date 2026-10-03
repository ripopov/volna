# Volna server

`volna-server` serves one immutable VTR or FST recording over framed stdin and
stdout. It supplies metadata, selected complete signal histories, complete
transaction tracks, and raw activity sidecars. Diagnostics go to stderr.
It uses [`volna-trace`](../volna-trace/README.md) and has no viewer or GUI dependency.

## Build and start

From the repository root, with Rust 1.96 or newer and a C compiler:

```sh
cargo build --locked --release -p volna-server
target/release/volna-server --help
target/release/volna-server path/to/recording.vtr
```

On Windows the executable is `target/release/volna-server.exe`.
The last command expects a protocol client on stdin. It does not start an HTTP
service or print a human-readable trace. A host can launch it as a child process
or connect its pipes through another transport. Keep stdout exclusively for
protocol bytes, and drain stderr separately. Local loading with `volna-trace`
opens files in process and does not launch this executable.

## Session lifecycle

The client first sends `Open { max_object_bytes }` with session ID zero, sequence
zero, and a nonzero request ID. A successful response assigns a fresh nonzero
session ID and transfers the recording header, scope pages, and variable pages.
The header announces how many hierarchy pages to expect.

Subsequent requests carry that session ID, sequence zero, and strictly increasing
request IDs. The server handles one request at a time. Every response packet,
including an error or object end, requires a matching acknowledgement before
the server sends the next packet. Use `volna_trace::remote::client::RemoteClient`
to manage decoding and client admission; the host provides pipe I/O and scheduling.

| Command | Result |
| --- | --- |
| `Signals(ids)` | One complete history or explicit error per distinct signal, with at most 64 IDs per request |
| `Track(id)` | Complete records and incident relations for a catalog stream or generator |
| `Activity { build: false }` | An existing, validated raw activity sidecar, or an error |
| `Activity { build: true }` | The sidecar, building it first if needed |
| `Close` | Release the reader and exit |

An object becomes complete only after its `End` marker and exact declared byte
count. Per-object failures are returned as `Error` responses. Invalid framing,
request identities, acknowledgements, or a changed recording terminate the
connection. EOF between requests releases the reader; EOF during a frame or
before a required acknowledgement fails the transfer.

The server checks file length and modification time after opening and around
loads. This detects ordinary file changes; it is not a snapshot mechanism.
Files must remain unchanged throughout a session. Live recordings are unsupported.

## Framing and object limits

Protocol **6** is independent of the VTR file-format version. Peers reject other
protocol versions; there is no negotiation. Each packet has a 12-byte header:

| Field | Encoding |
| --- | --- |
| Magic | Four bytes: `VLNA` |
| Version | Little-endian `u32`, currently 6 |
| Encoded body length | Little-endian `u32`, at most 1 MiB |

The body is an independent checksummed LZ4 frame containing a fixed-width,
little-endian bincode `Packet`. Objects span `Begin`, bounded `Data` packets
of at most 256 KiB, and `End`. The client specifies a per-object transfer limit
at open. This bounds serialized objects, not the server's process memory.
Track serialization borrows loaded records without keeping a second record cache.

The Rust definitions in `volna_trace::remote::transport` are the authoritative
wire schema. The service has no viewport, presentation, search, or VDB endpoints.

## Activity sidecars

The open catalog includes the activity source identity, cache availability,
and reader hostname. A build request uses the same VTR/FST activity builder as
local loading. A persistent sibling lock and a cache recheck coordinate server
processes. The sidecar is published atomically beside the trace, or in an
identity-keyed user cache when its directory is read-only.

A scan authorized by the client can finish and leave a reusable cache after a
disconnect. Cancelling the client's transfer prevents client installation; it
does not revoke a server scan already in progress. The server returns raw index
bytes. Classification and exact activity queries run on the client, using
complete signal reads for undecided identities.

## Verification

```sh
cargo test --locked -p volna-server --all-targets
cargo clippy --locked -p volna-server --all-targets --all-features -- -D warnings
```

Process tests launch the actual executable with isolated temporary files and
a watchdog. They compare local and remote VTR/FST results, aliases, full
transaction records, parallel relations, empty generators, explicit errors,
size limits, recording-change detection, and sidecar reuse between processes.
Hierarchy tests also exercise transfers across multiple metadata pages.
