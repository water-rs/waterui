# Cherenkov public API

This document is the design of Cherenkov's public API: the contract that every implementation direction satisfies, and the only surface through which the correctness oracle and the benchmark harness drive the engine. The decision log behind it is water-rs/cherenkov#2.

Sections marked **Proposal** are not yet agreed; everything else records a decision. Code blocks are signatures and usage, not implementations.

## Principles

- **The API describes what to draw.** Tiles, strips, passes and pipelines never appear in it. wgpu types appear only in the `interop` modules.
- **Semantic primitives are first-class.** A rounded rectangle, a shadow or a glyph run reaches the engine as itself, so its fast path survives. Nothing is lowered to a path at the API boundary.
- **Type safety wherever an invariant is static.** Colour spaces, image storage formats, backend capabilities, thread affinity and paired state are types. Facts that change at run time, such as a display's HDR headroom, stay values.
- **Memory is part of the design.** Shared `Picture`s, typed and compressed image storage, and GPU/CPU budgets with system memory-pressure handling are part of the API.
- **Invisible optimizations are verified invisible.** Layer caching and damage tracking must produce bit-identical output when disabled. This is exact by construction, not by tolerance:
  - Canonical f16 rounding and materialization points are part of the semantics, so a cached and an uncached render round at the same places.
  - Scroll offsets and integer layer translations snap to device pixels as part of the semantics.
  - While a layer's transform, component or scroll track is animating, its content's device translation — glyph runs included — is placed on the ¼-device-pixel grid (round to nearest), so a cached coverage emission is reused across the animation instead of re-rasterizing every frame. The frame a track settles — and every static frame — is placed exactly, glyph subpixel translation included.
  - Content under a fractional transform is re-rasterized rather than resampled from a cache. Promotion to system-compositor planes is compared against in-engine composition with a perceptual tolerance.
- **No runtime fallback.** A backend is chosen deliberately, at build time or once at process start by capability. A failure is an error.

## Crates and backends

| Crate | Directory | Contents |
|---|---|---|
| `cherenkov` | `src/` | Front end: engine, surfaces, layer tree, transactions, resource handles, scrolling, the render-thread loop and the `Backend` contract, CPU geometry. No GPU dependency. |
| `cherenkov-record` | `record/` | The recording layer: the `Draw` verbs, `Recorder`/`StaticRecorder`, `DisplayList`/`Picture`, live operands and operand animation, and the paint, shape, style and glyph vocabulary — no engine, GPU or text layout. `cherenkov` depends on it and re-exports it; another render target takes it alone. |
| `cherenkov-gpu` | `gpu/` | GPU backend `Gpu` and the wgpu, Apple, Android, Windows and Wayland interop. |
| `cherenkov-cpu` | `cpu/` | CPU backends `Raster` (desktop/server: full framebuffer, multi-threaded, SIMD) and `Banded<P>` (microcontroller: banded output, panel pixel formats, flash-resident assets). |
| `cherenkov-shader` | `shader/` | The shared shader composer on naga IR, used by the engine and by filtrate. |
| `filtrate`, `filtrate-core`, `filtrate-derive` | `filtrate/` | Independent filter library: definitions, a thin reference executor, and a derive macro. It keeps its own name and does not depend on the engine crates. |

Capabilities are traits implemented by backend types, so using a missing capability is a compile error:

| Capability trait | `Gpu` | `Raster` |
|---|---|---|
| `Uploads<F>` for an image format `F` | `Rgba8`, `Rgba16F` | `Rgba8`, `Rgba16F` |
| `GpuContent`, `ShaderPaint` | both | |
| `Filters`, `Runs<F>` for a filter `F`, `Effects` | every filter | filters with a CPU kernel |
| `Backdrop`, `BackdropRuns<K, F>` for a backdrop chain `F` | every chain | chains with a CPU kernel |
| `HdrOutput` | tone-mapped extended output | |
| `Planes` | Apple window surfaces | |

Targets beyond the current rows: `Gpu` is meant to accept every image format and grow `Planes` (system-compositor promotion); `Raster` targets `Uploads<Rgba8>` and `Filters`/`Runs<F>` for filters with a CPU kernel; a `Banded<P>` microcontroller backend (banded output, panel formats, flash-resident assets) targets panel-format uploads and CPU-kernel filters.

The table is the target; a backend slice implements the rows it has code for, and the compiler rejects the rest.

Recorded content (`Picture`, `Content`) is backend-independent. Components such as math, chart, svg and map never name a backend. Only the host names one. A render target that is not a Cherenkov backend reads a `DisplayList` through `DisplayList::view`: the commands in order and, per command, the live operand slots with their current values; `DisplayList::apply` then moves a slot to a `SlotUpdate`'s value.

On the native Android backend, the host picks `Gpu` or `Raster` once at process start by querying Vulkan capabilities: the floor is `VK_EXT_rasterization_order_attachment_access` or `VK_KHR_dynamic_rendering_local_read`, plus f16. These two are different synchronization architectures. Ordered attachment access orders overlapping fragments implicitly; local read needs explicit by-region dependencies between overlapping work. They are not interchangeable implementations of the same design. Apple needs no selection: every iOS 26 device and every Apple silicon Mac running macOS 26 meets the floor. macOS 26 also still runs on some Intel Macs, which are outside the floor.

## Threading

- **UI thread: the single state machine.** Layers, backdrop groups, transactions, live recording and nami subscriptions live here, and all of them are `!Send`.
- **Render thread: sole owner of GPU state.** A commit sends an owned change set over a channel. The native UI-to-render channel is bounded at 64 messages; the UI thread waits when it is full.
- **Parallelism over immutable data only.** Recorded `Picture`s are `Send`. Flattening, strip generation and glyph rasterization are data-parallel over owned data.
- **No locks anywhere in the engine.**

## Backend contract

The `cherenkov` crate owns the whole front end and the render thread's loop. A backend crate supplies the render side only: a config type, a `Backend` implementation, its capability implementations, and an `interop` module. Nothing in a backend crate is a public front-end type, and nothing is re-exported.

```rust
/// The render-thread contract. Implemented by a zero-sized marker type (`Gpu`, `Raster`).
pub trait Backend: Sized + 'static {
    type Config: RenderTransfer + 'static;               // GpuConfig, RasterConfig
    type Info: Clone + Send + 'static;                   // GpuInfo, RasterInfo: provenance for reports
    type Target: From<Offscreen> + RenderTransfer + 'static; // Offscreen or an interop window target
    type Renderer: Renderer;                             // the render-thread state; never leaves that thread

    /// Runs on the render thread, once. Creates the device or worker pool.
    fn init(config: Self::Config) -> Result<(Self::Renderer, Self::Info), EngineError>;
}

/// Everything the render loop asks of a backend. Every method runs on the render thread.
pub trait Renderer: 'static {
    type Target;
    type Font: RenderTransfer + 'static;                 // a font validated by `prepare_font`
    /// `waker` wakes the host for the surface's render-side completions; it is silent while the surface is hidden,
    /// and its `visibility()` gates the wakes of the backend's own sources on the surface (`WakeGate`).
    fn create_surface(&mut self, id: SurfaceId, target: Self::Target, waker: CompletionWaker) -> Result<SurfaceInfo, SurfaceError>;
    fn resize_surface(&mut self, id: SurfaceId, size: (u32, u32));
    /// Called when the host's announced visibility changes (see Visibility).
    fn set_visibility(&mut self, id: SurfaceId, visibility: Visibility);
    fn destroy_surface(&mut self, id: SurfaceId);

    /// Runs on the caller thread, before anything is queued: every check a font needs.
    fn prepare_font(font: FontData) -> Result<Self::Font, ResourceError>;
    fn add_font(&mut self, id: FontId, font: Self::Font);
    fn remove_font(&mut self, id: FontId);
    /// A rejection fails every render that draws the image (`RenderError::Rejected`).
    fn add_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError>;
    /// New pixels behind the same id; the render loop then marks changed the surfaces `samples` names.
    fn replace_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError>;
    /// Whether any layer content on `surface`, slot updates applied, samples `resource` (a font, an image or a shader).
    /// The render loop runs a resource's `remove_*` only once this is false for every surface.
    fn samples(&self, surface: SurfaceId, resource: ResourceId) -> bool;
    fn remove_image(&mut self, id: ImageId);

    /// Replaces or updates a layer's recorded content (`Content` / `Picture`), or clears it.
    /// Returns the previous picture when replaced or cleared, for the UI thread to recycle.
    fn set_content(&mut self, surface: SurfaceId, layer: LayerId, content: Option<ContentOp>) -> Option<Picture>;
    /// The layer is gone: drop every cache keyed on it.
    fn remove_layer(&mut self, surface: SurfaceId, layer: LayerId);

    /// Renders every surface in `frame` whose tree or content changed; returns whether a
    /// backend-side source (custom GPU content, an animated shader) wants another frame.
    fn render(&mut self, frame: &Frame<'_>, stats: &mut FrameStats) -> Result<Redraw, RenderError>;
    /// Returns accumulated GPU timings, oldest first, waiting for frames still in flight.
    fn finish_timings(&mut self) -> Result<Vec<FrameTiming>, RenderError> { Ok(Vec::new()) }
    fn readback(&mut self, surface: SurfaceId) -> Result<Readback, RenderError>;

    fn memory(&self) -> MemoryUsage;
    fn trim(&mut self, pressure: Pressure);
}
```

- **Surface limits and cadence.** `SurfaceInfo::max_dimension` lets the UI thread reject an oversized resize before sending it to a backend. Animated backend content returns `Redraw::Wanted { rate }`; the frontend combines its refresh range with active property animations. A window target uses the host-supplied range; `Offscreen` uses `Offscreen::rate` (60 Hz by default).
- **One copy of the layer tree.** The render loop in `cherenkov` owns a `SurfaceTree` per surface: the layer graph, every layer property, its animation track and the sampled value for the current frame. The backend never receives property ops; it keeps only what it alone can produce (encoded fragments, live display lists, atlases, GPU content objects) keyed by `LayerId`, and it reads the tree through `Frame`:

  ```rust
  pub struct Frame<'a> { pub id: FrameId, pub time: FrameTime, pub surfaces: &'a [SurfaceFrame<'a>] }
  pub struct SurfaceFrame<'a> { pub id: SurfaceId, pub size: (u32, u32), pub display: Display,
                                pub clear: WorkingColor, pub changed: bool, pub present_pending: bool,
                                pub display_moved: bool, pub tree: &'a SurfaceTree }
  impl SurfaceTree { pub fn root(&self) -> LayerId; pub fn layer(&self, id: LayerId) -> &LayerNode; }
  pub struct LayerNode { /* sampled for this frame: */ pub transform: Affine, pub opacity: f32,
                         pub scroll_offset: Vec2, pub clip: Option<ShapeData>, pub blend: BlendMode,
                         pub filter: Option<FilterId>, pub backdrop: Option<BackdropId>, pub children: Vec<LayerId>, /* tracks: private */ }
  impl LayerNode { /// `transform * translate(-scroll_offset)`: the space of the content and children.
                   pub fn content_transform(&self) -> Affine;
                   /// Whether child layers or content groups blend onto this layer.
                   pub fn blends_within(&self) -> bool;
                   /// A transform, component or scroll track is running this frame.
                   pub fn animating(&self) -> bool; }
  ```

  The clip applies in the layer's own space (`transform`); content and children are drawn in `content_transform()`, so scrolling moves them inside the clip and never re-records anything. `changed` is true when a property op, a content op or an animation step touched the surface since the last render; the backend renders exactly those surfaces.
