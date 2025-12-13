# waterui-graphics

High-performance GPU rendering primitives for WaterUI applications.

## Overview

`waterui-graphics` provides three distinct APIs for GPU-accelerated rendering, each targeting different levels of abstraction and use cases:

- **Canvas** - Beginner-friendly 2D vector graphics using Vello (shapes, paths, fills, strokes)
- **ShaderSurface** - Intermediate shader-based rendering with automatic pipeline setup
- **GpuSurface** - Advanced low-level wgpu access for custom GPU rendering

All three APIs render at display refresh rates (60-120fps+) and support HDR surfaces when available. The crate automatically handles surface format selection, preferring Rgba16Float for HDR displays and falling back to sRGB formats on SDR displays.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-graphics = "0.1.0"
```

Or via the main `waterui` crate:

```toml
[dependencies]
waterui = "0.1.0"
```

## Quick Start

### Canvas - Draw 2D Shapes

The simplest way to draw vector graphics:

```rust
use waterui::graphics::Canvas;
use waterui::graphics::kurbo::{Circle, Line, Point, Rect};
use waterui::graphics::peniko::Color;
use waterui::prelude::*;

fn main() -> impl View {
    vstack((
        text("H₂O Molecule").size(24),
        text("Simple 2D molecular visualization").size(14),
        Canvas::new(|ctx| {
            let size = ctx.size();
            let center = ctx.center();

            // Background
            ctx.fill(
                Rect::from_origin_size(Point::ZERO, size),
                Color::new([0.08, 0.1, 0.14, 1.0]),
            );

            // Molecule geometry
            let oxygen_radius = 40.0;
            let hydrogen_radius = 22.0;
            let bond_length = 90.0;

            // Water bond angle ~104.5°
            let angle = 104.5_f64.to_radians() / 2.0;

            let hx1 = center.x - bond_length * angle.sin();
            let hy1 = center.y + bond_length * angle.cos();

            let hx2 = center.x + bond_length * angle.sin();
            let hy2 = center.y + bond_length * angle.cos();

            let oxygen = center;
            let hydrogen1 = Point::new(hx1, hy1);
            let hydrogen2 = Point::new(hx2, hy2);

            // Bonds
            ctx.stroke(
                Line::new(oxygen, hydrogen1),
                Color::new([0.9, 0.9, 0.9, 0.8]),
                4.0,
            );

            ctx.stroke(
                Line::new(oxygen, hydrogen2),
                Color::new([0.9, 0.9, 0.9, 0.8]),
                4.0,
            );

            // Atoms
            // Oxygen (O)
            ctx.fill(
                Circle::new(oxygen, oxygen_radius),
                Color::new([0.85, 0.2, 0.25, 1.0]),
            );

            // Hydrogens (H)
            ctx.fill(
                Circle::new(hydrogen1, hydrogen_radius),
                Color::new([0.95, 0.95, 0.95, 1.0]),
            );

            ctx.fill(
                Circle::new(hydrogen2, hydrogen_radius),
                Color::new([0.95, 0.95, 0.95, 1.0]),
            );
        }),
        text("Bond angle ≈ 104.5°").size(12),
    ))
    .padding()
}
```

### ShaderSurface - WGSL Shaders Made Easy

Load and render fragment shaders with automatic uniform management:

```rust
use waterui::graphics::shader;
use waterui::prelude::*;

fn main() -> impl View {
    vstack((
        text("Flame Animation").size(24),
        text("GPU-rendered procedural fire").size(14),
        // Just one line to load and render a shader!
        shader!("starfield.wgsl").size(400.0, 500.0),
        text("Rendered at 120fps").size(12),
    ))
    .padding()
}
```

The shader automatically receives these uniforms:

```wgsl
struct Uniforms {
    time: f32,           // Elapsed time in seconds
    resolution: vec2<f32>, // Surface size in pixels
    _padding: f32,
}
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let t = uniforms.time;
    return vec4<f32>(uv.x, uv.y, sin(t), 1.0);
}
```

### GpuSurface - Full wgpu Control

For advanced rendering pipelines, implement the `GpuRenderer` trait:

```rust
use waterui::graphics::{GpuContext, GpuFrame, GpuRenderer, GpuSurface, bytemuck, wgpu};
use waterui::prelude::*;

