# Volna traces

A trace records a run's hierarchy, signal values, and related occurrences.

## Language

**Scope**:
A named group in the trace hierarchy, such as a module or an optional grouping of record sources. Scopes can contain other scopes, signals, and generators.

**Generator**:
A named source of transaction or log records. Its kind identifies the meaning of its records, and its attributes describe the source.
_Avoid_: Stream

**Transaction**:
An individually identified occurrence produced by a generator, with a begin time, end time, and attributes. An instantaneous occurrence has equal begin and end times.

**Clock**:
A generator describing the rising edges used to count cycles. Other generators can reference a clock to express their activity in cycles.
