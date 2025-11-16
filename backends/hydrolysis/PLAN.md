# Hydrolysis Rendering Plan

This document captures the research performed across the workspace and the plan for Hydrolysis
(located in `backends/hydrolysis/`), our single self-drawn renderer that all native backends rely on.
Hydrolysis drives both the
Vello (GPU) and tiny-skia (CPU) implementations (and future targets) while exposing one consistent
rendering architecture.

---

## 1. Research Snapshot

- `waterui_core::View`, `AnyView`, `ConfigurableView`, and `Native<T>` (see `core/src/view.rs`)
  define how declarative views eventually turn into renderer-owned objects. Raw views such as
  `FixedContainer`/`ScrollView` rely on renderers calling layout trait objects (`components/layout`).
- Reactive values come from `nami` (`Computed<T>`, `Binding<T>`). The `views` module
  (`core/src/views.rs`) exposes `Views`, `AnyViews`, and watchers for dynamic collections.
- Existing renderers (TUI in `backends/tui/src/renderer.rs` and the WIP web renderer) already use
  a `ViewDispatcher`, reactive watchers, and layout trait objects, but those utilities live inside
  each backend today. We will move all generic logic into Hydrolysis.
- Layout primitives (`components/layout/src/core.rs` + `container.rs` and `scroll.rs`) describe a
  clear three-pass protocol (propose → size → place) that the renderer must drive.

These findings anchor the design below.

---

## 2. Goals for Hydrolysis

1. Provide a reusable pipeline that converts `AnyView` into an internal render tree.
2. Own all renderer-facing utilities (dispatcher, tree diffing, layout runners, asset caches,
   event routing) so future backends do not duplicate logic.
3. Expose two concrete backends under one crate:
   - `gpu::vello` – wgpu + vello scene rendering.
   - `cpu::skia` – tiny-skia pixmap renderer for headless/testing.
4. Support the core components demanded by the product team: `Text`, `Image/Photo`, `Container`,
   `FixedContainer`, `ScrollView`, `TextField`, `Toggle`, `ProgressBar`, plus an extensible path
   for additional controls.
5. Keep render state reactive by wiring `Computed<T>`/`Binding<T>` watchers to tree nodes, so only
   the parts that change are re-drawn.
6. Require every render-tree node to implement a `RenderNode` trait with `layout(&mut LayoutCtx)`
   and `paint(&mut RenderCtx)` methods plus reactive hooks, so layout/render logic stays uniform
   no matter which backend is active.
7. Keep Hydrolysis modular: subsystems like text measurement, glyph shaping, image decoding,
   and widget logic must be backend-agnostic. Renderer-specific crates (Vello, tiny-skia,
   future embedded targets) plug into small traits instead of Hydrolysis depending on them.
8. Prioritise the GPU/CPU engines (`vello`, `tiny-skia`) up front. The TUI backend will adopt
   Hydrolysis later by implementing the same `RenderBackend` trait.

---

## 3. Proposed Crate Layout

```
backends/hydrolysis/
├── src/lib.rs                # crate façade + feature flags
├── src/dispatcher.rs         # current ViewDispatcher + future helper traits
├── src/tree/
│   ├── mod.rs                # RenderTree + arena + RenderNode trait
│   ├── parser.rs             # AnyView → RenderNode builder
│   ├── diff.rs               # structural diffing + change sets
│   └── reactive.rs           # watchers attached to nodes
├── src/layout/
│   ├── engine.rs             # drives Layout trait objects (propose/size/place)
│   ├── geometry.rs           # Rect/Size wrappers + conversions
│   └── flex.rs?              # helper algorithms if needed
├── src/assets/
│   ├── fonts.rs              # font loader, typography cache
│   ├── images.rs             # async image loader/cache (uses waterui_media::image)
│   └── shaders.rs            # GPU pipeline descriptors
├── src/text/
│   ├── mod.rs                # backend-agnostic text measurement API
│   └── shaping.rs            # glyph layout shared by CPU/GPU implementations
├── src/backend/
│   ├── mod.rs                # RenderBackend trait, FrameResult, feature gating
│   ├── gpu.rs                # Vello implementation (`gpu` feature)
│   └── cpu.rs                # tiny-skia implementation (`cpu` feature)
├── src/components/
│   ├── primitives.rs         # Text, Image, Shape, etc nodes
│   ├── controls.rs           # Toggle, ProgressBar, TextField
│   ├── layout.rs             # Container, FixedContainer, ScrollView nodes
│   └── widgets.rs            # shared code for interactive widgets
└── src/runtime/
    ├── scheduler.rs          # frame timer + dirty tree queue
    ├── input.rs              # focus + event dispatcher
    └── scene.rs              # immediate drawing traversal
```

