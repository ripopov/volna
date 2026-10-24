# Bound pipeline instrumentation

`core.sv` drives a small out-of-order core's handshake signals. `bind.sv`
attaches `tracer.sv` without changing the design. The tracer follows fetch
sequence numbers, ROB entries, and physical registers through stalls, paired
dispatch, replay, branch flushes, key reuse, retirement, and a final open
instruction. Bus requests retain their issuing instruction as parent.

Build the [pinned VTR SDK and Verilator](../../../vtr/README.md#development),
including the release CLI, then run from the Volna workspace root:

```sh
python3 volna/examples/verilator_pipeline/run.py
python3 volna/examples/verilator_pipeline/run.py --update-example
```

The verifier covers one and two simulator threads, inline and threaded SDK
execution, and recordings with and without waveforms. It checks all 14 owners,
their stages, attributes, statuses, events, parent identities, outgoing
relations, declared clock timing, and the expected stale-key diagnostic.
Waveforms must match an uninstrumented simulation. Only a fully passing matrix
can replace `volna/examples/pipeline_demo.vtr` with `--update-example`.

Artifacts default to `atemp/verilator-pipeline`; use `--out` to choose another
directory. The trace uses picosecond ticks. The verifier compares physical
nanoseconds, and Volna displays pipeline positions in the declared 10 ns clock.
