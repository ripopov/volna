# Conversion progress

`vtr-progress` defines work counters and versioned conversion progress messages.
It has no trace readers or process execution. Converters update counters; CLI and
native hosts render snapshots. Totals apply to the current phase, not the whole
conversion. Unknown totals remain indeterminate.

`vtr convert --progress=json` emits newline-delimited `Snapshot` objects on stdout.
Each object carries `version` (currently 1), `phase`, `completed`, optional `total`,
`input_bytes`, and `output_bytes`. Sizes are bytes on disk; output size measures
the private output file before publication and the destination after success.
`complete` is emitted only after publication succeeds. Diagnostics go to stderr.

```json
{"version":1,"phase":"writing","completed":42,"total":100,"input_bytes":268435456,"output_bytes":40579891}
```

Readers reject unsupported versions, invalid counters, incomplete lines, and
records longer than 4096 bytes. Update `VERSION` when changing this contract.
