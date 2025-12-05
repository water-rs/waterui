# Design Strategy: GPU-Accelerated Video Processing in `waterui-graphics`

## 1. Objective

Enable high-performance, customizable video processing (decoding, filtering, encoding) within `waterui-graphics` using WebGPU (`wgpu`). The system must support live streaming (network/camera), utilize unified memory (zero-copy) where possible, and integrate seamlessly with the `waterui` reactive system.

## 2. Core Architecture

The architecture follows a **Pipeline** pattern:
`Source (Decoder/Camera)` -> `Filter Graph (Compute Shaders)` -> `Sink (Renderer/Encoder)`

### 2.1. The `VideoFrame` Abstraction

To support unified memory, we cannot simply use `Vec<u8>`. We need an abstraction that wraps a GPU-resident resource.

```rust
pub enum FrameData {
    /// A handle to a wgpu Texture (Zero-copy from Decoder)
    Wgpu(wgpu::Texture),
    /// A platform-specific handle (e.g., CVPixelBuffer ref, AHardwareBuffer)
    /// Used for interop before importing to wgpu.
    Platform(Box<dyn Any>),
    /// Fallback CPU buffer
    Cpu(Vec<u8>),
}

pub struct VideoFrame {
    pub data: FrameData,
    pub timestamp: std::time::Duration,
    pub resolution: (u32, u32),
    pub format: wgpu::TextureFormat,
}
```

### 2.2. The `VideoSource` Trait

Abstracts over sources (Network Stream, File, Camera).

```rust
pub trait VideoSource {
    /// Poll for the next ready frame.
    /// Returns `None` if no new frame is available (yet).
    fn poll_frame(&mut self, ctx: &GraphicsContext) -> Option<VideoFrame>;

    /// Configure the stream (e.g., resolution, codec hints).
    fn configure(&mut self, config: StreamConfig);
}
```

### 2.3. Filter Pipeline (The "Customizable" Part)

Filters are `wgpu` Compute Shaders (or Render Passes) that transform one `VideoFrame` into another.

```rust
pub trait VideoFilter {
    /// Apply the filter to input, writing to output.
    fn process(&self, ctx: &GraphicsContext, input: &VideoFrame, output: &mut VideoFrame);
}

/// A generic filter that runs a user-provided WGSL shader.
pub struct ShaderFilter {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    // User-defined uniforms (e.g., blur radius, color adjustments)
    uniforms: wgpu::Buffer,
}
```

## 3. Unified Memory & Zero-Copy Strategy

To achieve high performance, we must avoid `CPU <-> GPU` data transfers.

### 3.1. Apple (macOS/iOS)

- **Decoder**: `VideoToolbox` decodes H.264/H.265 streams into `CVPixelBuffer`.
- **Interop**: Use `CVMetalTextureCache` to map `CVPixelBuffer` (IOSurface) to a `MTLTexture`.
- **WGPU**: Use `wgpu::hal` (or `wgpu`'s `unsafe` APIs) to wrap the underlying `MTLTexture` into a `wgpu::Texture`.
- **Result**: 0 copies. The decoder writes to memory, the GPU reads the exact same memory for filtering/rendering.

### 3.2. Android

- **Decoder**: `MediaCodec` decodes to a `Surface`.
- **Interop**: The `Surface` is backed by `AHardwareBuffer` (on modern Android).
- **WGPU (Vulkan)**: Import `AHardwareBuffer` as a Vulkan Image using `VK_ANDROID_external_memory_android_hardware_buffer`.
- **Result**: 0 copies.

### 3.3. Desktop (Windows/Linux)

- **Windows**: DX11/DX12 video textures shared handle.
- **Linux**: DMABUF / VA-API surface import to Vulkan.

## 4. Integration with `waterui`

### 4.1. `VideoView` Component

A new primitive widget in `waterui-graphics` (or `media`) that owns a `VideoPipeline`.

```rust
struct VideoView {
    pipeline: VideoPipeline,
}

impl View for VideoView {
    fn render(&self, ctx: &mut RenderContext) {
        // 1. Poll pipeline for latest processed frame (Texture)
        if let Some(texture) = self.pipeline.get_current_texture() {
            // 2. Draw texture to the UI quad
            ctx.draw_texture(texture, self.layout_rect);
        }
    }
}
```

### 4.2. Reactive Integration

Filters should be reactive.

```rust
let blur_radius = binding(5.0);
let video_view = VideoView::new(camera_source)
    .filter(BlurFilter::new().radius(blur_radius));
    // When `blur_radius` changes, the uniform buffer is updated automatically.
```

## 5. Implementation Roadmap

1.  **`waterui-graphics`**: Define `VideoFrame` and `VideoSource` traits.
2.  **`waterui-graphics`**: Implement `ShaderFilter` infrastructure (compiling WGSL, managing BindGroups).
3.  **Backends**: Implement `AppleVideoSource` (using VideoToolbox) and `AndroidVideoSource`.
4.  **Integration**: Connect `VideoView` to the `Hydrolysis` renderer to display the resulting `wgpu::Texture`.
