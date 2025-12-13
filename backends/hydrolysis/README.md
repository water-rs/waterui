# Hydrolysis

The self-drawn renderer backend for WaterUI, providing GPU and CPU rendering capabilities.

## Overview

Hydrolysis is WaterUI's experimental self-drawn renderer that rasterizes views directly into pixels, as opposed to the native backends (Apple/Android) which delegate to platform UI frameworks. This backend enables WaterUI applications to run on desktop platforms without requiring SwiftUI or Jetpack Compose, and serves as the foundation for the terminal UI backend.

The crate provides a pluggable architecture supporting multiple rendering surfaces through feature flags:
- **CPU rendering** via `tiny-skia` for software rasterization
- **GPU rendering** via `Vello` and `wgpu` for hardware-accelerated graphics

Unlike the FFI-based backends, Hydrolysis parses WaterUI's `AnyView` tree directly into a render tree of specialized nodes, performs layout passes, and emits draw commands to the selected backend surface.

## Installation

Add Hydrolysis to your `Cargo.toml`:

```toml
[dependencies]
hydrolysis = "0.1.0"

# Choose one or both rendering backends
# CPU backend (default)
hydrolysis = { version = "0.1.0", features = ["cpu"] }

# GPU backend
hydrolysis = { version = "0.1.0", features = ["gpu"] }
```

## Quick Start

### CPU Rendering with tiny-skia

```rust
use hydrolysis::{HydrolysisRenderer, backend::TinySkiaBackend};
use waterui::prelude::*;
use waterui_core::Environment;

fn main() {
    // Create environment and view
    let env = Environment::new();
    let view = Text::new("Hello, Hydrolysis!");

    // Initialize CPU backend with 800x600 pixmap
    let backend = TinySkiaBackend::new(800, 600)
        .expect("failed to create pixmap");

    let mut renderer = HydrolysisRenderer::new(backend);

    // Render the view
    renderer.render_view(&env, view);

    // Access the rendered pixmap
    let pixmap = renderer.backend().pixmap();
    pixmap.save_png("output.png").expect("failed to save");
}
```

### GPU Rendering with Vello

```rust
use hydrolysis::{HydrolysisRenderer, backend::VelloWgpuBackend};
use waterui::prelude::*;
use waterui_core::Environment;

fn main() {
    // Setup wgpu device, queue, and surface (typically from winit)
    // ... window setup code ...

    let backend = VelloWgpuBackend::new(surface, device, queue, config)
        .expect("failed to create GPU backend");

    let mut renderer = HydrolysisRenderer::new(backend);
    let env = Environment::new();

    // Render loop
    loop {
        let view = app_view();
        renderer.render_view(&env, view);
    }
}
```

## Core Concepts

### HydrolysisRenderer

The main entry point that orchestrates the rendering pipeline. It owns a `RenderBackend` implementation and a `RenderTree`, managing the full lifecycle from view parsing to frame presentation.

```rust
pub struct HydrolysisRenderer<B: RenderBackend> {
    backend: B,
    tree: RenderTree,
}
```

### RenderBackend Trait

All concrete rendering surfaces implement this trait:

```rust
pub trait RenderBackend: Debug {
    fn render(&mut self, tree: &mut RenderTree, env: &Environment) -> FrameResult;
}
```

Implementations:
- `TinySkiaBackend` - CPU rasterization into a `Pixmap`
- `VelloWgpuBackend` - GPU-accelerated rendering to wgpu surfaces

### RenderTree

An arena-based tree structure storing parsed `RenderNode` trait objects. Each node represents a component (Text, Divider, Button, etc.) and implements layout and paint operations.

```rust
pub struct RenderTree {
    nodes: Vec<NodeEntry>,
    root: Option<NodeId>,
    dirty: Vec<DirtyNode>,
}
```

Nodes are marked dirty when they require reprocessing due to layout invalidation, paint updates, or reactive state changes.

### RenderNode Trait

The core abstraction implemented by all renderable components:

```rust
pub trait RenderNode: Debug {
    fn layout(&mut self, ctx: LayoutCtx<'_>) -> LayoutResult;
    fn paint(&mut self, ctx: &mut RenderCtx<'_>);
    fn update_reactive(&mut self) {}
}
```

- `layout()` - Measures the node's size given constraints
- `paint()` - Emits draw commands into the scene
- `update_reactive()` - Refreshes cached reactive values when bindings change

### Scene and DrawCommand

Hydrolysis uses a retained-mode scene graph. During rendering, nodes emit `DrawCommand` instances into a `Scene`:

```rust
pub enum DrawCommand {
    SolidRect { rect: Rect, color: ResolvedColor },
    Text { content: String, origin: Point, color: ResolvedColor, size: f32 },
    Placeholder(&'static str),
}
```

Backends consume these commands and translate them to their native primitives (tiny-skia paths, Vello shapes, etc.).

## Examples

### Rendering a Styled Layout