- **One wake-up per frame.** `Surface::update`, layer drops and bound-signal changes only queue owned ops on the UI thread. `Engine::render(time)` drains every surface's queue into one `Message::Render { time, commits, reply }` and blocks on the reply, so the render thread wakes once per frame and applies the commits, samples the animations at `time`, renders, and answers with `Next` and the `FrameStats`. Surface creation, readback, `memory()` and `finish_timings()` are request/reply messages; everything else, resource registration and image replacement included, is fire-and-forget and ordered with the renders (see Resources).
- **Capabilities carry their render-side hooks.** A capability trait is not a marker: it declares the function the render loop calls, so a backend without the capability has no code path to reach, and no default or stub exists.

  ```rust
  pub trait ShaderPaint: Backend {
      fn validate_shader(source: &ShaderSource) -> Result<(), ResourceError>;  // caller thread: naga validation
      fn add_shader(r: &mut Self::Renderer, id: ShaderId, source: ShaderSource) -> Result<(), ResourceError>;
      fn remove_shader(r: &mut Self::Renderer, id: ShaderId);
  }
  pub trait Filters: Backend { fn remove_filter(r: &mut Self::Renderer, id: FilterId); }
  pub trait Runs<F: filtrate_core::Filter + Send>: Filters { fn add_filter(r: &mut Self::Renderer, id: FilterId, filter: F); }
  pub trait Effects: Filters { type Effect: Send + 'static; fn add_effect(r: &mut Self::Renderer, id: FilterId, effect: Self::Effect); } // Box<dyn filtrate::Effect + Send> on GPU backends
  pub trait GpuContent: Backend {
      type Content: RenderTransfer + 'static;             // rendered producer payload
      type Frame: RenderTransfer + 'static;              // submitted-frame payload (ExternalFrame on Gpu)
      fn frame_opaque(frame: &Self::Frame) -> bool;
      fn add_gpu_producer(r: &mut Self::Renderer, id: ProducerId, content: Self::Content);
      fn add_frame_producer(r: &mut Self::Renderer, id: ProducerId, dirty: Arc<AtomicBool>, gate: Arc<WakeGate>);
      fn bind_gpu_producer(r: &mut Self::Renderer, surface: SurfaceId, layer: LayerId, producer: GpuProducer<Self>, size: (u32, u32)) -> Option<bool>;   // current frame's declared alpha, None before the first
      fn submit_frame(r: &mut Self::Renderer, id: ProducerId, frame: Self::Frame) -> Vec<(SurfaceId, LayerId)>;
      fn retire_gpu_producer(r: &mut Self::Renderer, id: ProducerId);
      fn drain_gpu_producers(r: &mut Self::Renderer) -> Vec<(ProducerId, DrainedProducer<Self>)>;   // device replacement
  }
  pub trait Uploads<F: Format>: Backend {}      // which image storage formats `add_image` accepts
  pub trait Backdrop: Backend {               // unfiltered groups: `surface.backdrop_group_unfiltered`
      fn add_backdrop_group(r: &mut Self::Renderer, surface: SurfaceId, id: BackdropId);
      fn remove_backdrop_group(r: &mut Self::Renderer, surface: SurfaceId, id: BackdropId);
  }
  pub trait BackdropRuns<K, F>: Backdrop {    // filtered groups: `surface.backdrop_group`
      fn add_filtered_backdrop_group(r: &mut Self::Renderer, surface: SurfaceId, id: BackdropId, filter: F);
  }
  pub trait HdrOutput: Backend {}
  pub trait ProjectiveLayers: Backend {}      // `projection`, `tilt`, `depth` (docs/projective.md)
  pub trait Planes: Backend {}
  ```

  The `Engine`/`Transaction` methods bounded by these traits (`engine.shader`, `engine.filter`, `engine.gpu_producer`, `tx[&l].content(producer.at(size))`, `engine.image::<Astc4x4>`) wrap the hook into an owned `FnOnce(&mut B::Renderer) + Send` op that travels in the commit with the layer ops, in order. The `Renderer` core trait therefore has no shader, filter or GPU-producer methods at all.
- **Resource lifetime.** Handles (`Font`, `Image<F>`, `Shader`, `BackdropShader`, `Filter`) are `Clone` over an `Rc`; the last drop queues the release, ordered with every render. For a font, image, shader or backdrop shader the render loop, not the host, owns the invariant that a resource is freed only once no surface's installed content draws it (see Resources). The backend frees the GPU copy when the release is carried out (deferred to after in-flight frames where the API needs it). The same `Rc` carries the ops a kind queues while it lives: an `Image<F>` queues `replace_image` through it.
- **Errors.** `init` failures surface from `Engine::new` as `EngineError`. Surface creation errors are returned from `engine.surface`. Render errors fail that `render` call. Resource validation that does not need the device (font parsing, byte lengths, shader validation) runs on the UI thread before any message is sent and returns `ResourceError`; a rejection only the backend can detect fails the renders that draw the resource with `RenderError::Rejected` (see Resources).

## Engine

```rust
let engine: Engine<Gpu> = Engine::new(GpuConfig {
    budget: Budget { gpu: Bytes::mib(512), cpu: Bytes::mib(96) },
    pipeline_cache: Some(cache_dir),
    ..GpuConfig::default()
}).await?; // creates the device and asynchronously precompiles the closed pipeline set

engine.trim(Pressure::Critical);       // system memory warning
let usage: MemoryUsage = engine.memory();
```

- The engine owns its render state and may create its device or accept a `cherenkov_gpu::interop::SharedDevice` from the host. `GpuContent` implementations reach wgpu through their backend's `interop::wgpu`. See [retained host integrations](retained-integrations.md) for presentation, producer, shader and filter contracts.
- The pipeline set is closed and fully precompiled at creation, and the driver cache is persisted. Custom shaders compile when they are registered. Nothing compiles at draw time.
- `Engine` is `!Send` and lives on the UI thread. It spawns and owns the render thread; dropping it sends `Shutdown` and joins the thread.
- `engine.info()` is the backend's provenance (`B::Info`); `engine.stats()` the last frame's `FrameStats`.
- **Frame timing.** The render loop numbers every render with a `FrameId` (`Frame::id`), and a backend that draws reports it in `FrameStats::frame`. Every GPU timing is a `FrameTiming` tagged with the frame it measures; the render thread retains them and returns them only from `engine.finish_timings()`, oldest first. They accumulate until that call, which waits for frames still on the GPU; use it at the end of a measured window, never on the frame path. Timestamps (off by default in `GpuConfig`) are for tooling that calls `finish_timings()` at the end of its window.
- **Waking the host.** Changes made outside a frame (a `surface.update`, a layer drop, a bound signal firing) are queued, not sent. When the display link is paused after `Next::Idle`, the host must learn that a frame is needed: `engine.set_waker(|| link.request_now())` registers a callback that the engine calls at most once between two `render`s, the first time something is queued on a visible surface, and once when a surface becomes visible (see Visibility). Every wake goes through the surface the change belongs to, so a hidden surface wakes nothing. An image replacement wakes from the render loop, once it knows a visible surface draws the image. No callback means the host renders on its own schedule.

## Resources

Resources are RAII handles: they are `Clone`, and the GPU memory is released, deferred, once the last handle has dropped and no installed content draws the resource.

```rust
let font: Font = engine.font(FontSource::mapped(path)?)?;          // memory-mapped; never copied
let photo: Image<Astc4x4> = engine.image(
    ImageData::<Astc4x4>::new(width, height, encoded_astc)?.color_space(ImageColorSpace::DisplayP3),
)?;
let hdr: Image<Rgba16F> = engine.image(
    ImageData::<Rgba16F>::new(width, height, decoded)?.color_space(ImageColorSpace::LinearP3),
)?;
hdr.replace(ImageData::<Rgba16F>::new(width, height, next_frame)?)?;  // same id, new pixels
let shader: Shader = engine.shader(ShaderSource::wgsl(fragment))?;  // validated here, GPU only
```

- **One registration model, on every target.** `engine.font`, `engine.image`, `engine.shader`, `engine.backdrop_shader` and `image.replace` are synchronous on native and on wasm32, with the same signatures, so a recording pass can register a resource and draw it in the same frame without awaiting anything. Each call validates on the calling thread what needs no device, allocates the id, queues the backend operation and returns:
  - A font is parsed and prepared by the backend's `Renderer::prepare_font` (an unsupported colour-font format is `ResourceError::Unsupported`); `add_font` then cannot fail, so a font has no later rejection.
  - `ImageData` was validated by `ImageData::new`.
  - A shader's composed module passes naga validation and defines its entry point (`ShaderPaint::validate_shader`, `BackdropShaders::validate_backdrop_shader`).
  - An `Err` from these calls is either that validation or `ResourceError::Lost` (the render thread or executor is gone).

  The queued operation runs ahead of every later render on the render thread or the browser's serial executor. A rejection only the backend can detect (an image over the device's texture limit or the CPU image budget, a pipeline the device cannot create) never passes silently: the render loop records it against the resource, and every render that draws the resource fails with `RenderError::Rejected { resource: ResourceId, reason }`, naming the resource and the backend's reason, until an image replacement succeeds or the resource is freed. The check costs nothing while no rejection is recorded, and afterwards visits only the surfaces that changed since the last render: a rejected id reaches content only through a commit, and a rejected replacement marks every surface sampling the image changed. There is no fallback drawing. Dropping the last handle of a rejected resource enqueues no backend removal, since the backend never committed it.

- **Releasing.** Dropping the last handle of a font, image, shader or backdrop shader never frees what installed content still draws. The host may drop a handle as soon as the content it records next stops naming the resource, whether or not that content is installed yet; a render issued before the install still draws the resource.
  - The release asks every surface whether its installed content draws the resource: `Renderer::samples` for fonts, images and shaders, the layer tree for backdrop shaders. When none does, the backend's `remove_*` runs at once.
  - Otherwise the release is pending, and the render loop remembers which surfaces still draw the resource. After a render's commits are applied, and before its frame, each surface that changed is asked again; a surface's destruction drops it from the set. When the set is empty the removal runs, ahead of the frame that no longer draws the resource. A content replacement, a cleared content, a layer removal and a backdrop change all reach the loop as commits.
  - Content that names a pending resource again, such as a kept recording installed a second time, keeps the resource alive in the same way. Content installed after the removal ran names a resource the backend no longer has: lowering fails that render with an error naming the resource, and no backend draws a substitute.
  - Ids are allocated once per engine and never reused, so a pending id cannot name another resource.
  - A rejected resource's record lives until the release is carried out, so a surface that still draws it keeps failing with `RenderError::Rejected`; a rejected registration's removal is skipped because the backend never committed it.
  - The bookkeeping lives in the render loop shared by the native render thread and the browser executor, so both targets behave identically.
