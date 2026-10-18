# Volna traces

A trace records a run's hierarchy, signal values, and related occurrences.

## Language

**Signal**:
A value on a wire or in a register, represented by its history of value changes.

**Record**:
An individual runtime occurrence produced by a generator. An instantaneous record has a single timestamp; identical timestamps and payloads do not merge distinct occurrences.

**Scope**:
A named group in the trace hierarchy, such as a module or an optional grouping of record sources. Scopes can contain other scopes, signals, and generators.

**Physical hierarchy**:
The tree of scopes describing design structure and the locations of signals and record sources.

**Logical hierarchy**:
Runtime ownership among transactions and records. Dependency and causal relations can connect activity across this hierarchy.

**Generator**:
A named source of records. Its kind identifies the meaning of its records, and its attributes describe the source.
_Avoid_: Stream

**Transaction**:
A record representing a logical unit of computation in the design, with an identity and a lifetime. A transaction can own other transactions and records that describe its progress.

**Clock**:
A generator describing the rising edges used to count cycles. Other generators can reference a clock to express their activity in cycles.
