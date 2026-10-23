# VTR and Volna migration acceptance

Updated: 2026-10-23. The migration implementation and acceptance checks are
complete. This record accompanies the finalized SDK pin and Volna migration.

## Goal and acceptance criteria

Complete the VTR v3.1 migration first, then finish Volna. Backward compatibility is not required.

1. Audit every VTR component. Remove obsolete V2 code, APIs, assets, documentation, and compatibility paths.
2. Bring the Rust API into parity with the C++ reference in functionality, semantics, robustness, and usability.
3. Update or regenerate every VTR example and fixture. Update the Verilator integration, commit any required integration changes, and pin the correct revision.
4. Consolidate documentation into accurate, consistent, nonredundant explanations of the current rationale, design, implementation, and usage.
5. Remove dead code, unnecessary abstractions, duplication, and redundant tests. Follow DRY and KISS; retain meaningful behavioral, edge-case, and invariant coverage.
6. Run all required builds, tests, interoperability checks, benchmarks, and integration checks. Investigate failures and unintended regressions. Finish VTR in **one migration commit** by amending the existing migration commit.
7. Once VTR is complete and committed, repin it in Volna, including its finalized Verilator integration.
8. Migrate every Volna crate, example, and fixture to the current APIs and format. Remove V2/V2.1 remnants and compatibility machinery; apply the same quality standards.
9. Prove that all required tests, examples, integrations, and end-to-end workflows work. Perform a final audit of actual code, documentation, assets, dependency pins, and verification results.
10. Commit the completed Volna migration. No push has been requested for this migration work.

The reference family is called v3.1, but its actual versions are **WIRE 3.2, VTI 3.0, and C++ package 4.0**. Preserve those verified versions rather than renaming constants to 3.1.

## Delivery state

- VTR is `3368e526f7503028887dadf03a9dd9aec6953e9e`, the single migration
  commit after `a0fa76252fc454ba7420009bf93c0a0d80e1eb6b`.
- The Volna migration includes that finalized VTR gitlink, all SDK consumer
  changes, regenerated assets, maintained example sources, and verification.
- Verilator is pinned to `6eb99ee3ac238b0373707660aa7708122741933f`, with
  nested slang `7ddf4059f79eff508dd486eb42fd650cdf320d52`. Both source worktrees
  are clean. The actual pinned simulator was rebuilt and tested.
- No push was requested. Temporary artifacts under `atemp` are excluded from
  both migration commits.

## Verified SDK behavior

Exact `Reader::time_range()` in Rust and C++ includes waveform progress, ordinary
owners, clocks, located records, and embedded detail outside owner lifetimes.
It reads selected time columns without projecting payloads or filling owner
caches. Full transaction decoders retain their original fast paths. Tests cover
empty/relation-only traces, zero, all codecs and execution modes, full-u64 and
open stages, malformed columns, and payload admission limits.

Timed SystemVerilog child creation uses `vtr_item_open_child_at` and fixes parent
identity at creation while retaining the historical first-stage timestamp. Both
SDK guides and the DPI documentation describe the current contracts.

Current acceptance evidence under `atemp/vtr-migration`:

- `verify-restored-final.txt`: Rust workspace tests, fmt, strict all-target and
  all-feature Clippy, strict docs, package verification, fresh extracted package
  tests on Rust 1.97.1, four SDK examples, release CLI/WASM, bindgen, and actual
  Node/native conversion fidelity all pass.
- `native-restored-ctest.txt`: all 36 Release CTests pass.
- `sanitize-restored-unrestricted-ctest.txt`: all 36 ASan/UBSan/LSan tests pass
  with leak detection enabled. The sandbox run failed because LSan cannot run
  under its tracing; no sanitizer was disabled.
- `verify-restored-native.txt`: Release/Debug install consumers, repeated
  installation, actual pinned Verilator build, VTR/FST smoke, all 14 regenerated
  structured-log fixtures, eight full pipeline configurations, and all 15
  checked-in SDK VTR assets pass. Fresh logs match the checked-in semantics.
- `machete-restored.txt` and `deny-restored.txt`: dependency checks pass.

Both full default benchmark runs pass on CPU 0 with three repetitions and scale
1. All 490 cells were compared against the committed report, with no missing
cells; all 164 size/count metrics are identical. The tracked report is the
second complete run. Two seven-pair interleaved comparisons use the exact
committed baseline binary SHA `698092e5aa64e29b6c6c1c2c6808f3541b29f51b8335cb7bdf3397a7de91b12a`
and candidate `39413fa5dcd3a4a93b00492fc83375b5075b23f5ba4f2a7fb327c40a3d5ec677`.
All 180 paired metrics (186 values) were inspected in each run. The earlier
many-generator scan slowdown is resolved. Requests find p95 changes from +8.0%
in the first pair run to +0.7% in the repeat; repeat full write/read deltas are
about -3.0%..+0.5%. The only remaining relative close outlier is 0.11 to 0.12 ms.
Raw data, complete reports, and all slower measurements are retained in
`restored-paired*` and `committed-report-restored*.json`.