- **`Image<F>`.** `F` is the storage format: `Rgba8`, `Rgba16F`, `Astc4x4`, `Etc2Rgba`, `Bc7`, and panel formats for `Banded`. Only uncompressed formats have `update(region, pixels)`. Compressed formats are uploaded as-is, and compression is an explicit step (`engine.compress::<Astc4x4>(image)`), never implicit.
- **Replacing pixels.** `image.replace(data)` swaps an image's pixels behind the same `ImageId`, for content whose picture changes over time: animated images, decoder-backed frames, a reactive image view. Every recording that names the id keeps drawing it and shows the new pixels on the next frame, without re-recording. The replacement is a fire-and-forget `Message::ReplaceImage`, ordered with frames on the render thread, so no frame samples a partly written image, and the render loop wakes the host through each visible surface that draws the image, so a paused host renders the new pixels. It follows the registration model: `ResourceError::Lost` when the render thread is gone is its only error, and a rejection only the backend can detect leaves the previous pixels in place and fails the renders that draw the image with `RenderError::Rejected` until a later replacement succeeds. Replacing an image whose registration was rejected registers it anew. The render loop then asks the backend which surfaces' content samples the image (`Renderer::samples`, answered from the retained display lists with slot updates applied and nested pictures included) and marks only those changed, so every other surface keeps its skip. An animated image replaced at 30–60 Hz therefore redraws the surfaces that show it and nothing else.
  - The same dimensions reuse the backing storage: `queue.write_texture` into the existing texture on the GPU backend, which leaves every bind group valid; an in-place decode on the CPU backend, once the retained paint operands that shared the pixels are discarded.
  - Different dimensions reallocate the storage behind the same id. The GPU backend retires the bind groups that bound the old texture view. Both backends lower again the retained content that samples the image, because lowering resolves the image's dimensions into its paints.
  - Dropping the last handle after a replacement releases the image as before.
- **Colour metadata.** Every image carries its colour space and optional HDR metadata. `ImageColorSpace` is `Srgb`, `DisplayP3`, `LinearSrgb` or `LinearP3`; `LinearP3` is the working space and decodes as the identity. Conversion into the working space happens when the image is sampled.

## Surfaces and output

```rust
let window = engine.surface(interop::apple::LayerTarget::new(ca_layer))?;  // system-compositor parent
let embedded = engine.surface(interop::android::SurfaceControlTarget::new(parent, size))?;
let plain = engine.surface(interop::window::Target::new(raw_window_handle))?; // single surface
let snapshot = engine.surface(Offscreen::new(size, OffscreenFormat::LinearF16))?;
let panel = engine.surface(Bands::new(size, OffscreenFormat::LinearF16, |band| dma.send(band)))?; // Raster
```