`backends/hydrolysis/src/lib.rs` will become the façade that re-exports the dispatcher, tree builder,
backend traits, text subsystem traits, and helper contexts so platform crates only depend on this crate.

---

## 4. RenderNode Trait & Internal Tree

1. **Trait contract:** `pub trait RenderNode { fn layout(&mut self, LayoutCtx<'_>) -> LayoutResult; fn paint(&mut self, RenderCtx<'_>); fn update_reactive(&mut self); fn handle_event(&mut self, &EventCtx<'_>, Event) -> EventResult; }`
   - `LayoutCtx` exposes measurement APIs (text metrics, font ascender, image dimensions) implemented via backend-neutral traits.
   - `RenderCtx` records draw commands in a backend-agnostic scene stream. CPU (tiny-skia) and GPU (Vello) backends replay that stream to their surfaces to ensure identical visuals.
   - `EventCtx` gives nodes hit-testing helpers and access to `Binding` mutation APIs so they can respond to pointer/keyboard input immediately (no later “TUI pass”).
2. **Parsing:** Use the dispatcher to peel `AnyView` until we reach concrete config structs like `Native<TextConfig>` or raw views (`FixedContainer`, `ScrollView`). Each handler builds a `RenderNode`, stores references to reactive fields, registers watchers for `Computed<T>`/`Binding<T>`, and wires event callbacks.
3. **Node Types (initial set):**
   - `TextNode { content: Computed<StyledStr>, style: TextStyle, layout_rect, glyph_buffer }`
   - `ImageNode { source: Url, placeholder_node: Option<NodeId>, surface: ImageHandle }`
   - `ContainerNode` / `FixedContainerNode` wrapping `Box<dyn Layout>` and child nodes
   - `ScrollNode` tracking axis, scrollbar info, and scroll offsets
   - `ControlNode`s for `Slider`, `Stepper`, `Toggle`, `ProgressBar`, `TextField`, `Divider`, `Spacer`
4. **Reactive Wiring:** Every reactive input uses a `NodeSignal<T>` wrapper:
   - Holds the latest value, registers a `nami::watcher`, and enqueues `DirtyNode` events with `DirtyReason::Reactive`.
   - Control nodes update bindings via `Binding<T>::set` immediately on input events (TODO: when event pipeline lands, document potential backpressure).
5. **Diff/Retention:** For `Container`/`AnyViews`, use the `Views` watcher to map view IDs to node IDs; reuse existing nodes when only data changes. Keep a `NodeId ↔ ViewId` map to avoid reallocations.
6. **Render Tree Output:** `RenderTree` exposes `root_mut`, `node_mut`, `mark_dirty`, and `drain_dirty`. Nodes maintain child IDs + cached geometry so layout and repaint passes traverse the same structure.

This tree feeds both backends identically.

---

## 5. Layout, Scene Traversal & Interaction

1. **Layout Engine:** Walk `RenderTree` depth-first:
   - For each `LayoutNode`, call `layout.propose`/`size`/`place` to compute child `Rect`s.
   - Cache `ProposalSize`, `Size`, `Rect`, and `ChildMetadata` so nodes only re-layout when dirty.
2. **Scene Graph:** After layout, build a `SceneGraph` (flat draw command stream) capturing:
   - Paint commands (fill/stroke text, shapes, gradients, images)
   - Clip stacks for `ScrollView`
   - Input hit regions (rectangles + node IDs) used by the `EventDispatcher`.
3. **Input Pipeline:** `runtime::input` maintains:
   - Focus manager
   - Pointer/keyboard routing using the hit regions recorded during scene traversal
   - Event bubbling/capture so nodes can intercept or forward events
4. **Control Interactions:** Nodes implement simple state machines:
   - `Slider`: drag updates `Binding<f64>`; discrete steps if configured.
   - `Toggle`: pointer tap toggles `Binding<bool>`.
   - `TextField`: keyboard events mutate `Binding<Str>`, caret movement, selection, IME (TODO: IME integration).
   - `ProgressBar`: read-only visuals driven by `Computed<f64>`.
   - `Stepper`: plus/minus buttons backed by `Binding<i32>` (TODO: hold-to-repeat).
5. **TODO markers:** If any control lacks full interaction, annotate the node + plan with `TODO(interaction-name)` to keep the debt visible.

---

## 6. Frame Scheduler & Input