## Delivered Volna examples and behavior

All 20 tracked Volna VTR assets are 3.2 and fully validate with no unresolved
references (`volna-final-assets.json`). The old pipeline and theme assets have
been replaced with full regenerated recordings.

- `volna/examples/verilator_pipeline` contains the real five-file bound
  simulator example, its verifier, and usage instructions. Its eight-way matrix
  preserves all 14 owners, stages, attributes, statuses, events, parents,
  relations, clocks, the expected stale-key diagnostic, and waveforms identical
  to an uninstrumented baseline. `pipeline_demo.vtr` is generated only after
  that entire matrix passes. Viewer tests use semantic generator paths and
  timescale-aware tick positions.
- `volna/examples/volna_theme.rs` preserves the original 4,096-cycle simulation:
  both cores, C910's 13 stages, DMA, five log severities, weak/unknown signals,
  analog spikes, gated clocks, deep hierarchy, flushes, open rows, and relations.
  Original and current recordings both have 4,916 owners, 5,222 relations,
  seven logs, 297 variables and 293 signals. Every signal history and canonical
  owner/stage/event/parent/relation/log observation was compared. Four stages
  retain explicit open ends instead of the old reader's close-time bound;
  their effective intervals agree. Four additional first samples equal the
  original implicit values at their timestamps. The final generator output
  matches the completely compared semantic snapshot.
- The prior full GPU, pipeline, landing, feature, and analog ports remain intact.
  All six Rust generators execute and validate their recordings in
  `volna-final-examples.txt`; the GPU still records 80,000 instructions.
- Embedded stages outside owner lifetimes fit, paint, hover, and select at
  their recorded times. The original owner interval remains unchanged.
- VSIX packaging now builds the server and converter from their respective
  workspace manifests; selecting `vtr-cli` from the root workspace failed in
  the real package build. No compatibility path was added.
- Browser sample readiness is armed only after files are fetched and the new
  open request is enqueued. A deterministic delayed-fetch regression first
  reproduced an old workspace update prematurely completing sample loading.

## Volna acceptance evidence

- `volna-workspace-assets-final-unrestricted-tests.txt`: 797 passed, zero failed,
  five manually ignored scale tests across 54 suites. The initial sandbox run
  failed only on denied local sockets; the full unrestricted run passes.
- `volna-assets-final-fmt.txt` and `volna-assets-final-clippy-rebuild.txt`: fmt
  and strict workspace/all-target/all-feature Clippy pass. The converter was
  rebuilt after stale compiler metadata caused E0460; no source workaround.
- `volna-scale-assets-final-lib-tests.txt`: all four full release scale tests
  pass, including 100 million sorted/verified ordinals and 10 million resident
  transactions. The 100M index uses exactly 400,000,000 reserved bytes.
- `volna-scale-assets-final-pipeline-test.txt`: full 500,000-row/14-stage release
  test passes. Exact open is 43 ms, load 2.247 s, first frame 3 ms, RSS about
  1,112 MiB. Whole-trace/5,000-cycle/150-cycle painting is 3.26/0.37/0.34 ms.
  Exact range correctness deliberately costs more than the old incomplete 3 ms
  open; no workload reduction was used.
- Documentation build and check pass, with 663 local links/assets across 17
  generated pages. Browser acceptance covers 60 route/theme/viewport cases,
  30 accessibility audits, imports, worker cancellation/replacement, complete
  waveform and pipeline loading, and one-million-row sorting/filtering.
- Extension unit tests pass (27), package verifier tests pass (five), native
  packaging and bundled converter import pass, and installed VSIX tests pass
  for VTR/FST/Kanata/LOG-gzip with bundled tools. The final rebuilt packages pass
  after three WASM-only discarded-value warning fixes and the sample-readiness
  fix. The pinned nightly's shared-memory atomics feature warning remains expected.

## Final browser and package verification

- `docs-sample-race-before.txt`: the deterministic delayed-fetch regression
  fails on the previous host logic.
- `docs-sample-race-fixed-{build,check,tests}.txt`: final docs build/check,
  663-link audit, all browser cases, imports, and full 1M worker workloads pass.
- `vscode-assets-final-clean-{web-build,package,installed-tests}.txt`: final WASM
  rebuild, verified VSIX contents, and installed extension end-to-end pass.

Temporary originals, scripts, raw measurements, comparison snapshots, manifests,
and logs remain in `atemp/vtr-migration` and are excluded from the committed source tree.
