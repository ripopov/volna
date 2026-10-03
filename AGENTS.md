# Comments and documentation

Write comments and documentation for maintainers who have never seen the prompts or conversation. Preserve useful technical rationale, constraints, and invariants in self-contained terms; omit prompt references, acknowledgments, and change narration. Before finishing, review added or edited comments and documentation for reliance on conversation context.

A useful review test is: If the conversation disappeared, would this comment still help someone understand or safely change the code?

# Staging and commits

Stage completed changes. Commit only when explicitly requested by the user.
Include only task-related changes; preserve unrelated work and its staging state.

# Shared Volna design

For documentation and application UI changes, read and apply the
[Volna design skill](docs/design-system/SKILL.md). The shared tokens and assets
live in `docs/design-system`; engineering content lives in Markdown and builds
through Astro/Starlight. The current VTR guides in `vtr/docs` are authoritative.