fn main() -> impl View {
    vstack((
        text("Cinematic HDR Flame (GpuSurface)").size(24),
        text("HDR film buffer + bloom + ACES tonemap").size(14),
        GpuSurface::new(FlameRenderer::default()).size(400.0, 500.0),
        text("Rendered at 120fps").size(12),
    ))
    .padding()
}
```

## Core Concepts

### Canvas Drawing API

`Canvas` provides a callback-based API where you receive a `DrawingContext` each frame:

- **Shapes** - Fill and stroke circles, rectangles, lines, and arbitrary paths using `kurbo` geometry primitives
- **Styling** - Apply solid colors or gradients using `peniko` brushes
- **Layers** - Push clip layers and alpha layers for compositing effects
- **Performance** - Renders via Vello at full GPU speed with anti-aliasing

The `DrawingContext` provides helper methods:

```rust
ctx.size()      // Canvas dimensions as kurbo::Size
ctx.center()    // Center point
ctx.fill(shape, color)              // Fill a shape with solid color
ctx.fill_brush(shape, brush)        // Fill with gradient/pattern
ctx.stroke(shape, color, width)     // Stroke a shape outline
ctx.push_clip(shape)                // Begin clipping region
ctx.push_alpha(alpha, bounds)       // Begin alpha layer
ctx.pop_layer()                     // End layer
ctx.scene()                         // Access underlying Vello scene
```

### ShaderSurface Uniforms

`ShaderSurface` automatically injects a uniform buffer accessible via `@group(0) @binding(0)`:

- `time: f32` - Elapsed seconds since shader creation (for animations)
- `resolution: vec2<f32>` - Current surface size in pixels
- Fragment input `uv: vec2<f32>` - Normalized coordinates (0.0 to 1.0)

### GpuRenderer Lifecycle

The `GpuRenderer` trait defines three lifecycle methods:

```rust
pub trait GpuRenderer: Send + 'static {
    fn setup(&mut self, ctx: &GpuContext);
    fn render(&mut self, frame: &GpuFrame);
    fn resize(&mut self, width: u32, height: u32) {}  // Optional
}
```

- `setup()` - Called once when GPU resources are ready; create pipelines, buffers, bind groups
- `resize()` - Called when surface size changes (before render); recreate size-dependent resources
- `render()` - Called each frame with `GpuFrame` containing device, queue, texture, and dimensions

### HDR Support

All three APIs automatically detect and utilize HDR surfaces:

```rust
// In setup()
if ctx.is_hdr() {
    // Surface format is Rgba16Float or Rgba32Float
    // Use extended color range (values > 1.0)
}

// In render()
if frame.is_hdr() {
    // Render with HDR-specific parameters
}
```

HDR surfaces use `Rgba16Float` format when available, allowing color values beyond 1.0 for highlights and bloom effects.

## Examples

### Drawing with Gradients

```rust
use waterui::graphics::Canvas;
use waterui::graphics::kurbo::Circle;
use waterui::graphics::peniko::{Brush, Color, Gradient};

Canvas::new(|ctx| {
    let gradient = Gradient::new_linear((0.0, 0.0), (ctx.width as f64, ctx.height as f64))
        .with_stops([
            (0.0, Color::new([1.0, 0.2, 0.2, 1.0])),
            (1.0, Color::new([0.2, 0.2, 1.0, 1.0])),
        ]);

    ctx.fill_brush(
        Circle::new(ctx.center(), 100.0),
        &Brush::Gradient(gradient),
    );
})
```

### Inline Shader

```rust
use waterui::graphics::ShaderSurface;

ShaderSurface::new(r#"
    @fragment
    fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
        let t = uniforms.time;
        let color = vec3<f32>(uv.x, uv.y, sin(t));
        return vec4<f32>(color, 1.0);
    }
"#)
```

### Custom Render Pipeline

See `/Users/lexoliu/Coding/waterui/examples/flame/src/lib.rs` for a complete example implementing:
- Multi-pass HDR rendering
- Procedural flame shader with fractal noise
- Bloom post-processing with separable Gaussian blur
- ACES tonemapping with vignette and film grain

## API Overview

### Canvas Module

- `Canvas::new(draw_fn)` - Create canvas with drawing callback
- `DrawingContext` - Frame-by-frame drawing context with shape rendering methods
- Re-exports: `kurbo` (2D geometry), `peniko` (colors, brushes, gradients)

### ShaderSurface Module

- `ShaderSurface::new(wgsl_source)` - Create surface from WGSL fragment shader string
- `shader!(path)` - Macro to load shader from file at compile time
- Automatic uniforms: `time`, `resolution`

### GpuSurface Module

- `GpuSurface::new(renderer)` - Create surface with custom `GpuRenderer`
- `GpuRenderer` trait - Implement for custom GPU rendering logic
- `GpuContext` - GPU resources during setup (device, queue, surface format)
- `GpuFrame` - Frame data during render (device, queue, texture, view, dimensions)
- `preferred_surface_format(caps)` - Helper to select best surface format (HDR preferred)

### Re-exported Dependencies

- `wgpu` - Direct access to wgpu types for `GpuRenderer` implementations
- `bytemuck` - Safe byte conversions for uniform buffers
- `kurbo` - 2D geometry (via `vello::kurbo`)
- `peniko` - Styling primitives (via `vello::peniko`)

## Features

### Default Features

- `canvas` - Enables Canvas API (depends on `wgpu` and `vello`)

### Optional Features

- `wgpu` - Enables GpuSurface and ShaderSurface (no Canvas)

All features are enabled by default. To use only lower-level GPU APIs without Vello:

```toml
[dependencies]
waterui-graphics = { version = "0.1.0", default-features = false, features = ["wgpu"] }
```

## Performance Notes

- Canvas uses Vello's GPU-accelerated vector renderer with area-based anti-aliasing
- All rendering stretches to fill available space by default (`StretchAxis::Both`)
- Use `.size(width, height)` modifier to constrain dimensions
- ShaderSurface compiles WGSL at setup time; compilation errors appear in logs
- GpuSurface provides zero-cost abstraction over raw wgpu rendering

## Platform Support

Graphics rendering requires a platform backend that supports wgpu:

- **Apple** (iOS, macOS) - Metal backend
- **Android** - Vulkan backend
- **Hydrolysis** - CPU rendering via Vello/tiny-skia (experimental)

Terminal UI backend (`tui`) does not support GPU rendering.
