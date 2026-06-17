# WaterUI Runtime Semantics

Use this reference when a WaterUI task touches renderer-local state, `GpuSurface`, or visual testing.

## Fine-Grained Reactivity

- WaterUI uses fine-grained reactivity with reconstruction semantics.
- If parent-driven control flow rebuilds or recreates a component instance, that instance's local state is expected to reset.
- Do not "fix" rebuild-driven resets by caching or restoring component-local state across rebuilds.
- If behavior must survive rebuilds, the source of truth must live in explicit reactive state owned at the correct level, not in hidden renderer caches.

## GpuSurface

- `GpuSurface::new(renderer)` owns a single `GpuView` instance for the lifetime of that surface.
- `GpuView::setup()` is the place to create persistent GPU resources for that renderer instance.
- `GpuContext::redraw_handle` exists so async work or external events can request another frame for the same surface instance.
- If the `GpuSurface` is torn down because the view was rebuilt, the `GpuView` instance is gone. That is not a bug to work around with out-of-band renderer resurrection.
- Do not move GPU renderer state into shared runtime slots just to survive `GpuSurface` drop or parent rebuild.

## Visual Testing

- In this repository, "visual test" means the agent reads the generated image directly with its own vision capability.
- Heuristic image checks are forbidden, including pixel counters, threshold diffs, bbox approximations, dominant-color checks, brightness checks, and similar proxy code.
- `waterui-testing` should primarily validate interaction logic and Hydrolysis accessibility output. Visual snapshots are still valuable, but the agent must inspect the actual PNGs directly when visual correctness matters.
- Prefer `#[waterui::test(view_fn)]` when a test only needs the default `UiTest::new().mount(view_fn)` setup. Use explicit `UiTest` construction only when a custom viewport or environment is genuinely required.
- For layout verification under `waterui-testing`, prefer semantic children and assert bounds relationships from the Hydrolysis tree instead of attaching synthetic accessibility metadata to purely decorative color fills.

## Component Body Shape

- Avoid introducing an unnecessary `Dynamic` around otherwise static components just because the body has a simple branch.
- `waterui_chart::Tooltip` is a concrete regression that mounted cleanly only after replacing a `#[view_builder]` body branch with explicit `AnyView` branching.
- If a component does not need reactive reconstruction semantics inside its body, keep the body shape simple and concrete.
