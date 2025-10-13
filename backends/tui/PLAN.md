# Terminal Backend Plan

This document tracks the milestone-level plan for the terminal (TUI) backend.
Keep it concise so we can adjust status quickly as pieces land.

## Milestones

1. **Project Scaffolding** ✅
   - Finalise crate layout mirroring the web backend structure. ✅
   - Introduce placeholder terminal + renderer wiring guarded behind features. ✅
   - Document build instructions and contribution expectations in the README. ✅

2. **Terminal Driver Integration** ✅
   - Evaluate terminal crates (`crossterm`, `ratatui`, `tui-rs`) and pick a
     baseline abstraction. ✅ (`crossterm` selected)
   - Implement terminal initialisation, resize handling, and teardown hooks. ✅
   - Surface minimal logging so issues are debuggable without a UI. ➖ (basic error propagation only)

3. **Renderer Pipeline** 🔄
   - Translate `AnyView` nodes into terminal widgets using `waterui-render-utils`. ✅ (bespoke walker implemented)
   - Add diffing logic to minimise screen updates and support animations. ⏳
   - Implement input routing back into the reactive runtime. ⏳

4. **Component Coverage** 🔄
   - Provide renderers for the core component set (text, buttons, layout, forms). ⏳ (text/layout/navigation ready)
   - Wire focus management and navigation hints for keyboard-driven UIs. ⏳
   - Ensure colour/attribute fallbacks for monochrome terminals. 🔁 (basic colour mapping landed)

5. **Testing & Tooling**
   - Create snapshot-style integration tests for complex layouts.
   - Automate smoke tests in CI using headless terminal emulation.
   - Publish developer ergonomics docs (logging, debugging, profiling).

## Open Questions

- Which renderer crates offer the best balance between control and maintainability?
- How should we expose theme support and colour palettes without leaking backend
  specifics into `waterui-core`?
- Do we need a dedicated async runtime for input handling, or can we reuse the
  existing executor abstractions?

> Update this plan as progress is made so downstream consumers know the current
> state of the terminal backend.
