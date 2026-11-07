# Derived-signal playground

[`derived.vtr`](derived.vtr) and the reusable
[`derived.vtr.volna.example.json`](derived.vtr.volna.example.json) provide four prepared tabs,
25 derived waveforms and measurements, and two reusable sampling choices.
The recording contains ten digital playground signals, three real sensors,
five GPU performance counters, and two statistics sampling signals. It is under
100,000 bytes; generation enforces that limit.

```sh
# Open the prepared example directly.
cargo run --locked -p volna -- volna/examples/derived.vtr.volna.example.json
# Regenerate both assets from the bundled statistics excerpt.
cargo run --locked -p volna --example derived
# Generate elsewhere without changing the delivered example.
cargo run --locked -p volna --example derived -- /tmp/derived.vtr
cargo test --locked -p volna --example derived
```

In a running Volna window, choose **Open Workspace…** from the File menu or the
command palette and select `derived.vtr.volna.example.json`. Loading the example
keeps your autosave destination: normally `derived.vtr.volna.json`, or the
configured fallback storage. Reopen the example to reset the demo. To update the
example itself, choose **Save Workspace As…** and select its filename; that
one-time write does not redirect autosave. The generator also updates only the
example template and leaves your personal workspace alone.

The generator is [`derived.rs`](derived.rs). The streaming queue and sensors are
deterministic synthetic data. The GPU statistics are real interval counts copied
from a gem5 recording. These are independent workloads sharing a timeline;
the sensors do not measure the GPU experiment.

## Prepared views

| Tab | Demonstration |
| --- | --- |
| Handshake & queue | Rising-edge sampling, Boolean predicates, event-driven backpressure, packet conservation, captured packet/byte totals, and queue peak |
| Power & sensors | Multiplying real inputs, a moving average, time integration, signed bus interpretation, threshold detection, and physical-unit measurements |
| GPU statistics | Sampling on a bus change, interval bandwidth, traffic per clock, bank-service cost, and weighted aggregate ratios |
| Bandwidth samples [GB/s] | A saved table of all 64 derived bandwidth samples, created using the formula editor's table action; timestamps are in ps |

Each wave tab has its own zoom and cursor. The first opens at 48 clocks so that
handshakes remain readable; the other two show the whole recording. Formula rows
use the shared cyan tint. Open any formula row with **Derive** to inspect or edit
its expression and bindings.

**Sampling** offers `stream.clk` rising edges as the trace default and
`stats.sample_number` value changes as another saved choice. Saved formulas carry
their own sampling expression. **@* · Any input change** is appropriate for
combinational expressions such as backpressure and instantaneous power. Use
clock edges to count transfers and sample-number changes to sum statistics.

## Recorded signals

The clock period is 476 ps, preserving the source GPU shader period. There are
6,400 measured clocks and one additional clock of post-roll. The post-roll keeps
the last statistics boundary inside whole-range measurements. The synthetic
queue accepts packets on rising edges when `reset_n && valid && ready`, holds
offered payloads through backpressure, and services packets at a changing rate.
Periodic service pauses fill the eight-entry queue. Unfinished packets remain
in flight at the end; no completions are invented to drain it.

| Signal | Meaning |
| --- | --- |
| `stream.clk` | Rising edges at `238 + 476*n` ps |
| `stream.reset_n` | Active-low reset for the first four clocks |
| `stream.valid` | Producer offers a packet and holds it until accepted |
| `stream.ready` | The queue has space; no combinational bypass when full |
| `stream.payload_bytes` | Offered size, cycling through 16, 32, 64, and 128 bytes; zero when idle |
| `stream.queue_depth` | Accepted minus completed packets, bounded by eight |
| `stream.accepted_count` | Cumulative rising-edge handshakes |
| `stream.completed_count` | Cumulative serviced packets |
| `stream.accepted_bytes` | Cumulative accepted payload bytes |
| `sensors.adc_code` | Signed 12-bit two's-complement sine, 1,024 codes/V; unknown until the conversion at clock 10 |
| `sensors.supply_v` | Synthetic supply voltage with queue-dependent droop |
| `sensors.load_a` | Synthetic current with an idle component, queue-dependent load, and ripple |
| `sensors.temperature_c` | Illustrative first-order thermal response; not a calibrated device model |
| `stats.sample_number` | Zero baseline followed by interval numbers 1–64; changes every 100 clocks |
| `stats.clock_count` | Zero baseline followed by 100, 200, …, 6,400 elapsed excerpt clocks |
| `stats.lds_read_bytes` | LDS bank read bytes in the preceding 100 clocks, including atomic reads |
| `stats.lds_write_bytes` | LDS bank write bytes in the preceding 100 clocks, including atomic writes |
| `stats.lds_conflict_cycles` | Additional modeled bank-service cycles due to conflicts; not elapsed kernel stall time |
| `stats.lds_bank_accesses` | Serviced four-byte bank words; broadcasts share accesses and atomics count once |
| `stats.lds_requests` | Processed LDS memory packets; an atomic is one request |

