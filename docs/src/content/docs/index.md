---
title: Volna engineering documentation
description: Trace storage, file format, and engineering guides for Volna.
template: splash
hero:
  title: Volna engineering docs
  tagline: Hardware debug starts with a trace. Understand the runtime data, write a recording, and build tools around it.
  actions:
    - text: Start with VTR
      link: ./vtr/getting-started/
      icon: right-arrow
      variant: primary
    - text: Read the format
      link: ./vtr/specification/
      variant: secondary
---

<div class="v-status"><span class="v-status__label">PROJECT STATUS</span> Under development · VTR format is experimental</div>

## The trace layer

Volna is a hardware debug environment under development. This workspace currently
contains **VTR (Volna Trace Record)**, a Rust library for writing and querying
hardware simulation traces.

A recording carries signal values, transactions, runtime hierarchy, relations,
and structured logs on a shared time base. Design semantics and presentation
belong to the separate VDB and application layers.

<div class="v-guide-grid">
<a class="v-guide" href="./vtr/"><span class="v-guide__label">01 / START HERE</span><strong>VTR trace library</strong><span>Scope, data model, and the boundary between a trace and a viewer.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="./vtr/specification/"><span class="v-guide__label">02 / REFERENCE</span><strong>File format</strong><span>Container layout, binary encodings, recovery, and conformance.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="./vtr/rationale/"><span class="v-guide__label">03 / DECISIONS</span><strong>Design rationale</strong><span>Why sections, local compression, stable identity, and derived indexes.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="./vtr/logging/"><span class="v-guide__label">04 / GUIDE</span><strong>Structured logging</strong><span>Typed call sites and timestamped records alongside transactions.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
</div>

## Work with the library

Building the workspace requires Rust 1.85 or newer and a C compiler to build
the Zstandard dependency.

The [getting started guide](./vtr/getting-started/) includes a complete waveform
round trip and instructions for browsing the Rust API documentation.

## Maintain the documentation

Pages are Markdown, with SVG figures, Mermaid diagrams, and small HTML/JavaScript
widgets when interaction helps explain a concept. The [authoring guide](./authoring/)
covers the build and content conventions. The [design foundations](./design/)
are shared by the documentation and future Volna application UI.
