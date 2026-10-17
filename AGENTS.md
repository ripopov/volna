# Comments and documentation

Express ideas directly through clear names, types, control flow, and meaningful
tests. Improve confusing code before adding a comment to explain it. Avoid
comments that restate the implementation. Keep module descriptions brief and at
the top of the module.

Use brief comments when code cannot convey the reason for a constraint or
decision: external requirements, non-obvious tradeoffs or rejected alternatives,
and subtle safety invariants that a maintainer might otherwise remove. Explain
why the constraint or decision matters, rather than narrating how the code works.

Write comments and documentation for maintainers who have never seen the prompts
or conversation. Keep rationale self-contained; omit prompt references,
acknowledgments, and change narration. Before finishing, review added or edited
comments and documentation: would they still help someone understand or safely
change the code if the conversation disappeared?

# Testing

Do not add trivial unit tests that merely mirror static declarations or
configuration. Focus tests on meaningful behavior, edge cases, and nontrivial invariants.

# Staging and commits

Stage completed changes. Commit only when explicitly requested by the user.
Include only task-related changes; preserve unrelated work and its staging state.

# Temporary plans

`./atemp` is a temporary directory for plans. Never commit this directory or its contents.

# Shared Volna design

For application UI work or when creating or editing HTML, read and apply the
[Volna design skill](docs/design-system/SKILL.md). Do not apply it to Markdown-only
documentation edits, Rust data model work, or other non-UI code. The shared tokens
and assets live in `docs/design-system`; engineering content lives in Markdown
and builds through Astro/Starlight. The current VTR guides in `vtr/vtr/docs` are
authoritative.

# UI design workflow

For visual design discussions, inspect the existing implementation and offer an
interactive mockup before changing application code. Include the controls and
states needed to evaluate the design, such as active sort direction, inherited
settings, and explicit overrides. Provide a clickable standalone HTML preview
alongside any inline visualization. Keep drafts and review screenshots in `atemp`.

Once a direction is accepted, implement it using the existing behavior and shared
design system. Keep drawing and interaction bounds aligned. Verify the running
application in both themes, at narrow widths and different zoom levels, including
keyboard use. Inspect application screenshots and test meaningful interactions.

# Compatibility

Volna is a research project. Do not add backwards-compatibility machinery
for old assets or interfaces. When a
protocol, serialization format, or other API layer changes, bump its version
as needed and ignore or regenerate assets produced by older versions.
