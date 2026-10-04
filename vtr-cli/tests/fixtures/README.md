# CLI fixtures

`pipeline.kanata` and `waves.vcd` are hand-authored regression inputs, licensed
under the repository's MIT license. They cover overlapping instructions in two
threads, stages, late labels, a wakeup dependency, flush/open status, aliases,
four-state values, reals, repeated same-time events, and a 10 ns time base. Tests generate malformed
and gzip inputs in temporary directories.

FST conversion and activity tests reuse the committed
[trace-loader fixtures](../../../volna-trace/tests/fixtures/README.md), including
their documented provenance. No fixture regeneration is required to run tests.