```rust
use hydrolysis::{HydrolysisRenderer, backend::TinySkiaBackend};
use waterui::prelude::*;
use waterui_core::Environment;

let env = Environment::new();
let backend = TinySkiaBackend::new(400, 300).unwrap();
let mut renderer = HydrolysisRenderer::new(backend);

let view = VStack::new((
    Text::new("Welcome to Hydrolysis"),
    Divider::new(),
    Text::new("Self-drawn rendering"),
));

renderer.render_view(&env, view);
```

### Reactive State with CPU Backend

```rust
use hydrolysis::{HydrolysisRenderer, backend::TinySkiaBackend};
use waterui::prelude::*;
use waterui_core::Environment;
use nami::Binding;

let env = Environment::new();
let counter = Binding::new(0);

let view = Button::new(
    Text::new(counter.map(|n| format!("Count: {n}"))),
    move || counter.update(|n| n + 1),
);

let backend = TinySkiaBackend::new(200, 100).unwrap();
let mut renderer = HydrolysisRenderer::new(backend);
renderer.render_view(&env, view);
```

### Accessing Rendered Output

```rust
use hydrolysis::{HydrolysisRenderer, backend::TinySkiaBackend};
use waterui::prelude::*;
use waterui_core::Environment;

let env = Environment::new();
let view = Text::new("Render me!");

let backend = TinySkiaBackend::new(800, 600).unwrap();
let mut renderer = HydrolysisRenderer::new(backend);

renderer.render_view(&env, view);

// Access the pixmap directly
let pixmap = renderer.backend().pixmap();
let pixel_data = pixmap.data();
println!("Rendered {} bytes", pixel_data.len());
```

## API Overview

### Main Types

- `HydrolysisRenderer<B>` - High-level renderer driving the pipeline
- `RenderBackend` - Trait for surface implementations
- `TinySkiaBackend` - CPU surface using tiny-skia
- `VelloWgpuBackend` - GPU surface using Vello/wgpu
- `RenderTree` - Arena storing parsed render nodes
- `RenderNode` - Trait for components implementing layout/paint
- `Scene` - Collection of draw commands
- `DrawCommand` - Primitive rendering operations

### Layout Primitives

- `LayoutCtx` - Context passed during layout pass
- `LayoutResult` - Size returned by layout operations
- `Point` - 2D coordinate (x, y)
- `Size` - 2D dimensions (width, height)
- `Rect` - Axis-aligned rectangle (origin, size)

### Reactive Integration

- `NodeSignal<T>` - Wraps `Computed<T>` with dirty tracking for render nodes
- `DirtyNode` - Entry marking a node requiring reprocessing
- `DirtyReason` - Why a node became dirty (Layout, Paint, Reactive)

## Features

### `cpu` (default)

Enables the CPU rendering backend using `tiny-skia`. This backend:
- Renders into a software `Pixmap`
- Supports arbitrary canvas sizes
- Provides simple PNG export
- Ideal for headless rendering and testing

### `gpu`

Enables the GPU rendering backend using `Vello` and `wgpu`. This backend:
- Leverages hardware acceleration
- Integrates with `winit` for windowing
- Supports high-DPI displays with scale factors
- Automatically handles surface reconfiguration

## Architecture Notes

### View Parsing

The `build_tree()` function in `tree::parser` converts an `AnyView` into a `RenderTree` by:
1. Downcasting to known view types (Text, Button, Divider, etc.)
2. Creating specialized `RenderNode` implementations
3. Recursively expanding composite views via `.body()`
4. Building parent-child relationships in the tree

Currently supported view types:
- `Text` - Text rendering with basic styling
- `Divider` - Horizontal/vertical separator lines
- `FixedContainer` - Layout containers (HStack, VStack, ZStack)
- `Spacer` - Flexible layout gaps
- `Slider`, `Stepper`, `Toggle`, `TextField` - Form controls
- `ProgressView` - Progress indicators

### Layout Engine

The `LayoutEngine` performs a depth-first traversal calling `layout()` on each node. Layout is currently simplified and does not yet implement full constraint-based sizing or flex layout algorithms.

### Render Pipeline

1. **Mark Dirty** - Reactive changes or manual invalidation mark nodes dirty
2. **Update Reactive** - Nodes refresh cached `NodeSignal` values
3. **Layout Pass** - `LayoutEngine` measures all nodes
4. **Paint Pass** - Nodes emit `DrawCommand` instances into a `Scene`
5. **Rasterize** - Backend translates scene to surface (pixmap or GPU texture)

## Current Limitations

This backend is under active development. Known limitations:

- **Text rendering**: Placeholder text measurement; no font shaping or cosmic-text integration yet
- **Layout system**: Simplified layout; missing constraint propagation and flex sizing
- **Diffing**: Rebuilds tree every frame; no incremental updates or node reuse
- **Reactive wiring**: Manual dirty marking; automatic binding watchers not fully integrated
- **Gradient support**: Only solid fills implemented; gradients are placeholders
- **Image rendering**: Not yet implemented

Check inline `TODO` comments in the source for specific work items.

## Contributing

Hydrolysis is experimental and contributions are welcome. Priority areas:
- Text shaping and glyph rendering integration
- Proper constraint-based layout engine
- Incremental tree diffing and node reuse
- Image and gradient rendering
- Performance profiling and optimization
