# VTR command-line tools

`vtr-cli` supplies the `vtr` executable and Rust converters for Kanata, VCD,
and FST. It requires Rust 1.97.1 or newer. Build or install from the workspace:

```sh
cargo build -p vtr-cli --locked
cargo install --path vtr-cli --locked
vtr --help
vtr convert --help
```

## Commands

| Command | Input | Purpose |
| --- | --- | --- |
| `info FILE` | VTR | Metadata, time base, hierarchy counts, and sections |
| `hierarchy FILE [--vars] [--depth N] [--sizes]` | VTR | Declarations or scope counts; aliases count once per scope's signal total |
| `value FILE SIGNAL TIME` | VTR | Signal value at a tick |
| `changes FILE SIGNAL [--from T] [--to T] [--max N]` | VTR | Changes in an inclusive window |
| `transactions FILE [--stream PATH] [--from T] [--to T] [--max N]` | VTR | Transactions, attributes, events, and stages |
| `transactions FILE --id ID[,ID…]` | VTR | Selected transactions and their incoming/outgoing relations |
| `logs FILE [--stream PATH] [--severity LEVEL] [--from T] [--to T] [--max N]` | VTR | Formatted messages; `--sites` lists declarations instead |
| `clocks FILE` | VTR | Declared clocks and edge stretches |
| `convert INPUT OUTPUT [--format kanata\|vcd\|fst]` | Kanata, VCD, FST | Create a VTR recording |
| `to-vcd INPUT OUTPUT` | VTR | Export waveforms |
| `recover INPUT OUTPUT` | VTR | Copy verified sections into a complete recording |
| `index FILE [--check] [--threads N] [--memory MiB]` | VTR, FST | Build or validate an activity sidecar |
| `active FILE [--from T] [--to T] [--build]` | VTR, FST | List changing signals once each, using their first declaration |

Times are unsigned integer ticks in the file's time base, with both window
ends included. An omitted bound means zero or the largest tick. Signal and
stream paths use dots; quote paths containing spaces. Query output defaults to
100 records where `--max` is available. Text output is intended for terminal
inspection, not as a stable serialization protocol.

Conversion, export, and recovery require a new destination path. They publish
output only after successful completion. Exit codes are 0 for success, 1 for
`index --check` with no valid sidecar, 2 for invalid arguments or processing
errors, and 3 for successful recovery that dropped bytes.

Activity commands use `volna-trace` for both formats. Sidecars are validated
against the source identity and written beside the trace, with the library's
user-cache fallback. `active` resolves undecided signals from the recording;
it requires an index or `--build`. `--memory` limits builder scratch, not total
process memory or the retained index. Keep recordings unchanged while reading.

## Conversion semantics and limits

- **Kanata 0004:** plain text or gzip, detected by its header. `C=`/`C` set or
  advance a nonnegative cycle; `I` creates an instruction; `L` labels the
  instruction, detail, or most recent stage; `S`/`E` delimit stages; `R` retires
  or flushes; `W` connects producer to consumer. Each thread is a pipeline
  stream using the period-one `cpu.cycle` clock. Ticks are abstract cycles
  (`time.unit=cycle`), not physical seconds. Repeated labels are newline joined.
  Instructions remain in memory until EOF for late labels and dependencies;
  unfinished instructions have Open status. Unknown commands, unsupported
  versions, dangling references, duplicate IDs, and backwards cycles are errors.
- **VCD:** standard declarations and the string extension, with aliases, bus
  ranges, four-state bits, repeated events, reals, and strings. Decimal
  timescale factors, producer version/date, and dump-control intervals are
  retained; a missing timescale defaults to nanoseconds. Values stream in
  timestamp order. Undeclared identifiers, incompatible aliases, overwide
  values, truncated commands, and backwards times are errors. Vendor-specific
  declarations, GHW, and EVCD are not supported.
- **FST:** the shared `volna-trace` session supplies hierarchy, aliases, scope
  and variable types, components, input/output/inout directions, events,
  nine-state logic, reals, and raw string/port bytes. Other directions become
  implicit and root variables appear under `(top)`. FST attributes, enum tables,
  and producer metadata are omitted. Nonzero time offsets and dump-off regions
  are rejected. All distinct histories must fit in memory. The CLI disables
  VTR deduplication to retain the session's samples.
- **VCD export:** supports two/four-state bits, events, and reals. Nine-state
  declarations, byte strings, unsupported timescales, time-zero offsets, and
  blackout intervals are rejected. Hierarchy names must be representable as VCD
  tokens, optionally followed by a bus range. Nonstandard scope kinds become
  modules and non-event bit declarations become wires. Transactions, logs,
  and declared clocks are omitted.

## Examples

Run from the workspace root; these use committed, hand-authored fixtures:

```sh
work=$(mktemp -d)
cargo run -p vtr-cli --locked -- convert vtr-cli/tests/fixtures/waves.vcd "$work/waves.vtr"
cargo run -p vtr-cli --locked -- value "$work/waves.vtr" 'top.bus [3:0]' 5
cargo run -p vtr-cli --locked -- active "$work/waves.vtr" --from 5 --to 7 --build
cargo run -p vtr-cli --locked -- to-vcd "$work/waves.vtr" "$work/waves.vcd"
cargo run -p vtr-cli --locked -- convert vtr-cli/tests/fixtures/pipeline.kanata "$work/pipeline.vtr"
cargo run -p vtr-cli --locked -- transactions "$work/pipeline.vtr" --stream cpu.thread0
cargo run -p vtr-cli --locked -- clocks "$work/pipeline.vtr"
```

Rust callers use `kanata::convert_kanata`, `vcd::convert_vcd`, or
`fst::convert_fst` with a fresh `vtr::Writer`, then explicitly close it.
Converters leave partial writer state after errors; discard that output.
`vcdout::write_vcd` writes to any `std::io::Write` sink. See the crate rustdoc
for a runnable example.

For FST activity in downstream tests, open `volna_trace::OpenSpec::Path`, then
use `Session::build_activity`, `activity`, and `resolve_activity`. Supply a
`BuildControl` in the build options and a `MemoryBudget`. There is no CLI-owned
FST reader or activity wrapper. Shared FST fixtures and their provenance are in
[`volna-trace/tests/fixtures`](../volna-trace/tests/fixtures/README.md).

## Verification and license

```sh
cargo test -p vtr-cli --locked
cargo clippy -p vtr-cli --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc -p vtr-cli --no-deps --locked
```

Tests cover conversion values and time bases, alias identity, Kanata lifecycle
semantics, malformed input, CLI queries, recovery, activity, and failed-output
cleanup. They require no simulator, network, GUI, or external checkout.

Licensed under MIT; see [LICENSE](../LICENSE).