- **Output storage follows the target.** An `Offscreen` surface holds exactly one full-frame buffer, in the `OffscreenFormat` the host asked for — it is the readback image. A `Bands` target on `Raster` holds no framebuffer at all: each rasterized band (a row strip with its filter apron) is delivered to the sink in row order and its storage reused, so peak pixel memory is band-sized, not frame-sized. `Bands` surfaces are not readable; `readback` returns an error. Working scratch never scales with the frame: bands, coverage accumulators and isolation aprons are pooled per worker and bounded by band size (#114).

- **System-compositor parents.** Targets that expose one (`CALayer`, `SurfaceControl`, a DirectComposition visual, a Wayland subsurface) let the engine build **planes**. Most layers are composited inside the engine onto one plane. Eligible layers are promoted automatically to their own system layers: video frames, custom GPU content and large stable layers. A layer is not promoted when it is under a backdrop, uses a non-default blend or has a clip the system cannot express. Hardware overlay budgets also limit promotion.
  - **Decision (#90).** Promotion is decided per frame from the sampled tree alone (`render::planes::plan`), in paint order. A candidate is an external frame the platform's system layer can show itself (on Apple: the planes of one `IOSurface` in the layout and range the frame declares, opaque). It stays in the engine, with a named cause, when an ancestor isolates it (opacity, filter, blend), it carries a filter or a non-default blend, it or any layer painted above it samples a backdrop, a surface-level blend is painted above it, its opacity applies to child layers, a transform or clip on its path is not expressible by the system layer, a shaped clip nests in another, or the plane budget is spent. The budget goes to candidates in paint order.
  - **Parts.** A promoted layer splits the engine's composition: layers painted before it draw into the part below, layers painted after it — its own children included — into a transparent part above, so controls stay above the video. Each part is a full-surface texture presented through its own system layer.
  - **Apple.** A `WindowTarget` captures the view's backing layer on the main thread (`WindowTarget::new` panics elsewhere on Apple). The engine owns a layer tree under it: one `CAMetalLayer` per part (`presentsWithTransaction`), and per plane one nested layer per tree level (transform, clip as `masksToBounds` with `cornerRadius`/`maskedCorners`/`cornerCurve`, scroll as the bounds origin) around an `AVSampleBufferDisplayLayer` fed a `CVPixelBuffer` over the frame's own `IOSurface`, with the frame's primaries, transfer, matrix and chroma siting as attachments. Every frame's geometry and part presentation commits in one `CATransaction`; frames are handed to their display layers after it commits, each enqueue in its own transaction. A producer sync (`FrameSync::Metal`) hands the frame over from an `MTLSharedEvent` listener, never a CPU wait. A display layer fed off the main thread fits its video into its bounds only in a main-thread layout, so a plane whose frame size changes, its first frame included, queues that layout on the main queue after the hand-off commits; until the main run loop turns once, the new frame shows at the previous fit. The budget is two planes. A surface with planes is composited by the system and is not readable.
  - **Android.** A `SurfaceControlTarget` realizes the surface as child surface controls (API 29) of an engine container under the host's parent, whose space is the surface's device pixels. The engine's composited parts present on `AHardwareBuffer`s it renders into (RGBA8, sRGB dataspace, three per part), and a promoted external frame's own buffer goes on a plane of its own with its acquire fence, the dataspace its `FrameColor` maps to and the `HdrMetadata` it was imported with — never through a GPU copy. One transaction per frame sets every part's buffer, every plane's buffer and every changed property (geometry and crop through `setGeometry`, z-order, visibility, alpha), so a plane and the content around it never tear. A plane's release fence is merged into the frame's `FenceFd` release payload, so the producer reuses a buffer only once the system compositor let go of it too. On Android a frame is shown on a plane when its buffer is an `AHardwareBuffer` allocated with `COMPOSER_OVERLAY` usage, its acquire is a sync fence (or none) and its release a fence (or none), its colour contract is one dataspace exactly (BT.709 or BT.2020 with the matching matrix, PQ and HLG at the 203-nit reference white), and its alpha is opaque or premultiplied. Its path must carry only transforms that are axis-aligned up to mirrors and quarter turns and only rectangular clips; destination and crop round to whole pixels. One layer per surface is promoted, so a surface stays at three system layers: the part below, the plane and the part above.
- **Visibility.** The host announces whether the user can see a surface with `surface.visibility(Visibility::Hidden | Visibility::Visible)` from the platform's public signal (window occlusion or minimization, the app in the background, the view detached from its window, `document.visibilityState`); surfaces start visible and a repeated announcement does nothing. It is one setter rather than a `hide`/`show` pair because the host forwards a platform state, and a state is a value (the same shape as `surface.display`).
  - **While hidden** nothing on the surface asks for a frame: its animation tracks and live operands are not sampled, bound signals, transactions, image replacements it draws, custom GPU content, filters and external-frame installs wake no host, the render loop leaves it out of every `Frame` and out of `Next`, and its backend sources do not count in `Redraw`. Every wake on the surface's behalf stops the moment the call returns, whichever thread it starts on. The engine's own wakes go through the surface's waker; a source the backend drives on its own (a GPU producer's or a filter's redraw request) fires through a `WakeGate` that holds the `SurfaceVisibility` of each surface it draws into (from `CompletionWaker::visibility`) and is open only while one of them is visible. The render loop updates a gate's surfaces from its frames, so the membership may lag a frame, but each flag flips on the UI thread when the host announces the change and is read when the wake fires. `Renderer::set_visibility` reaches the backend in order with every other message and decides what counts in `Redraw`.
  - **State keeps flowing.** Changes are accepted and applied while hidden. A hidden surface sends every change to the render loop as it is made (`Message::Apply`) — transactions, layer creates and drops, bound signals and live operands, and whatever was queued for the next frame when the surface hid — and the render loop applies it to the layer tree and the installed content without sampling an animation or drawing. Resource releases see that content in order: content installed while hidden keeps a released resource alive exactly as visible content does, and content that stops drawing it frees it while the surface is still hidden. Nothing accumulates on the UI thread however long the surface stays hidden. Resizes, display changes and resource operations are applied as usual.
  - **Becoming visible** asks the host for exactly one frame, even when the host dropped a frame it was asked for while the surface was hidden. That frame redraws the surface whole from the current state and presents it. Every animation is sampled at that frame's time: a track that ran on while hidden shows where it is now, with no replay of the missed frames, and a track committed while hidden starts on that frame, like any other.
  - **Rendering while every surface of the engine is hidden** is `RenderError::Hidden`, never a silent no-op: a host renders only while a surface is visible. With some surface visible, `render` draws the visible ones and leaves the hidden ones untouched.
  - Native and wasm32 share this in the common render loop and the per-surface waker.
- **Many small surfaces are first-class.** A native backend embeds one surface per self-drawn component, and a list may hold dozens. All surfaces share the engine's pipelines, atlases and caches. Creating and dropping one is cheap. All dirty surfaces render in one submission per frame.
- **Display properties.** Headroom and scale belong to the display, so the host sets them when they change: `surface.display(Display { headroom, scale })`. Presentation reads `headroom` every frame — a headroom change takes effect on the next frame and never re-lowers or re-records content. A move to another display is announced separately as `surface.display_moved()` — a move between numerically identical displays is invisible in `Display`'s values — and it rides the next frame as `SurfaceFrame::display_moved` so a presenting backend re-enumerates the surface's capabilities where a headroom-only update never does (#98).
- **Window output is negotiated, never defaulted.** A window surface selects its swapchain format and colour space from the surface's advertised format/colour-space pairs through wgpu 30's surface colour-space API — an extended-range pair where offered, a wide-gamut SDR pair, else tone-mapped sRGB — and reports the choice and its reason in `OutputSelection`; a silent sRGB fallback does not exist (#98). `WindowTarget::require_color_space` pins a required `wgpu::SurfaceColorSpace`: a surface that cannot advertise it fails creation with `UnsupportedTarget` rather than substituting. `WindowTarget::output_probe` hands the host a `DisplayProbe` — sampled on the main thread on Apple — whose `tone_map_headroom` feeds `Display::headroom` for live EDR and whose `selection()` answers what a hypothetical move would negotiate. The current negotiation is readable at `WindowSurface::selection`.
- **Window presentation pacing is the host's choice, never substituted.** `WindowTarget::display_sync(DisplaySync)` states whether presentation waits for the display (#214). `DisplaySync::Synchronized`, the default, is FIFO: every frame waits for vertical blank and is shown whole, so nothing tears; every surface supports it, and relaxed FIFO, which tears a late frame, is never chosen for it. `DisplaySync::Unsynchronized`, for input-latency measurement and benchmarks, never waits for the display: mailbox where the surface advertises it (the newest frame replaces a queued one and is shown whole at the next vertical blank), otherwise immediate (shown at once, may tear). A surface that advertises neither — Metal on iOS, WebGPU — fails with `UnsupportedTarget` rather than presenting synchronized. The resolved mode is reported as `OutputSelection::present_mode`, and a display move that re-runs negotiation re-resolves it; a surface that no longer advertises what the request needs is a render error, never a reconfiguration to a substitute. On macOS the engine's parts realize immediate presentation as `CAMetalLayer.displaySyncEnabled = false`; they still present inside the frame's `CATransaction`. On Apple, where the engine creates its layers on the main queue, an unsatisfiable `require_color_space` or `display_sync` request arrives as the first render's error instead of at surface creation.

## Frame driving

The host owns the event loop and the display link. The engine says when the next frame is needed and at what rate, because only the engine knows every running animation.

```rust
match engine.render(FrameTime::at(target_presentation_time))? {
    Next::Idle => link.pause(),
    Next::At { time, rate } => link.request(time, rate), // rate: RefreshRange, e.g. 60..=120
}
```

- `Next::At` is returned while any animation track of a visible surface is unsettled, or a backend source (custom GPU content, an animated shader paint) on a visible surface asked for a redraw. `time` is the frame time plus one interval at the top of `rate`; `rate` is `60..=120` while a spring, curve or fast decay runs and `30..=60` while only a decay slower than one device pixel per frame at 60 Hz remains. A curve that ends before the next frame is sampled at its endpoint on that frame and then settles.
- Animation tracks start on the first frame that samples them, so a transaction committed between frames starts at the next presentation time, never at wall-clock commit time.

## Layer tree

```rust
let card: Layer = surface.layer();               // Layer: !Clone, !Send
surface.update(|tx| {
    tx[&root].push(&card);
    tx[&card]
        .transform(Affine::translate((24.0, 80.0)))
        .clip(ContinuousRect::new(bounds, 16.0))
        .opacity(&opacity_signal)                // bound: later changes need no transaction
        .content(content);
});
drop(card);                                      // removed at the next commit
```

- **Properties:** `transform`, `opacity`, `clip`, `blend`, `filter`, `backdrop`, `scroll_offset`, `layout_size`, `content`, and child order (`push`, `insert`, `remove`). Each property accepts a constant or a nami signal. A bound signal keeps updating the layer with no further transactions. The subscription is owned by the layer and released when the layer drops.
- **Layout size.** `tx[&layer].layout_size(size)` is the size the host lays the layer out at, in its content coordinates (`Size::ZERO` until set); the host drives it from layout with a constant or a signal. It is UI-thread state that recordings read, not a render-thread property: it changes when the call is made, so a recording later in the same transaction already sees it, and `.animation(...)` does not apply to it. See Recording.
- **Binding.** `transform`, `opacity`, `scroll_offset` and `clip` take `impl Into<Live<T>>`, the same target the `Recorder` uses: any `Signal<Output = T>`, and constants are signals. Binding a property replaces that property's previous subscription. A change fires on the UI thread, is queued as the same owned op a transaction would produce, reaches the render thread with the next frame, and calls the waker. If the change's nami `Context` metadata carries an `Animation`, the op carries it too and the render thread interpolates; otherwise the value snaps. A transaction that sets the property again also replaces the binding.
- **Stable identity.** The layer handle is the identity. Content versions are internal: setting `content` bumps the version, and caching keys on (layer, version).
- **Content kinds:** recorded `Content`, a shared `Picture`, and `GpuProducer` bindings — rendered producers (custom GPU pipelines: `engine.gpu_producer(GpuContentBox)` returns a `Clone` handle owned by the one view instance) and submitted-frame producers (video, web views: `engine.frame_producer()` returns a `GpuProducer` + `FrameSink` pair; `sink.submit(frame)` installs each `ExternalFrame` and wakes the host while a binding is visible, and a frame producer has no setup). `producer.at(size)` binds a layer at the pixel size it needs; bindings of one producer across the engine's surfaces share its current frame — a rendered producer draws into a buffer from the renderer-owned frame ring, sized to the largest binding and rendered at most once per frame — and `ImageSource::Content` samples it, so there is no separate attachment path. The last drop retires the producer through the transaction stream, and `engine.drain_gpu_producers()` hands every live producer to a device replacement: a rendered producer's content re-registers and runs `setup` again, a frame producer's next submit supplies the frame on the new device.

## Animation

```rust
surface.update_animated(Spring::smooth(), |tx| {
    tx[&sheet].transform(open);
    tx[&scrim].opacity(0.4).animation(Curve::ease_out(Duration::from_millis(200)));
});
```

- **Two levels.** A transaction-wide animation applies to every property it changes, and `.animation(...)` overrides it for one property.
- **From nami.** A bound signal's change carries WaterUI's `Animation` in its `Context` metadata. The engine reads it and interpolates, so WaterUI's `.animation(...)` reaches the engine with no glue.
- **One set of animation types.** `Spring { response, damping }`, `Curve` (cubic Bézier with a duration), and `Decay { velocity, deceleration }` with optional rubber-banding. WaterUI's `Animation` becomes these types, the same way colours were unified.

  ```rust
  pub enum Animation { Spring(Spring), Curve(Curve), Decay(Decay) }   // the value nami metadata carries
  pub struct Spring { pub response: f64, pub damping: f64 }            // period in seconds; damping ratio (1 = critical)
  impl Spring { pub fn smooth() -> Self /* 0.5, 1.0 */; pub fn snappy() -> Self /* 0.5, 0.85 */; pub fn bouncy() -> Self /* 0.5, 0.7 */;
                pub fn from_physics(stiffness: f64, damping: f64) -> Self /* WaterUI's Spring, unit mass */ }
  pub struct Curve { pub p1: Point, pub p2: Point, pub duration: Duration } // control points of x(t), y(t) on [0, 1]
  impl Curve { pub fn bezier(duration, x1, y1, x2, y2) -> Self /* WaterUI's Bezier */; pub fn linear(d); pub fn ease_in(d); pub fn ease_out(d); pub fn ease_in_out(d) }
  pub struct Decay { pub velocity: Vec2, pub deceleration: f64, pub rubber_band: Option<Rect> }
  impl Decay { pub fn new(velocity: Vec2) -> Self /* deceleration 4.0 s⁻¹ */; pub fn rubber_band(self, bounds: Rect) -> Self }
  ```

- **Sampling.** A track holds the start value, the target, the animation and the time it started. `Spring` is the closed-form damped oscillator per lane (`Affine` has six lanes, `Vec2` two, `f32` one), from the start value with the start velocity; it settles when every lane is within `1e-3` of the target and slower than `1e-3` per second. `Curve` is `start + (target − start) · y(x⁻¹(t / duration))`, clamped to the endpoints. A layer track's sampling never touches recorded content: the backend draws the same fragments under a new `transform`, `opacity` or `scroll_offset`. An operand track lives in the content's `LiveState`, samples its operand's lanes the same way at frame time, and emits the frame's value as a slot update, so only the commands referencing the animated operand re-lower.
- **Retargeting.** A new value for a property with a running track starts a new track from the value *and velocity* the old track had at the last sampled frame, so a spring retargeted mid-flight is continuous in position and velocity, and a curve restarts from its current value. A change without an animation snaps and drops the track.
- **Animatable properties** are `transform`, `opacity`, `scroll_offset`, the #77 components, and the projective `tilt` and `depth`. `.animation(...)` on any other property, or `Decay` on anything but `scroll_offset`, is an invariant violation and panics.
- **Out-of-process handoff.** On promoted layers, `transform` and `opacity` animations are handed to Core Animation (Apple) or DirectComposition (Windows) whenever the curve maps exactly: springs map to `CASpringAnimation`, and Bézier curves map to `CAMediaTimingFunction`. Everything else, and everything on Android, is engine-driven.

## Scrolling

```rust
tx[&list].scroll_offset(offset);                                  // tracking a finger
tx[&list].scroll_offset(target).animation(
    Decay::new(velocity).rubber_band(content_bounds));            // fling, engine-driven
```

Scrolled content is never re-recorded. Gesture recognition stays with the host.

- `scroll_offset` translates the layer's content and children by `−offset` inside the layer's clip; `transform` is untouched.
- **Decay** starts at the value set by the transaction with `velocity` and decelerates exponentially: `x(t) = x₀ + v·(1 − e^(−k·t)) / k`, `k = deceleration` per second. With `rubber_band(bounds)`, the moment the offset leaves `bounds` the remaining motion becomes a critically damped `Spring { response: 0.4, damping: 1.0 }` from the current position and velocity back to the nearest point of `bounds`, so the overshoot and the return are one continuous motion. Without rubber-banding the decay runs until its velocity is below `1e-3` px/s.
- **Snapping.** The sampled offset is rounded to the device-pixel grid of the surface (`surface.display(Display { scale, .. })`, default `1.0`) before it reaches the backend: `round(offset · scale) / scale`. The track itself is not snapped, so a slow decay still settles smoothly.

## Recording

There are two recorders, and the difference between them is thread affinity:

```rust
/// Any thread. Constants only. Produces a frozen, shareable, Send Picture.
let icon: Picture = Picture::record(|c: &mut StaticRecorder| { /* … */ });

/// UI thread only (the surface is the proof). Accepts nami signals anywhere a value is accepted.
let content: Content = surface.record(|c: &mut Recorder| { /* … */ });
```

- **Layer recording.** `tx[&layer].record(...)` replaces the content and reuses its retired recording storage once the render thread releases it.
- **Layout size (#26).** A live recording is made for a layer and reads that layer's layout size as a signal, `c.layout_size()` (`LayoutSize: Signal<Output = Size>`): `tx[&layer].record` reads `layer`'s, `surface.record` the root layer's. Size-dependent geometry binds to it like any other value, so a host resize updates only the commands that reference it, without re-recording; a resize set under an animation (the transaction's, or a bound change's `Animation` metadata) animates those operands. A component never carries its own size binding: the host drives the layer's.

  ```rust
  surface.update(|tx| {
      tx[&chart].layout_size(layout.clone())                // the host's layout result signal
          .record(|c| {
              let size = c.layout_size();
              c.fill(size.clone().map(|s| s.to_rect()), background);
              c.stroke(size.map(|s| axis(s)), Stroke::new(1.0), ink);
          });
  });
  ```
- Recording always goes through a surface: there is no free-standing live recorder, so every recording has a layer's size to read. `Picture::record` stays free-standing because it holds no signals.

Both implement one drawing trait. A generic associated type decides what a parameter accepts:

```rust
pub trait Draw {
    /// `StaticRecorder`: `T`. `Recorder`: any `Signal<Output = T>` (constants are signals).
    type Value<T: 'static>;

    fn fill<S: Shape>(&mut self, shape: impl Into<Self::Value<S>>, paint: impl Into<Self::Value<Paint>>);
    fn stroke<S: Shape>(&mut self, shape: impl Into<Self::Value<S>>, style: impl Into<Self::Value<Stroke>>,
                        paint: impl Into<Self::Value<Paint>>);
    fn shadow<S: Shape>(&mut self, shape: impl Into<Self::Value<S>>, shadow: impl Into<Self::Value<Shadow>>);
    fn glyphs(&mut self, run: impl Into<Self::Value<GlyphRun>>, paint: impl Into<Self::Value<Paint>>);
    fn image<F: Format>(&mut self, image: &Image<F>, dst: impl Into<Self::Value<Rect>>, sampling: Sampling);
    fn picture(&mut self, picture: &Picture, transform: impl Into<Self::Value<Affine>>);

    fn clip<S: Shape>(&mut self, shape: impl Into<Self::Value<S>>, body: impl FnOnce(&mut Self));
    fn transform(&mut self, t: impl Into<Self::Value<Affine>>, body: impl FnOnce(&mut Self));
    fn group(&mut self, group: Group, body: impl FnOnce(&mut Self)); // opacity, blend, filter
}
```

- **Numeric changes.** A signal passed to `Recorder` becomes an engine-side value slot. When it changes, only the commands that reference it are regenerated, and damage is exactly those commands. A change whose nami `Context` metadata carries an `Animation` animates the slot the same way a layer property does: the engine samples the operand at frame time from the value the slot last held, `Next` schedules frames until the track settles, and only the referencing commands re-lower per frame — the per-frame cost is bounded by the animated operands, not the content size. An operand without shared lanes with its target (a colour going to a gradient, say) snaps. Structural changes re-record. A glyph run is a value slot too, so a reshaped text value is a slot update, not a re-record.
- **Shape signals.** A signal of a shape (`radius.map(|r| Circle::new(c, r))`) is how geometry becomes reactive. nami's `map` and `zip` compose it, and there is no per-field generic.
- **Paired state is closure scopes only.** There is no ambient mutable state and no push/pop.
- **nami `kurbo` feature.** nami gains a `kurbo` feature that implements constant `Signal` for kurbo types, so `impl Signal<Output = Affine>` accepts a plain `Affine`. The orphan rule prevents Cherenkov from doing this itself. Cherenkov's own types implement constant `Signal` in Cherenkov.

### Canvas (WaterUI)

Canvas and `Content` are fully unified. Canvas is a WaterUI view holding a recording closure that receives `&mut Recorder`; there is no second drawing API and no adapter.

- `DrawingState`, `save`/`restore`, the `set_*` style setters and `push_*`/`pop_layer` are removed.
- Styles are explicit parameters (`fill(shape, paint)`, `stroke(shape, style, paint)`). Transform, clip, opacity and blend are closure scopes. Reactive numbers are nami signals. Text goes through parley layouts.
- Structural changes re-run the closure, and numeric changes flow through bound signals.
- chart and mermaid do not use Canvas; they record directly.

## Shapes

```rust
pub trait Shape: 'static {
    /// What the engine may draw with a fast path. The default for kurbo shapes uses
    /// kurbo's own `as_rect` / `as_rounded_rect` / `as_circle` / `as_line`, and falls
    /// back to the path elements otherwise.
    fn semantic(&self) -> Semantic<'_>;
}

pub enum Semantic<'a> {
    Rect(Rect), RoundedRect(RoundedRect), Continuous(ContinuousRect), Circle(Circle),
    Ellipse(Ellipse), Line(Line), Path(PathRef<'a>),
}

impl<T: kurbo::Shape + 'static> Shape for T { /* … */ }
impl Shape for ContinuousRect { /* Semantic::Continuous */ }
// kurbo's Shape has no ellipse downcast, so the blanket impl recognises
// kurbo::Ellipse by type and gives it Semantic::Ellipse; no separate oval type.
```

- `ShapeData::Path` stores its elements in an `Arc` slice, so cloning the shape shares path storage.
- `ContinuousRect::to_path(tolerance)` expands its Lamé corners to a `BezPath` of line segments within `tolerance`; smoothing 0 gives circular-arc corners.
- **Custom shapes are open.** `waterui-shape` merges here, and Lyon is removed.
- **The semantic vocabulary is closed.** It is the set of fast paths. Besides the shapes above, it includes `Border` (a stroked rounded or continuous rectangle of a given width) and `InnerShadow`. These are the most common UI elements after the rounded rectangle, and otherwise they would fall to the general path route.
- **Proposal: native path type.** If profiling shows `BezPath`'s f64 storage is a bottleneck for large paths, add an engine-native f32 path type that also implements `Shape`. `BezPath` stays accepted.

## Paint and stroke

```rust
pub enum Paint {
    Solid(WorkingColor),             // colours convert to the working space when recorded
    Linear(LinearGradient), Radial(RadialGradient), Sweep(SweepGradient),
    Mesh(MeshGradient),              // Hydrolysis panics on this today
    Image(ImagePaint),               // pattern: image, transform, extend modes, sampling
    Shader(ShaderPaint),             // user WGSL fragment shader; GPU only (ShaderPaint capability)
    Transformed(TransformedPaint),   // shared paint plus independent paint-to-shape Affine
}
```

- `impl<CS: ColorSpace> From<Color<CS>> for Paint`, and likewise for each gradient type.
- **Gradients.** Stops are colours in any space. The interpolation space is a gradient property; the default is the working space, and an sRGB-encoded option exists for web compatibility.
- **Paint coordinates.** `Paint::transformed` maps paint coordinates into shape space without changing geometry, stroke width or clipping. `TransformedPaint` shares its source through `Arc<Paint>`; a live transform updates only its recorded users. Nested transforms compose outside-in, and non-finite or non-invertible transforms fail rendering. See [the #68 decision](paint-transform.md).
- **Stroke** is `kurbo::Stroke`: width, joins, caps, miter limit, dashes.
- Every join, cap and miter limit renders on every shape. The GPU backend draws round joins, and miter joins with a limit ≥ √2 on right-angle corners, analytically; other combinations are stroked to a path and rasterized by coverage.
- **Shader paints** replace `ShaderSurface`, `FlowingGradient` and `ViewEffect`. They inherit the shape, clip, antialiasing and on-chip blending, and they receive time and any signal-bound uniforms.
- **Shader paints follow a portable contract.** Inputs are explicit: coordinates, time, uniforms, declared resources and sampling footprints. Gradients are passed explicitly, and nothing relies on implicit fragment derivatives or on fragment-stage built-ins. This lets the same paint run in a fragment shader, a tile interpreter or a compute shader, so the paint contract never pre-selects the raster architecture.

## Colour

```rust
Color::<DisplayP3>::new([1.0, 0.2, 0.1, 1.0])
Color::<LinearSrgb>::new([4.0, 4.0, 4.0, 1.0])   // four times SDR white
DynColor::from_css(parsed)                         // colour space known only at run time
```

- **Typed colour spaces.** `Color<CS>` converts to the working space (linear Display P3) through a matrix that is constant-folded when monomorphised. HDR is extended values above 1.0, relative to SDR white.
- **The display supplies headroom.** Effects may read it. Output tone-maps to the display's headroom (#97): presentation scales each pixel by the extended-Reinhard/EDR shoulder evaluated at its largest channel — a per-pixel scalar, so saturated highlights keep their hue. Values in `[0, 1]` pass through; values above `1` compress smoothly towards `H` (an SDR display is `H = 1`, so highlights roll off instead of clipping). The tone map runs before the #96 gamut map. An sRGB destination's ceiling is `1` regardless of `headroom`; an extended `LinearDisplayP3` destination rolls off to the host's headroom and keeps values above 1 extended. The same curve runs in the f64 oracle (`oracle/src/tone.rs`), `present.wgsl`, and the CPU backend's `present_srgb8`.
- **sRGB output gamut-maps, never channel-clips (#96).** Linear P3 components outside `[0, 1]` go through Ottosson's analytic OKLab clip — the hue slice's cusp triangle with one Halley refinement, projecting towards the lightness axis at adaptively-chosen lightness (`ok_color.h`'s `gamut_clip_adaptive_L0_L_cusp`) — so out-of-gamut colours keep their hue and gradients stay continuous across the boundary. It was chosen over the CSS Color 4 chroma binary search (the ΔE_OK/JND spec map, which served as the measurement reference): on the boundary sweep the analytic clip held its mean ΔE_OK to the spec map at 0.0104 (p99 0.072, max 0.177 at HDR lightness the spec maps to an end colour) with hue preserved to a 7.5° maximum, while its fixed per-pixel cost measured ~3× cheaper in the present pass on lavapipe (28.5 vs 92 ms per 2752×2064 frame) — a ratio real devices confirm (#158, `docs/validation-158.md`: 2–3× slower present-pass p50 on an iPhone 16 Pro and an Apple M1 across interleaved rounds). The map also carries the spec's local-MINDE rule: where the plain clip is already within one ΔE_OK JND of the colour, the clip's bytes are kept — so in-gamut colours and P3↔sRGB round-trip ULP noise pass through bit-for-bit, yielding exactly the bytes the old convert-and-clamp produced. The same algorithm runs in the f64 oracle (`oracle/src/gamut.rs`), in `present.wgsl`, and in the CPU backend's `present_srgb8`.
- **Blending space** is linear by default. Groups can opt into sRGB-encoded blending for web compatibility (`Group::blend_space(BlendSpace::SrgbEncoded)`, #81). An encoded group's members composite with each other **in the encoded space**, and the group composites onto its backdrop in that space: the space belongs to the isolation level, so member `src_over` and the pop's blend are both encoded. (The alternative — encoding only the group-onto-backdrop composite while members stay linear, one isolation per translucent element — measured ~20× slower per frame on Metal on the overlapping-translucent corpus scene, so member-space compositing is the only semantics.) A non-semantic group (opacity 1, `normal` blend, no filter) declared in a non-linear space still isolates; a transparent clip scope declared linear composites in its parent's space.
- **Group and tree-layer isolation.** A group or tree layer composites through its own offscreen when it blends, has opacity below one, carries a filter, or has a blended descendant: a group, or for a layer a child layer or a group in its content. A blending child isolates its own layer in turn, so a layer checks direct children only. The root layer renders into the surface target and needs no offscreen. Every other group or layer composes in place with identical results.
- **WaterUI unification.** WaterUI's `ResolvedColor` becomes Cherenkov's colour type, with headroom folded into extended values.

## Text

Shaping stays outside the engine: parley, which covers complex scripts (Arabic, Indic, Thai, Hebrew and so on). The engine is responsible for everything between shaped glyphs and pixels, for every script:

```rust
let layout: TextLayout = TextLayout::new(parley_layout, |font: &parley::FontData| fonts.get(engine, font))?;
draw_text(&mut c, &layout, origin); // parley adapter: TextLayout wraps parley::Layout<Paint>
c.glyphs(&GlyphRun {
    font, size: 17.0, coords: variation_coords.into(),
    glyphs: glyphs.into(),        // id, position, and an optional per-glyph transform (vertical CJK)
    style: GlyphStyle::Fill,
}, paint);                        // paint is a separate parameter, so it can be bound to a signal
```

- **The parley adapter (#26).** `TextLayout::new(layout, font)` takes a shaped `parley::Layout<Paint>` (the engine re-exports the `parley` it lowers as `cherenkov::parley`; parley's unset brush is `Paint::default()`, opaque black) and lowers it once:
  - Every parley glyph run becomes one `GlyphRun` (font size, normalized coords, positioned glyphs) filled with the style's brush. `font` is called once per distinct font data (blob and collection index) in the layout and returns the engine `Font` it draws with; hosts cache it across layouts, since the engine keys glyph caches on the font id. `FontSource::from(&parley::FontData)` builds the registration (it copies the bytes once, like `FontSource::mapped`). The `TextLayout` keeps those `Font`s alive.
  - A synthetic oblique (fontique's `skew`) becomes each glyph's transform, `skew(−tan θ, 0)` about the glyph origin. A synthetic bold (fontique's `embolden`, for a weight heavier than any face or `wght` axis of the font offers) draws the run filled and then stroked with a mitred outline whose width is 1/24 of the em at 9 px and below, 1/32 at 36 px and above, and linear in between, so each outline grows by half that width on every side. The pair sits in an isolated group unless the brush is an opaque colour, so a translucent, gradient or image brush covers the overlap once; with an opaque colour the two composite to the same pixels without the group.
  - Underlines and strikethroughs become rectangle fills: the top edge at `baseline − offset` and the thickness from the decoration, or the run's font metrics where the style leaves them unset, across the run's advance, filled with the decoration's brush. A decoration continuing into the next run of the line with the same brush, offset and thickness is one rectangle, so a style change inside an underline leaves no seam.
  - Each line draws its underlines, then its glyphs, then its strikethroughs. Inline boxes draw nothing: their content is the host's.
  - `draw_text(&mut c, &layout, origin)` records those primitives as constants with the layout's top-left at `origin` (glyph positions add in f64 and round once to f32), through the same `glyphs`, `fill` and `group` commands as hand-built content, so both backends draw them on the existing glyph, stroked-glyph and rectangle paths. `layout.layout()` returns the parley layout for metrics and hit testing.
- `GlyphRun.glyphs` and `GlyphRun.coords` are `Arc` slices; cloning a run shares both.
- **Large scripts.** CJK text can touch thousands of distinct glyphs per screen.
  - The glyph atlas is budgeted and evicts least-recently-used pages.
  - Subpixel positions are quantized.
  - Glyph rasterization runs data-parallel with SIMD.
  - Glyphs above a size threshold are drawn as paths instead of atlas entries.
- **Colour glyphs.** COLRv0/v1 and bitmap strikes (sbix, CBDT/CBLC) are drawn natively. Emoji ZWJ sequences arrive as single glyphs from shaping.
- **Vertical text.** Glyph runs carry per-glyph transforms, so shaping can emit rotated or upright vertical glyphs.
- **Coverage correction.** Blending coverage in linear space makes text, especially thin CJK strokes, look lighter than users expect. Text coverage therefore gets a perceptual contrast and gamma correction, applied only to glyph coverage and never to geometry.
- **Font data.** Fonts are memory-mapped and never copied, which matters for Noto CJK-sized fallback chains. On `Banded`, glyph subsets are pre-rasterized into flash at build time.
- **Variable fonts** take normalized coordinates on the run.
- **Glyph realization is an experimental axis** (coverage atlas, direct curve evaluation, the path route, or distance fields for validated sizes), decided by the device farm. The CPU exact-area glyph rasterizer is both the correctness reference and the CPU backends' route. COLRv1 glyphs are a paint graph: every realization handles their transforms, gradients and compositing, and cached colour glyphs key on palette and foreground.
- **Per-glyph transforms** apply about the glyph origin, between the font scale and the glyph position. A pure translation folds into the glyph position and keeps the atlas path; any other transform is realized as outline coverage (the path route) filled with the run paint, never the atlas. A non-finite or non-invertible transform is a render error.
- **Test coverage.** The correctness corpus (#3) includes Latin, CJK (horizontal and vertical), Arabic, Hebrew, Devanagari, Thai, emoji ZWJ sequences and COLRv1 glyphs. The `text-layout-*` scenes carry a parley input (brushes, gradients, decorations, a synthetic oblique, a synthetic bold under opaque, translucent and gradient brushes, wrapping, bidirectional text over a font stack) that the Cherenkov adapters record through `draw_text`; their items are the generator's reference lowering of the same layout, which the oracle draws, in sRGB, P3-only and HDR inks.

## Effects and filters

### Repository and crates

filtrate moves into this repository, with its history, as an independent crate family: `filtrate`, `filtrate-core` and `filtrate-derive` in `filtrate/`. They keep their names, as an independently published brand, and they do not depend on the engine crates.

A shared **shader composer** crate, `cherenkov-shader` in `shader/`, is built on naga IR and used by both filtrate and Cherenkov. Every shader fragment is a naga function: primitive shading, paints, blending, backdrop sampling, YUV conversion and filter stages. Composition, inlining, specialization (constant parameters, f16/f32 and subgroup variants) and dead-code elimination all happen on the IR, so no shader text is built by string splicing. This mirrors Skia Graphite's `ShaderCodeDictionary`, where each snippet is "the ABI of an SkSL module function and its uniform data", but with one composer for the whole repository.

### Contract

1. **Stages are functions.** A colour stage is `fn(color, params) -> color`. A spatial stage is `fn(input, sampler, uv, size, params) -> color`, where `size` is the stage's input image in pixels — the executor supplies it, so the stage never queries the texture's dimensions. Shared helpers live in WGSL library modules registered with the composer; every stage's snippet references them by name and the composer imports each referenced function once per composed module.
2. **Filter kinds as types.** `ColorFilter` has a `LINEAR` property. It is necessary, but not sufficient, for pushing the filter down into each primitive's shading. `LINEAR` means a linear map on premultiplied RGBA with an identity alpha row. Only such a map commutes with src-over, \(M(a + (1-\alpha_a)b) = Ma + (1-\alpha_a)Mb\). Saturation, hue rotation, grayscale, sepia and multiplicative brightness qualify. So do offsets proportional to alpha — Brightness's `+amount·a`, Contrast's `0.5·a` pivot, Invert's `a - rgb`, a ColorMatrix bias times alpha. Those are matrix coefficients, not offsets, and commute with src-over; a constant offset is not scaled by the \(1-\alpha_a\) terms in the composition and would not commute. Anything that touches alpha otherwise, or that adds a constant offset, is not `LINEAR`: pushed down, the offset would be applied once per primitive instead of once per group. Push-down is legal only when the operation is `LINEAR` **and** every composition inside the group is premultiplied source-over, with no intermediate clamping, un-premultiplying or rounding boundary. So eligibility is a property of the operation *and* its composition context, and the engine checks both. The correctness corpus includes overlapping translucent primitives inside filtered groups to catch violations. `SpatialFilter` samples neighbours. `Chain<A, B>` is a `ColorFilter` exactly when both halves are.
3. **Footprint.** `SpatialFilter::footprint(&self) -> Footprint` is the maximum sample reach for the current parameters — an absolute pixel component plus a fraction of the image extent, which reaches like twirl or perspective report instead of claiming an unbounded reach; while a parameter animates, it is the maximum over its animation track. The executor resolves it against the actual input size. It sizes intermediates, damage expansion, backdrop regions and band or tile aprons.
4. **Working-space constants and operating space.** Luma and saturation coefficients come from the working space (linear P3) as engine-provided constants. A filter also declares the colour space it operates in. For web compatibility, CSS filter functions operate in sRGB, so the engine converts around them. Extended values (negative components, values above 1) stay extended.
5. **Shape input.** A filter can declare that it needs the clip shape's signed distance field or its mask.
6. **CPU kernels.** A filter may implement `CpuFilter` with a CPU kernel, which makes it available through `Filters` and `Runs<Raster>`; `Banded<_>` can also run filters with CPU kernels. CPU-run filters are `Send + Sync`: bands apply them in parallel. The oracle cross-checks every kernel against its shader. `RasterConfig::redraw` wakes an idle host when filter parameters change asynchronously.

### Execution

The composer produces a normalized form: segment boundaries plus the possible materialization points and their cost parameters. The executor chooses among them.

- **filtrate's thin wgpu executor** runs composed programs pass by pass. It serves UI-independent consumers, such as waterkit's export pipeline, and it is the reference for the oracle.
- **Cherenkov** chooses by where the filter sits:
  - **Colour filters.** A `LINEAR` filter is pushed down into each primitive's fragment shading before blending, so it needs no group at all. Any other colour filter is applied when its group resolves in tile memory, using on-chip programmable blending, so nothing goes back to memory. Nesting spills to a texture only when it exceeds on-chip capacity.
  - **Spatial filters.** Layer content is rendered to a transient intermediate covering the content bounds plus the footprint, and processed in fragment passes, which keeps lossless framebuffer compression. Large blurs downsample first. The final pass applies the colour suffix and blends into the parent. Keeping small-footprint spatial filters on chip with Apple tile shaders is a farm axis, not a guarantee.
  - **Backdrops.** A `BackdropGroup` defines one explicit capture point in painter order. Only members that sample the backdrop at that point can share it; an effect at a different paint-order position has a different backdrop. The group resolves its parent once, over the members' bounds plus the footprint, and the capture may be stored sparsely, so two small distant members do not force a capture and blur of the empty space between them. The spatial chain runs once, at reduced resolution where the blur contract permits. Each member samples the shared result with its own effect in its composite shader.
  - **Parameters** are value slots, which may be bound to nami signals. Changing one updates a uniform only; nothing is recompiled or re-recorded.
  - **Compilation** happens when a chain is first registered, and the driver pipeline cache is persisted.
  - **Apple tile passes.** naga cannot express imageblocks, tile render pipelines or raster order groups. On the GPU backend's Apple route, tile and imageblock passes are therefore thin native MSL scaffolding around function bodies emitted by the shared naga composer. Every shared function still comes from the composer; only the tile-pass declarations are native.
  - **CPU backends** apply pushed-down colour functions per span, run spatial kernels with footprint-wide aprons between bands, and capture backdrops into band-bounded buffers scheduled the same way: a capture plus its chain's apron is a windowed intermediate, never a full-frame buffer.

```rust
tx[&card].filter(Saturation(1.2).then(Brightness(0.9)));  // ColorFilter: fused, no extra pass
c.group(Group::new().filter(Blur::new(4.0)), |c| { /* … */ });
```

Multi-input operations (blend with an image, displacement, LUT) take `Image<F>` handles as auxiliary inputs.

## Backdrop

Requires the `Backdrop` capability for an unfiltered group; `surface.backdrop_group` requires `BackdropRuns<K, F>` for a chain `F` in kind `K`.

```rust
let glass: BackdropGroup = surface.backdrop_group(Blur::new(24.0).then(Saturation(1.8))); // SpatialFilter
tx[&toolbar].backdrop(glass.sample_with(Refraction { depth: 8.0, strength: 12.0 }));
tx[&tab_bar].backdrop(glass.sample()); // plain bilinear sample of the shared capture
```

- **One capture chain per group, stored per region.** A group owns one spatial filter chain; its capture may be stored as several disjoint regions rather than one bounding rect. Each member's bounds inflated by the filter's apron (and the member's effect reach) is integer-rounded; aproned rects that overlap or touch always merge, and farther ones merge whenever the empty space between them costs less than one capture pass's fixed overhead — distant members therefore do not capture and blur the empty space between them. A group whose filter footprint is relative to the region size (nonzero `extent`) keeps a single union region, since per-region sizes would change the result. Each member applies its own per-element effect on the shared result: a colour filter, a shader, or a filter that takes shape input. So there is one chain — and typically one region — per group, whatever the number of members.
- **Member effects.** `sample_with(effect)` accepts `Color` (a 3×4 premultiplied matrix), `Refraction { depth, strength }` (edge-following displacement), `Rim { width, color, gain }` (an additive rim light inside the member edge), or a `BackdropShaderEffect` from a registered `BackdropShader`'s `effect(uniforms)`. `sample()` stays the plain unshifted sample. An effect's sampling reach grows the group's capture region around the member; `Color` and `Rim` reach is zero.
- **Effect shaders.** `engine.backdrop_shader(BackdropShaderSource::wgsl(src).reach(px))` registers a `backdrop_effect(p, sdf, normal, size, params)` fragment — `p` is the member pixel in device space, `sdf`/`normal` the member clip's signed distance and outward normal, `size` the member's device size — which samples the shared capture through `backdrop_sample(q)`. `BackdropShader` is an RAII handle; a member still sampling a dropped shader fails the frame.
- **Capability.** Custom shaders sit behind `BackdropShaders: Backdrop` (`validate_backdrop_shader` on the caller thread, `add_backdrop_shader`/`remove_backdrop_shader` on the render thread), the same trait-per-capability shape as `Filters`/`Runs<F>`: a backend that cannot run user fragment code does not implement the trait. `Engine<B>` binds shaders only where `B: BackdropShaders`. `Color`, `Refraction` and `Rim` are built-in effects on `Backdrop` itself — no extra bound.
- **Backends.** The GPU backend implements all four effects; the CPU backend implements `Color`, `Refraction` and `Rim`, and rejects a `Shader` member effect with `Unsupported("backdrop-shader")`.
- **Handle rules.** `BackdropGroup` is an RAII, `!Send` handle. A refraction, rim or shader effect on a member whose clip is a path or mask — anything without an analytic SDF — fails with `Unsupported("backdrop-effect-sdf-path")`; `Color` works on any member.

## External content

```rust
// Video and web views: retained planes sampled in place on the shared
// device, promoted to hardware overlays when eligible.
let frame = ExternalFrame::yuv(luma, chroma, FrameColor::BT2020_PQ)?
    .sync(FrameSync::Metal { event, value });
let (video, sink) = engine.frame_producer();
sink.submit(frame);
tx[&player].content(video.at((width, height)));

// Custom GPU pipelines (particles): GPU backend only.
impl GpuContent for Particles {
    async fn setup(&mut self, gpu: &interop::wgpu::Context<'_>) { /* … */ }
    fn render(&mut self, frame: &mut interop::wgpu::Frame<'_>) { /* … */ }
}
let sparks = engine.gpu_producer(GpuContentBox::new(Particles::new(), wake_redraw));
tx[&sparks_layer].content(sparks.at((width, height)));
```

- **The engine does YUV conversion and tone mapping** for external frames when it composites them itself.
- **Custom GPU content composites like any other layer:** it can be clipped, filtered, animated and used as a backdrop source.
- **The map records `Content`, not `GpuContent`.** Each tile is a frozen `Picture` and the camera is the layer transform, so pinch-zoom and fling run in the engine. Tessellation is refreshed at the new zoom level once the gesture settles.

### Importing a foreign texture

Each backend wraps a foreign texture once, in place, into a `wgpu::Texture`
ready for `ExternalFrame::rgb` (or a YUV role it accepts): no pixel upload,
no copy, no conversion texture.

```rust
// Metal (apple): an MTLTexture — typically IOSurface- or CVPixelBuffer-
// backed — wraps through wgpu-hal; the caller keeps ownership and lifetime.
let plane = unsafe {
    interop::metal::import_texture(&device, mtl_texture, format)
};

// WebGPU (wasm32): a foreign GPUTexture wraps through
// Device::create_texture_from_webgpu_handle after a reflected contract
// check and a submission probe.
let plane = interop::web::import_texture(&device, &queue, interop::web::WebTexture {
    texture: gpu_texture,   // the producer's GPUTexture handle
    device: gpu_device,     // the GPUDevice that created it (identity token)
    release: interop::web::WebTextureLease::new(move || pool.retire(id)),
}).await?;
```

- **`interop::metal::import_texture` is `unsafe`.** The `MTLTexture` must be
  live on the same `MTLDevice` the engine wraps (or its peer group), `format`
  must be byte-compatible with its pixel format, and the texture must stay
  alive and unwritten — except by the producer — for as long as a frame
  referencing it can be in flight.
- **`interop::web::import_texture` validates the provider contract** by
  reflection — an actual `GPUTexture` (a `GPUExternalTexture` is
  `InvalidWebTexture::NotATexture`), the owning-device token
  (`DeviceMismatch`), `rgba8unorm`/`bgra8unorm`/`rgba16float`, single-sample
  2D, one mip, `TEXTURE_BINDING` (`Contract(InvalidFrame)`) — and by a
  submission probe that catches destroyed or cross-device textures
  (`Unusable`). The lease's release hook runs exactly once: at rejection,
  or when the wrapper's last clone is dropped — slot replacement, detach,
  surface or engine teardown.
- **A transient handle cannot be detected.** A context's current canvas
  texture satisfies every check but is recycled by the browser; it is
  excluded by the provider contract — immutable contents and guaranteed
  lifetime through retained and in-flight use — and must not be offered.

## Damage (invisible)

- **Damage is computed from the change set,** at three levels:
  - layer properties;
  - content replacement;
  - command level: the old and new display lists are compared, and bound-value slots contribute their commands' bounds.
- **What damage drives:**
  - partial rasterization on every backend;
  - partial present on the GPU (`VK_KHR_incremental_present`, EGL swap-with-damage);
  - partial panel transfer on `Banded` over SPI/QSPI.

## Test hooks and serialization

- **Float readback.** `Offscreen` surfaces read back in linear extended f16: `surface.readback().await`.
- **`Config::invisible_optimizations(Enabled | Disabled)`.** The oracle renders every scene twice and requires bit-identical output. Plane promotion has its own perceptual check.
- **Frame statistics.** `engine.stats()` reports GPU time, pass count, plane count and damage area for the last frame.
- **Captures.** With the `capture` feature, `surface.capture()` produces a serializable `Capture` (serde). It holds the layer tree, the contents with signal values resolved at capture time, and the resources, stored by content hash. The cross-engine suite (#3) records real Hydrolysis frames this way and replays them through every engine's adapter.

## Errors

Errors are `thiserror` enums per operation family: `EngineError`, `SurfaceError`, `ResourceError`, `RenderError` (including device loss). Invariant violations panic with a message. Nothing silently degrades.

## Retained lowering in the GPU and CPU backends

The first-party backends retain each layer's resolved command operations and
its device realizations. `DisplayList::apply` contributes normalized `Dirty`
ranges to that layer; multiple commits before a render merge their ranges.
The shared `lowering` module records each source command's operation span and
ambient content transform. A stable update replaces just those spans and
invalidates just their device instances, gradient stops and coverage. Dirty
realizations reuse their vector storage. A changed operation count, glyph
count or scope structure rebuilds the affected layer's layout.

`lowering::Content::current()` returns the prepared operations together with
their source display list only while the content is clean and prepared.
Backends use this borrowed view to inspect static capture bounds without
compiling recorded commands a second time. Pending content changes return
`None`; callers must not inspect stale operations.

Layer transforms, scrolling, clips and opacity are read from the sampled tree
while composing retained operations. They do not resolve content again.
Device placement changes regenerate the coverage that depends on that
placement, including fractional transforms; opacity-only changes reuse the
content instances and assemble the required isolation/composite passes.
Atlas generations invalidate retained GPU addresses on growth or eviction.
Deferred GPU atlas writes patch both the frame instances and retained cache
addresses before submission. Critical memory pressure releases retained device
output as well as the glyph caches.

Both backends route flattened path edges through `lowering::resolve_winding`
before signed-area accumulation: overlapping windings are resolved to boundary
edges whose winding is 0 or 1 everywhere, drawn under `NonZero`, so the
accumulator stays exact on self-overlapping outlines (stroke joins and caps,
self-intersecting fills). `None` leaves non-overlapping paths untouched, so
scenes without overlap lower identically to before.

The CPU and GPU `dirty` integration tests run the same deterministic randomized
slot updates and require exact readback bits against full lowering after every
frame. The sequence includes nested scopes, glyph-count changes, animated layer
properties, resize and cache eviction. Stable two-command updates assert
`FrameStats::commands_lowered == 2`; property-only frames assert zero.

`scenes/perf/live-dashboard` is the paired benchmark: one text value and one
bar height change on every frame of an otherwise static page. `encode` measures
the UI-thread slot changes; backend lowering runs inside `submit` with render
composition and submission. The GPU backend also reports render-thread CPU
phases, including `lower_seconds`, independently of its deferred GPU timestamps.
Submission time can include driver backpressure; use the lowering phase to
isolate CPU lowering work.


### Browser executor (#80)

On native targets the existing synchronous API and dedicated render thread
remain unchanged. Configurations, targets and custom producers must be `Send`.
On wasm32 an engine owns one serial executor on the JS thread that created it.
WebGPU handles, custom producers and filter setup futures remain on that thread.
`RenderTransfer` expresses this target-dependent requirement; it is `Send` on
native and imposes no transfer bound on wasm32. There is no unsafe `Send` shim.
Browser lowering currently runs on that same thread; no GPU-bearing payload is
sent to workers. Any future worker protocol must consist only of owned `Send`
CPU data, never resource tables or JavaScript handles.

On wasm32 `Engine::new`, `surface`, `render`, `memory`, `finish_timings`, and
`Surface::readback` are asynchronous: they wait on the device. Hosts await these
methods from their event loop. Resource registration (`font`, `image`,
`shader`, `backdrop_shader`, `Image::replace`), recording, edits, resource drops
and signal notifications are synchronous with the native signatures and enqueue
ordered work (see Resources). An operation already enqueued completes even if
its reply future is dropped. The `surface` future owns its backend id from the
moment the request is enqueued, so dropping it before the reply still destroys
the surface once the backend commits it; a rejected request destroys
nothing. Engine drop enqueues
shutdown after preceding operations; remaining handles become disconnected.
Host notifications arriving during an awaited render request the next frame.
Hosts serialize frame requests and continue honoring `Next` and the wake callback.

`GpuConfig::device` preserves the supplied adapter/device/queue on both targets,
including all enabled features. Browser initialization, shader pipeline creation,
producer/filter setup, texture readback and timing completion yield to browser
promises rather than block on channels or device polling. `cherenkov::Instant`
uses the browser performance clock on wasm32 and is `std::time::Instant` on
native. Native callers need no timestamp conversion.

### Memory measurement (#101)

Native hosts can obtain the exact device an engine would create with
`SharedDevice::create(&gpu_config)`, then pass it back through
`GpuConfig::device`. This helper wraps the engine's normal creation path,
preserving adapter selection, enabled capabilities and rendering behavior by
construction.

#### Running the browser tests

The `browser` test target in `cherenkov-gpu` executes against a real
WebGPU-enabled headless browser:

```
CHROMEDRIVER=/path/to/chromedriver \
WASM_BINDGEN_TEST_WEBDRIVER_JSON=/path/to/webdriver.json \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
cargo test --locked -p cherenkov-gpu --test browser --target wasm32-unknown-unknown
```

The `wasm-bindgen-test-runner` binary version must match the crate's
`wasm-bindgen` version, and `chromedriver` must match the installed Chrome.
`webdriver.json` supplies the browser capabilities; Chrome must launch with
WebGPU available (for headless runs, `--enable-unsafe-webgpu` plus a working
rasterizer such as `--use-angle=swiftshader`). The suite covers `!Send`
producers on the owning JS thread, `SharedDevice` reuse across engines,
a font, an image and a shader registered and drawn in one frame with no await
between, shader validation before queueing, asynchronous filter setup, host wakes requested while a
render is awaiting browser work, and incremental lowering matching full
lowering pixel-for-pixel.

### Compositor-owned property tracks (#90)

`AnimationTrack<T: Animatable>` exposes a running property's original `from`,
lane `velocity`, `target`, `animation`, and presentation-clock `start`.
`LayerNode::animations() -> Option<LayerAnimations>` returns both descriptions
together, as `transform: Option<AnimationTrack<Affine>>` and
`opacity: Option<AnimationTrack<f32>>`. `None` means that at least one track
cannot be described completely, including an unsampled track, scrolling,
nonlinear component motion, or a projective layer. `Some` with both fields
empty means that no property is moving. A sole translation component can be
represented as an affine translation track. The backend then decides whether
its native animation primitive can express the described tracks exactly.

`Renderer::owned_animations(surface) -> &[LayerId]` reports layers whose
**complete** running property animation was accepted by the last successful
presentation. Its default is empty. Owned tracks remain in the canonical
tree and are sampled before commits, so retargeting after an idle interval
preserves the current position and velocity. They do not request engine
frames. Recorded operand animations and every unowned property retain their
normal scheduling. Demotion, an unsupported track, or failed presentation
withdraws ownership.

On Apple, eligible leaf planes hand translation and opacity curves and
springs to Core Animation. The native track keeps the original presentation
clock and spring velocity. Matrix animation with changing linear coefficients,
scroll decay, and nonlinear component combinations continue to require engine
frames: Core Animation's decomposed matrix interpolation is not the engine's
coefficient interpolation. A handoff also requires eligibility throughout the
motion; sampled non-overlap with translucent content above is insufficient.
A translucent or fading moving layer cannot take ownership above an earlier
plane. Installed native motion retains its position and linear transform
through placement updates; unchanged placements cause no native transaction.

`SurfaceTree::composition_stamp(promoted)` versions the engine-composited
pixels while excluding only promoted layers' outer property stamps. Content,
clips, ordering, resource generations, surface size, clear colour and display
state remain dependencies of retained engine parts. Property-only plane
updates can consequently retain those parts on both native platforms.

`lowering::Content::current() -> Option<(&[O], &DisplayList)>` lends the
prepared operations together with the source they index. Dirty or unprepared
content returns `None`, so static-capture admission cannot inspect obsolete
operations or compile content twice. Immutable captures keep only their
native presentation buffer after publication. Output changes recapture from
the retained source; they do not keep a second engine texture alive.

### Component transform animation (#77)

Layer edits gain `translation(Live<Vec2>)`, `rotation(Live<f64>)`,
`scale(Live<Vec2>)`, `skew(Live<Vec2>)` and `pivot(Live<Vec2>)`, accepting the
same constants/signals and `.animation(...)` as the existing properties.
Their defaults are zero except scale, whose default is `(1, 1)`. The existing
`transform(Live<Affine>)` remains an independent base matrix with its existing
coefficient interpolation; it never decomposes or overwrites the components.

The sampled local matrix is
`base * translate(translation + pivot) * rotate(rotation) * skew(skew) *
scale(scale) * translate(-pivot)`. The rightmost operation acts first. Skew's
off-diagonal entries are `tan(y)` and `tan(x)`; both skew and rotation use
radians. Pivot is in the content's local coordinates. Scroll offset still
applies after this matrix, exactly as before.

Rotation is an unwrapped scalar: `0 -> pi` passes through a quarter turn,
`0 -> 2*pi` makes a full turn, and negative or multiple turns keep their
direction and winding. There is no inferred shortest path. Each property
has its own subscription and curve/spring track, sampled at presentation
time. Retargeting keeps the last sampled position and velocity. Clearing
one binding or snapping one property leaves all other components running.
Decay remains exclusive to scroll offsets. `Next` requests frames while any
component track runs and becomes idle after all tracks settle.

This is a layer capability rather than animation metadata on a recorded
matrix operand: components can bind directly to signals without a host tree
walk or re-encoding. Backend lowering sees only the sampled affine matrix.
Layers using only the existing matrix allocate no component storage.

### Projective layers (#84)

`Projective` is a checked 4×4 `f64` homogeneous transform on column
vectors, with positive Z toward the viewer. On backends with the
`ProjectiveLayers` capability (`Gpu`, `Raster`), `LayerEdit` gains four
methods:

- `projection(Live<Projective>)`: the base matrix; never animated;
- `tilt(Live<Vec2>)`: X/Y rotation in radians;
- `depth(Live<f64>)`: Z translation;
- `clear_projection()`.

The complete pose composes #77's order around them:

```text
transform · T(translation + pivot) · projection · T(0, 0, depth)
  · Rz(rotation) · Ry(tilt.y) · Rx(tilt.x) · skew · scale · T(−pivot)
```

Tilt and depth are component tracks with unwrapped angles and
velocity-preserving retargeting. The raw matrix is replaced, never
interpolated. A projective layer is a flattening boundary. Its subtree
renders into a clip-bounded, layer-local RGBA16F image at a conservative
power-of-two density, with a full area-average mip chain. The image is
projected with a bounded 16-tap anisotropic trilinear filter when the
layer composes into its parent, where opacity and blend apply once.
Clipping happens in homogeneous coordinates against `W > 0` and the
viewport, before division.

Local images are retained without the outer pose, so a matrix-only frame
realizes nothing. Invalid poses are `RenderError::ProjectivePose`, and
images beyond the dimension or byte limits are
`RenderError::ProjectiveUnsupported`. An unclipped layer, a projective
backdrop member, or a backdrop group spanning composition spaces is
`RenderError::Unsupported`. The full contract is
[`docs/projective.md`](projective.md).

### Mesh colour interpolation (#79)

`MeshGradient::interpolation(MeshColorInterpolation)` selects `Linear` (the
default) or `Smoothstep`; `interpolation_mode()` reads it. Linear preserves
the existing bilinear premultiplied-linear-P3 colour weights. Smoothstep
first replaces each recovered patch coordinate `t` by `t*t*(3-2*t)`, then
uses the same bilinear colour interpolation. At `t=0.25`, its weight is
`0.15625`. Alpha is interpolated together with premultiplied colour.

This setting changes colour weights only. Geometry, inverse branch selection,
overlap ownership, coverage, transparent samples outside all patches and
paint-coordinate transforms retain their existing contracts. It is independent
of gradient colour-space interpolation. CPU, GPU and the f64 oracle implement
both modes. Captured meshes without a setting deserialize as Linear; Linear
is omitted when serializing, preserving old captures. Live mesh operands may
switch modes while preserving unrelated retained commands and device output.

### Arbitrary silhouette shadows (#76)

`shadow(shape, Shadow)` accepts the same filled silhouette as `fill`, including
paths, ellipses and continuous corners. Offset, spread and Gaussian sigma are in
shape units, before the enclosing affine. Positive spread grows coverage with a
local disk; negative spread erodes it. The blur follows both affine axes, so a
nonuniform scale or skew also changes its covariance. The enclosing clip applies
to the completed shadow. Off-viewport shape coverage can contribute visible blur.

The GPU convolution pipeline is created with the renderer. Its intermediate
texture and kernels allocate only when needed; memory pressure drops those
allocations while preserving the pipeline, so drawing never compiles it.

GPU convolution stays on the GPU after native coverage capture. CPU convolution
uses the CPU renderer's coverage. Both retain prepared content and device output;
changing a live shadow patches its command, while unrelated commands are reused.
Existing analytic rounded-box shadows keep their established arithmetic. General
captures include a six-sigma halo and are bounded by backend address/texture limits;
an unrepresentable capture is an error, never an alternate rendering path.

### Vulkan external frames (#166)

`interop::vulkan` imports producer frames — a dmabuf or an Android
`AHardwareBuffer` — as NV12/P010 plane pairs or single RGB planes on the
engine's shared `VkDevice` and queue, zero-copy, synchronised on the GPU.

- **`Device::new(&SharedDevice)`** opens the native context for the engine's
  device and reports `Caps` — the capability record every fd, modifier,
  conversion and foreign-family claim is checked against. Missing
  capabilities are `NativeError::Unsupported`, never an emulation.
- **`Device::import(FrameSource)`** takes a `DmaBuf`/`Ahb` descriptor with
  `Wait` and `ReleaseSync` contracts and returns a `Frame`.
- **`Wait::{OpaqueFd, SyncFd, Timeline}`** names the producer fence. fd
  payloads are consumed into a binary semaphore at first use: the engine
  takes ownership of the fd (`vkImportSemaphoreFdKHR` takes it on success
  for every handle type), and the consumer waits on the GPU. A timeline
  payload carries the host's semaphore and wait point unchanged.
- **`ReleaseSync::{FenceFd, Timeline}`** names what the engine signals when
  the last retained owner retires. `FenceFd` exports a `SYNC_FD` once the
  release submission is accepted — `Frame::release_fd` hands the fence to
  the producer; before that it is `Unready`, after the frame is already
  taken it is `Invalid`.
- **`Frame::{size, repr, imported_bytes, lease, unlease, release_fd}`** is
  the #165 retained-frame contract unchanged: the engine holds the frame
  while any layer attachment references it.

Wrap-time state (recorded for #2): `create_texture_from_hal` describes the
wrapped image as `TextureUses::RESOURCE`, which maps to
`SHADER_READ_ONLY_OPTIMAL`, while the driver's actual layout at wrap time is
the producer's (`UNDEFINED`/`GENERAL`). No `TextureUses` combination maps to
`GENERAL` without also adding storage or copy usage the image does not have,
so naming the layout honestly would lie about the usage instead. The
natively recorded acquire barrier lands the real `SHADER_READ_ONLY_OPTIMAL`
transition before the first wgpu use in the same submission, so the tracked
state is never observed wrong — the discrepancy is documented rather than
hidden behind invented usage bits.

`Caps::queue_family_foreign` is true on Android even when
`VK_EXT_queue_family_foreign` is not enabled: `VK_QUEUE_FAMILY_FOREIGN_EXT`
is defined by the platform's `AHardwareBuffer` contract itself and is usable
there without the extension. Elsewhere it reports the enabled extension.

Public surface kept for the standalone Android device-test binary (recorded
here per the review on L4): `Native` and `Native::new`, `Native::staged`,
`stage_acquire`/`cancel_staged`, `Generation` with `state()`/`lease_count()`,
`State`, `PendingAcquire`/`PendingWait`, and `Frame::generation`. Everything
else on the encode path — `Release`, `Lease`, `Views`, `submit_waits`,
`mark_submitted`, `drain_releases`, `create_pool`, the framebuffer/set
caches and `KIND_*` — is `pub(crate)`; the lavapipe suite moved into the
crate for that reason.

Threading decisions (recorded for #2, L5/L6):

- The renderer itself still takes no locks. The mutexes on `Shared`,
  `Generation` and `Native` cover producer/host-thread import racing the
  render thread, staged acquire/replace on one engine, and the
  `submit_lock` that serializes a staged queue wait into exactly one
  `vkQueueSubmit`. `unsafe impl Send/Sync` on `Native` covers lease
  pointers that travel only to the submission-completion callback, never
  to another worker.
- Two engines sharing one `SharedDevice` can interleave staged waits —
  `submit_lock` is per renderer, which is the documented shape of the
  retained-frame model (one engine per `SharedDevice`).
- `Ahb` is `!Send` (a raw `AHardwareBuffer` pointer): an
  `FrameSource::Ahb` descriptor is created and imported on the producer or
  host thread, while the resulting `Frame` stays `Send`.
