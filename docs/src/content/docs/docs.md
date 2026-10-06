---
title: Volna engineering documentation
description: Trace storage, local and remote loading, viewer architecture, and engineering guides for Volna.
template: splash
hero:
  title: Volna engineering docs
  tagline: Hardware debug starts with a trace. Understand the runtime data, write a recording, and inspect it in the viewer.
  actions:
    - text: Start with VTR
      link: ../vtr/
      icon: right-arrow
      variant: primary
    - text: Read the format
      link: ../vtr/specification/
      variant: secondary
---

<div class="v-status"><span class="v-status__label">PROJECT STATUS</span> Under development · VTR format is experimental</div>

## The workspace

Volna is a hardware debug environment under development. The workspace includes
**VTR (Versatile Trace Record)** for writing and querying
hardware simulation traces, **vtr-capi** for C and simulator integrations,
**vtr-guard** for Linux crash handling, **volna-trace** for immutable VTR/FST
access, and **volna-server** for serving a recording to a remote client.
**volna-core** owns viewer state and rendering models; **volna** is the GPUI
desktop frontend.

A recording carries signal values, transactions, runtime hierarchy, relations,
and structured logs on a shared time base. Design semantics and presentation
belong to the separate VDB and application layers.

<div class="v-guide-grid">
<a class="v-guide" href="../vtr/"><span class="v-guide__label">01 / START HERE</span><strong>VTR trace library</strong><span>Data model, boundaries, and a first trace in Rust.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="../vtr/specification/"><span class="v-guide__label">02 / REFERENCE</span><strong>File format</strong><span>Container layout, binary encodings, recovery, and conformance.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="../vtr/rationale/"><span class="v-guide__label">03 / DECISIONS</span><strong>Design rationale</strong><span>Why sections, local compression, stable identity, and derived indexes.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="../vtr/logging/"><span class="v-guide__label">04 / GUIDE</span><strong>Structured logging</strong><span>Typed call sites and timestamped records alongside transactions.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="../vtr-capi/"><span class="v-guide__label">05 / C API</span><strong>Build with C</strong><span>Link libvtr, manage handles, and write a trace from C.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="../vtr-guard/"><span class="v-guide__label">06 / RECOVERY</span><strong>Crash guard</strong><span>Finish watched writers after Linux faults, stop requests, and exit.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="../volna-trace/"><span class="v-guide__label">07 / LOADING</span><strong>Local and remote traces</strong><span>Open VTR/FST recordings, load complete objects, and manage memory.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="../volna-server/"><span class="v-guide__label">08 / HOSTING</span><strong>Headless server</strong><span>Process hosting, framed protocol, and activity sidecar transfers.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="../volna-core/"><span class="v-guide__label">09 / VIEWER CORE</span><strong>Viewer models</strong><span>Commands, panel state, display lists, persistence, and undo.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
<a class="v-guide" href="../volna/"><span class="v-guide__label">10 / DESKTOP</span><strong>Run the viewer</strong><span>Build the GPUI frontend and open VTR or FST recordings.</span><span class="v-guide__arrow" aria-hidden="true">→</span></a>
</div>

## Work with the libraries

Building the workspace requires Rust 1.97.1 or newer and a C compiler to build
the Zstandard dependency.

The [VTR guide](../vtr/) includes a complete waveform
round trip and instructions for browsing the Rust API documentation.
The [C API guide](../vtr-capi/) covers building, linking, ownership, and errors;
the [crash guard guide](../vtr-guard/) covers Linux process recovery.
The [trace loading guide](../volna-trace/) explains the shared session API.
The [server guide](../volna-server/) covers building and hosting the executable.
The [viewer core guide](../volna-core/) explains the frontend contract; the
[desktop guide](../volna/) covers building and running the application.

## Maintain the documentation

Pages are Markdown, with SVG figures, Mermaid diagrams, and small HTML/JavaScript
widgets when interaction helps explain a concept. The [authoring guide](../authoring/)
covers the build and content conventions. The [design foundations](../design/)
are shared by the documentation and Volna application UI.