The synthetic sensor histories update every ten clocks and hold their values
between conversions. Their real values are quantized to 1/4,096 for compact
encoding. The ADC spans ±1.5 V; **ADC limit exceeded** identifies magnitudes above
1.25 V, the demo's alarm limit. Power is voltage × current, and energy uses seconds
internally before conversion to nJ. The moving average uses a 20 ns window and a
476 ps output step.
The clock-sampled mean starts at the first rising edge, excluding its uncaptured
prefix; the event-driven mean covers the whole recording.

## Statistics provenance and interpretation

[`support/derived_stats.json`](support/derived_stats.json) preserves the five
source paths, descriptions, units, recording identity, and all 64 sample rows.
The source is `workspace/output/runs/hip-matmul-lds-gem5-kmd-vtr/trace.vtr`,
recording `4e33376f6f28af6f3a496a958a99c7f1`. The excerpt covers shader cycles
79,400–85,800, originally 37.7944–40.8408 µs. Only the time origin changes.
Every retained source window is complete and periodic, has duration 100 shader
clocks, and belongs to one statistics-reset generation.

The counters aggregate all eight source compute units. Values are already
interval counts: **do not subtract consecutive samples**. They are copied
exactly into 32-bit buses after checking that each source real is a finite,
nonnegative integer. The zero baseline has no measured interval and is not
included in captured sample totals. Before the first 47.6 ns sampling boundary,
captured waveforms have no value. Ratios with a zero denominator remain unknown.

| Derived quantity | Interpretation and expected result |
| --- | --- |
| LDS traffic [GB/s] | `(read bytes + write bytes) / 47.6`, because every window is 47.6 ns; decimal GB/s |
| LDS traffic [bytes/clock] | `(read bytes + write bytes) / 100` |
| Extra bank cycles/request | Conflict penalty divided by processed requests in that window; not a stall percentage |
| Bank words/request | Bank accesses divided by processed packets; this is distinct from logical payload bytes/request |
| Bank conflict present | True when the interval contains additional conflict cycles |
| Total LDS bytes | **299,344 bytes**: sum each interval once using `@sample` |
| Total LDS requests | **2,206 requests**, including repeated adjacent counter values |
| Weighted extra cycles/request | **2,640 / 2,206 ≈ 1.196736**; a ratio of totals, not an unweighted mean of interval ratios |
| Mean LDS traffic [bytes/clock] | **299,344 / 6,400 = 46.7725** |

Bank bytes count full words for subword accesses. Atomic reads and writes
contribute to both byte counters but count once in bank accesses, so
`read bytes + write bytes` need not equal `4 * bank accesses`. The bandwidth
describes modeled LDS service traffic, not DRAM bandwidth or application payload
throughput. The table is a saved result snapshot; editing a formula does not
update an already exported table.

To reproduce the excerpt from the original large recording, use its absolute
path in place of `SOURCE.vtr`:

```sh
cargo run --locked -p volna --example derived -- \
  --extract-stats SOURCE.vtr volna/examples/support/derived_stats.json 79400
cargo run --locked -p volna --example derived
```

Extraction validates the original shader clock, sampling-window metadata,
counter types, and reset generation. A different source recording or starting
cycle changes the example's expected totals and requires reviewing its tests
and documentation. Normal regeneration needs only the bundled JSON. VTR assigns
a new recording identity on each run; the signal data and workspace are
reproducible.
