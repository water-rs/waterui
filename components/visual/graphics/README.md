# waterui-graphics

GPU and engine rendering primitives for WaterUI applications.

## Overview

`waterui-graphics` hosts the rendering contracts a WaterUI backend consumes:

- **GpuContentView** - a view driven by a `GpuContent` producer drawing raw wgpu frames
- **SceneView** - a view whose `SceneContent` records vector scenes into `cherenkov::Recorder`
- **ShaderPaintView** - a view painted by a WGSL fragment shader
- **Gradients** - `Gradient` (linear, radial, angular, mesh), the signal-driven `MeshGradient`, and the GPU-animated `AnimatedMeshGradient` and `FlowingGradient`
- **OffscreenRenderer** - headless rendering of either kind of content to `OffscreenImage`
- **Effects** - GPU filter, transition and capture effects applied to arbitrary views

GPU paths run at display refresh rates and support HDR surfaces when the host offers one.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-graphics = "0.1.0"
```

Or via the main `waterui` crate:

```toml
[dependencies]
waterui = "0.3"
```

## Quick Start

### GpuContentView - Full wgpu Control

For custom GPU rendering, implement the `GpuContent` trait and wrap it in a
`GpuContentView`:

```rust
use waterui::graphics::{Context, Frame, GpuContent, GpuContentView};
use waterui::prelude::*;

struct Flame;

impl GpuContent for Flame {
    fn setup(&mut self, gpu: &Context<'_>) {
        // Create pipelines, buffers and bind groups against `gpu.device`.
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        // Encode one frame into `frame.view`, submit to `frame.queue`,
        // and call `frame.request_redraw()` when the content animates.
    }
}

fn main() -> impl View {
    GpuContentView::new(Flame).size(400.0, 500.0)
}
```

`GpuContent` is `Send`: it is constructed on the UI thread and then lives on
the render thread. Producers that are confined to the UI thread — a browser
engine, a camera stream — keep their state in a `.on_frame` hook and post
frames to the render side through shared state:

```rust
GpuContentView::new(RenderSide::new())
    .on_frame(move || bridge.borrow_mut().frame())
```

Per-view hooks also exist for input (`on_input`), the IME caret
(`on_ime_caret`), and accessibility (`labeled`, `described`).

### SceneView - Vector Scenes

`SceneContent` records a vector scene into a `cherenkov::Recorder` each frame;
the engine resources it names come from `RecordingResources`:

```rust
use waterui::graphics::{RecordingResources, SceneContent, SceneView, cherenkov};

struct Graph;

impl SceneContent for Graph {
    fn build_scene(
        &mut self,
        recorder: &mut cherenkov::Recorder,
        resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> bool {
        // Record drawing commands; return true when the scene changed.
        true
    }
}

let view = SceneView::new(Graph);
```

### ShaderPaintView - WGSL Shaders Made Easy

```rust
use waterui::graphics::ShaderPaintView;

let view = ShaderPaintView::new(r#"
    @fragment
    fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
        let t = uniforms.time;
        return vec4<f32>(uv.x, uv.y, sin(t), 1.0);
    }
"#).animated(true);
```

`uniforms.time` and `uniforms.resolution` are injected automatically; extra
user uniforms follow a signal via `.uniforms(...)`.

## Core Concepts

### GpuContent Lifecycle

```rust
pub trait GpuContent: Send + 'static {
    fn setup(&mut self, gpu: &Context<'_>);
    fn render(&mut self, frame: &mut Frame<'_>);
    fn is_opaque(&self) -> bool { false }
    fn intrinsic_size(&self) -> Option<Size> { None }
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions { /* fill proposal */ }
    fn preferred_surface_hdr(&self) -> Option<bool> { None }
}
```

- `setup()` - Called once on the render thread; create pipelines, buffers, bind groups
- `render()` - Called per frame with `Frame` carrying `device`, `queue`, `texture`, `view`, `format`, `width`, `height`, `scale`, `elapsed`, `delta`

### External Frames

Frames that already live in GPU memory — a video decoder's `CVPixelBuffer`, an
`AHardwareBuffer`, a dmabuf — are not drawn by content. An
`ExternalFrameSource` imports their planes onto the host's device and
publishes `cherenkov_gpu::interop::ExternalFrame`s, which become the content
of the `ExternalFrameView`'s own engine layer:

```rust
pub trait ExternalFrameSource: 'static {
    fn start(&mut self, output: FrameOutput);
    fn is_opaque(&self) -> bool { false }
    fn intrinsic_size(&self) -> Option<Size> { None }
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions { /* fill proposal */ }
    fn preferred_surface_hdr(&self) -> Option<bool> { None }
}
```

- `start()` - Called on the UI thread each time a host builds the layer —
  first presentation, and again on the new device after a device loss. The
  `FrameOutput` is cloneable and `Send` on native targets; the decoder thread
  imports planes onto `output.device()` and calls `output.present(frame)`.
- A newer frame replaces one the host has not drawn yet; the host drains the
  newest frame once per engine pass, so publishing never rebuilds the view.
- The engine samples the planes in place and converts them with the frame's
  `FrameColor` (matrix, range, siting, primaries, transfer, reference white)
  behind its `FrameSync`.
- `present` returns `RetiredOutput` once the host that started the output is
  gone; stop producing for it.

### Offscreen Rendering

`GpuRuntime::render_content(content, size, scale)` renders a `GpuContent` to an
`OffscreenImage` without a window; `OffscreenRenderer` does the same for scene
content on either the GPU or CPU (`OffscreenRenderer::cpu`).

## API Overview

### Gpu Module

- `GpuContentView::new(content)` - Create a view from a `GpuContent`
- `ExternalFrameView::new(source)` - Create a view from an `ExternalFrameSource`
- `FrameOutput` - Where a source publishes frames: `device()`, `queue()`, `present(frame)`
- `GpuContentRenderer` / `ExternalFrameRenderer` - Host-side renderers presenting either view into a native texture
- `Context` - GPU resources during setup (adapter, device, queue, format, redraw)
- `Frame` - Frame data during render (device, queue, texture, view, dimensions, timings)
- `GpuRuntime` - Shared wgpu runtime for offscreen and host-driven rendering
- `preferred_surface_format(caps, prefer_hdr)` - Pick a surface format, HDR preferred

### Scene Module

- `SceneView::new(content)` - Create a view from a `SceneContent`
- `SceneResources` - Engine resource registry a host builds with `new(Rc<Engine<B>>)`; `recording()` opens a `RecordingResources` for each recording
- `RecordingResources` - The registration half handed to `build_scene`: `name`/`hold` plus `font`/`image`/`image16f`/`shader`
- `Picture` - A retained recorded scene

### Re-exported Dependencies

- `wgpu` - Direct access to wgpu types for `GpuContent` implementations
- `cherenkov` - The scene engine (`Recorder`, `Engine`, `Display`)
- `kurbo` - 2D geometry
- `bytemuck` - Safe byte conversions for uniform buffers

## Platform Support

Graphics rendering requires a platform backend that presents engine content:

- **Apple** (iOS, macOS) - Metal backend
- **Android** - Vulkan backend
- **Hydrolysis** - Cherenkov GPU backend

Terminal UI backend (`tui`) does not support GPU rendering.