1. `runtime::scheduler::FrameClock` ticks every vsync (GPU) or timer (CPU). When the tree has any
   dirty nodes, schedule a redraw.
2. `runtime::input::EventDispatcher` records focus, hit-testing rectangles, and routes pointer/
   keyboard events back into control nodes. Control nodes update their `Binding`s, which feed the
   reactive pipeline.
3. Provide hooks so window backends can forward platform events into this dispatcher.

---

## 7. Backend Implementations

### 7.1 GPU (Vello/wgpu)

- **Dependencies:** `wgpu`, `vello`, `vello_text`, `vello_encoding`, `wgpu-hal`.
- **Context:** `gpu::Context` owns `wgpu::Instance`, `Adapter`, `Device`, `Queue`, and swapchain
  (`Surface`) handles taken from the `window` crate. Build `RenderSurface` objects per window.
- **Scene Building:** Convert `SceneGraph` into Vello `Scene` via `SceneBuilder`.
  - Text: use `vello_glyph` with cached font atlas.
  - Image: upload textures as `wgpu::Texture`s; keep `ImageCache` keyed by `Url`.
  - Shapes (progress track, toggles): render with Vello path operations.
- **Rendering:** Each frame:
  1. Acquire swapchain texture.
  2. Rebuild/patch the Vello `Scene`.
  3. Submit `RenderContext::render_to_surface`.
  4. Present.
- **Animation:** Honor `FrameClock` delta to run transitions (e.g., progress animation).
- **Hot Reload:** Keep device/context creation separate so CLI hot reload can re-use surfaces.

### 7.2 CPU (tiny-skia)

Both modules share the same `RenderBackend` trait, so future surfaces (TUI, embedded, etc.) can
plug in once they are ready.

- **Context:** `cpu::PixmapSurface` wraps a `tiny_skia::PixmapMut`. Window backends can upload it
  to OS textures (e.g., share as RGBA buffer).
- **Scene Execution:** Traverse `SceneGraph` and draw directly:
  - Text via `tiny_skia::TextLayout` (or integrate `cosmic-text` for shaping).
  - Images via `Pixmap::draw_pixmap`.
  - Controls via primitive drawing.
- **Reuse Layout Tree:** Use the exact same `SceneGraph` creation logic as GPU to avoid drift.
- **Testing Hooks:** Provide `render_to_png(&mut self, path)` for CI snapshots.

---

## 8. Component Coverage Plan

| Component        | Source Config (example)                       | Node responsibilities                                                                 |
|------------------|-----------------------------------------------|--------------------------------------------------------------------------------------|
| `Text`           | `Native<TextConfig>`                          | Watch `Computed<StyledStr>`, resolve font + color, produce glyph runs.               |
| `Image`/`Photo`  | `Native<PhotoConfig>`                         | Async load via `assets::images`, placeholder view as nested tree.                   |
| `Container`      | `Native<Container>`                           | Watch `AnyViews`, rebuild children incrementally.                                    |
| `FixedContainer` | `Native<FixedContainer>`                      | Store boxed layout + Vec<AnyView>, rebuild entirely when config changes.            |
| `ScrollView`     | `Native<ScrollView>`                          | Maintain scroll state, clip child scene, emit scrollbars (optional).                |
| `TextField`      | `Native<TextFieldConfig>`                     | Render text, caret, prompt, and send keystrokes to `Binding<Str>`.                  |
| `Toggle`         | `Native<ToggleConfig>`                        | Render switch visuals, update `Binding<bool>` on interaction.                       |
| `ProgressBar`    | (to be defined under controls/media)          | Draw determinate progress rectangle, animate `Binding<f32>` changes.                |
| `ImageView`      | (if `components/media::image`)                | Share logic with Photo but allow raw pixels.                                        |

Each handler lives under `src/components/` and registers with the dispatcher.

---

## 9. Implementation Phases

1. **Crate scaffolding**
   - Move the existing `ViewDispatcher` into `src/dispatcher.rs`.
   - Add feature flags (`gpu`, `cpu`, default `std`), dependencies (wgpu, vello, tiny-skia, nami).
2. **Render tree + reactive core**
   - Implement arena, node ids, parser, and watchers.
   - Provide minimal node types (`Group`, `Placeholder`) to validate pipeline.
3. **Layout engine + geometry**
   - Integrate `waterui_layout::Layout` trait objects.
   - Implement `FixedContainer` and `Container` parsing + layout driving.
4. **Component handlers**
   - Text + color resolution first (reuses `waterui_text`).
   - Image/Photo loader + cache.
   - Controls (TextField, Toggle, ProgressBar) including event plumbing.
5. **Backend adapters**
   - Implement Vello renderer (surface creation, scene building, texture caches).
   - Implement tiny-skia renderer (pixmap drawing, CPU event loop integration).
6. **Testing + samples**
   - Snapshot tests for CPU backend (PNG diffs).
   - Smoke tests with offscreen GPU surface.
   - Example binary under `backends/hydrolysis/examples/` rendering the demo view and saving both CPU/GPU outputs (TODO: GPU comparison when headless surfaces are available).
7. **Adopt in backends**
   - Integrate Hydrolysis into desktop/mobile backends via window surfaces.
   - (TODO) Port the legacy TUI backend once GPU/CPU paths are stable.

---

## 10. Open Questions & Risks

1. **Text shaping:** `StyledStr` may require advanced shaping (ligatures, bidi). Need to decide
   between `vello_text` vs `cosmic-text` for both GPU and CPU parity.
2. **Async image loading:** `Photo::load` is currently `todo!()`. We need an async runtime hook
   for decoding without blocking the render thread.
3. **Event dispatch:** Window crate does not yet expose input events; we may need to expand it or
   let renderers integrate with `winit`.
4. **Scroll/Focus state:** Determine whether state lives inside nodes or in a higher-level
   `FocusManager`. The plan assumes a central manager under `runtime::input`.
5. **FFI integration:** Native backends (Android/iOS) still consume FFI, so we must confirm that
   building Rust-native renderers will not conflict with existing workflows.

This plan will guide the upcoming implementation work in Hydrolysis.

---

## 11. Current Status & Immediate Next Steps

- ✅ **Scene/backends** – tiny-skia and Vello surfaces read the shared `DrawCommand` stream. Text nodes currently emit placeholder text commands.
- ✅ **Tree builder** – `build_tree` recognises `Native<TextConfig>` and resolves other views via `body(env)`. Containers/controls/images are pending.
- ⚙️ **Work in progress**
  - Reactive helpers (`NodeSignal`, binding watchers) – needed to mark nodes dirty automatically.
  - Layout engine wiring – `LayoutEngine` is scaffolded but not yet fed with real layout nodes.
  - Control/input layer – bindings, focus, hit-testing, and pointer/keyboard routing still TODO.
- 🎯 **Immediate priorities**
  1. Integrate `cosmic-text` (or `vello_text`) for real text shaping + measurement across CPU/GPU.
  2. Parse raw layout views (`FixedContainer`, `ScrollView`, Spacer) and drive them through `LayoutEngine`. (Fixed containers + scroll views now stubbed; stacks pending.)
  3. Implement basic control nodes (Slider/Toggle/Stepper/TextField/Progress/Divider placeholder nodes present, awaiting proper skins) with draw skins and binding mutations.
  4. Introduce the event infrastructure so bindings update instantly on user input (reactive signals now cover computed value watching).
 5. Add CPU snapshot + GPU smoke tests plus a CLI sample that renders the full demo view via Hydrolysis.

---

## 12. Iteration Checklist

- [x] Wire `HydrolysisRenderer`, scene recording, and tiny-skia PNG sample for manual validation.
- [x] Parse `FixedContainer`/`ScrollView` into placeholder layout nodes during tree construction (layout logic still TODO).
- [ ] Integrate a real text shaper/measurement pipeline (target: `cosmic-text` once dependency access is available) and feed glyph runs into the `Scene`.
- [ ] Flesh out `FixedContainerNode`/`LayoutEngine` so `waterui_layout::Layout` objects drive child measurement/placement.
- [ ] Introduce `NodeSignal`/binding watchers that mark nodes dirty and update internal state without rebuilding the entire tree.
- [ ] Flesh out render nodes (current placeholders for Divider/Spacer/Slider/Stepper/Toggle/Progress/TextField need real skins + interactive states).
- [ ] Build the `runtime::input` dispatcher (hit testing, focus, pointer/keyboard routing) so control nodes can react to user events immediately.
- [ ] Implement async image/texture caches (Photo/Image) and media nodes required by the demo.
- [ ] Expand testing: CPU snapshot diffs, GPU smoke tests, and CLI demos that render the full example view across both backends.
## 2.1 Feature Flags

- `cpu` (default): enables the tiny-skia backend (CPU raster surface). Compiles without linking
  to GPU or wasm dependencies.
- `gpu`: pulls in `wgpu` + `vello` to enable the Vello-based renderer. Platforms can opt-in when
  they have a wgpu surface available.
Both features can be enabled at once, letting applications pick the backend at runtime.
