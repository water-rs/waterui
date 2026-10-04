# Retained host integrations

The shared engine owns recording, live operands, layer transactions, animation sampling and `Next` scheduling. Choose a backend at build time; recording APIs do not depend on wgpu. GPU integrations live in `cherenkov_gpu::interop`. No capability silently switches backend.

## Recording and animation

`Recorder::new()` / `finish()` is the explicit lifecycle of `Content::record`. Finishing preserves subscriptions. Install the resulting `Content` on a retained layer once: a changed signal then patches its command, while unchanged device output remains cached. `ShapeData` implements `Shape` and can be passed by value without copying a path's storage.

`Content::into_picture()` freezes the latest received operand values and releases subscriptions. Use it for static, shared pictures; retaining live content is the path for paint-only updates without a host tree walk. The exported `curve_value`, `spring_step`, `decay_step` and `settled` are the same samplers used by the engine. Hosts that can express an animation through layer tracks should let the engine sample it at `FrameTime` and honor `Next`.

## Window and texture presentation

`WindowTarget` owns its window handle. The engine retains readable premultiplied linear Display P3 output in `Rgba16Float`, then converts it for the swapchain. A timed-out or occluded acquisition schedules a retry using that retained output. A transparent window requires a known premultiplied or straight compositor convention; unknown inherited alpha is insufficient. `WindowTarget::display_sync` chooses display-synchronized presentation (the default) or unsynchronized presentation for latency measurement and benchmarks; a surface that cannot present as asked is an error, never a substitute mode.

`GpuConfig::device` accepts a `SharedDevice`. Its instance, adapter, device and queue must belong to one creation chain. The device's enabled features and limits apply, and timestamps remain opt-in through `GpuConfig::timestamps`.

`TextureTarget::new(size)` returns a target and a texture notification receiver. Notifications occur on creation and resize. The host samples the engine-owned texture after rendering, on the same device and queue. Resize replaces the texture: consume the new notification before presenting the new extent. Dropping the receiver stops notifications without destroying the surface. Window and texture targets expose a positive, ordered refresh range through `rate`.

`Presenter::texture` replaces a host attachment with the retained engine output, scaling to its extent. `OutputColor::Srgb` converts P3 primaries and applies the sRGB transfer. `OutputAlpha::Premultiplied` multiplies *after* that transfer, for both ordinary and hardware-sRGB formats. `LinearDisplayP3` preserves extended values in a float attachment and requires a non-sRGB format. The source is premultiplied linear P3; producer helpers `cherenkov_srgb` and `cherenkov_premultiplied_srgb` convert browser/encoded inputs into this working space.

## Custom GPU producers

A `GpuContentBox` moves a `Send` producer onto the render thread; `Engine::gpu_producer` registers it and returns a `Clone` `GpuProducer` handle owned by the one view instance — there is no cache keyed by content. Setup receives the owning adapter, device, queue, output format and redraw requester; it runs on the first drawn binding, may run more than once on a new device each time (`Engine::drain_gpu_producers` hands live producers to a device replacement), and replaces every device resource it created before. `GpuProducer::at(size)` binds a layer at the pixel size it needs; a size change is a new binding. Every binding of a producer — on any surface of the engine, a persistent surface and a transient capture target alike — samples the producer's current frame, so there is no separate attachment path: a rendered producer draws into a buffer from the renderer-owned frame ring, sized to the componentwise largest requested size and reallocated without setup, and that buffer becomes the current frame every binding shows. The frame declares the ring's format and the working space, so decoding is the identity. The surface's compositor decides what the ring is — scan-out-capable `IOSurface` buffers deep enough for the ones the system compositor may still hold on Apple (each reused only after its release signal; when every buffer is held the frame is skipped with the redraw flag kept — the CPU never blocks), and exactly one wgpu texture where the platform has no system planes. The producer renders at most once per frame. The last handle drop retires the producer through the transaction stream. Dimensions must be nonzero and within the device limit.

## Submitted-frame producers

`Engine::frame_producer()` returns a `(GpuProducer, FrameSink)` pair — a producer whose current frame is submitted rather than rendered. `FrameSink::submit(frame)` is `Send`: it installs the frame as the producer's current frame and wakes the host while any binding is drawn on a visible surface — use it for video and web views, whose `ExternalFrame` planes the backend samples in place or promotes to hardware overlays when eligible. A frame producer has no setup: a device replacement drops its frame, and the next submit supplies one on the new device. Bindings of either producer kind are `Source::Frame` candidates for the surface's `planes::plan`, judged by the compositor's `shows` and the existing rules, and several bindings may show one buffer.

A producer writes premultiplied linear Display P3. The engine composites its current frame through the same transform, clip, opacity, blend and filter paths as recorded content. The frame is retained between updates. `Frame::elapsed` and `delta` use the host's `FrameTime`, not a private wall clock. `Frame::request_redraw()` asks for another presentation; an asynchronous `RedrawHandle` request marks the frame dirty and wakes the host once until consumed.

Detached producers retain pending requests but do not run or wake the host. Reattachment consumes their latest state. Removing content disables outstanding wake handles. Callbacks may run on producer threads, so use a thread-safe event-loop proxy. Ordinary live operands use the shared UI-thread `Engine::set_waker` contract.

## Shader paints

`Engine::shader(ShaderSource::wgsl(source))` validates and registers WGSL on the render thread. Source provides a fragment entry named `main`, receives normalized UVs at location 0, and returns premultiplied linear Display P3. The prelude exposes `uniforms.time`, `uniforms.resolution`, `params` (16 `vec4<f32>` values), and the working-space conversion helpers.

A `ShaderPaint` supplies at most 64 uniform floats. Resource keys preserve every float's exact bits, shader identity and texture extent. Retained emissions own stable keys across frames; a dirty live paint operand cannot retarget another cached draw. Shader coordinates cover complete geometry, including stroke expansion and the part outside the viewport. Collapsed axes sample the center of a one-pixel shader texture; ordinary geometry lowering still determines coverage. Ordinary non-shader paints keep their existing preparation and path cache behavior.

Static shader textures render once per key. Animated sources sample the engine presentation timeline and keep `Next` active only while used by attached output. Removing or replacing a use releases its unused textures and invalidates corresponding bindings.

## Filters and effects

The shared `Filters`, `Runs<F>` and `Effects` traits remain backend-generic. `Gpu` implements filtrate execution; custom GPU `Effect` values are wrapped in `interop::EffectBox`. CPU-kernel support can implement the same shared traits without introducing a GPU dependency into recording.

A filter captures premultiplied working-space content over the surface extent, including transparent padding, then composites the filtered result through the enclosing layer state. Input and output textures have exactly the reported extent even after shrinking a surface. Filter commands share the engine encoder, preserving ordered capture/composition and asynchronous timestamp resolution.

Callbacks are installed before lazy setup. Setup and encoding failures return `RenderError`; they do not panic the render thread. `EffectFrameTiming` carries engine presentation time and frame sequence. A shared effect consumes a nonzero animation delta only on its first use in that presentation; subsequent uses receive zero delta. Custom effects must honor the shared-encoder contract and retain resources referenced by their encoded commands.

A callback through `GpuConfig::redraw` wakes the host for an attached dirty effect; requests coalesce until consumed. Detached effects neither wake the host nor keep `Next` active. Returning `Ok(true)` from encoding schedules another presentation. No effect or producer requires a continuous host repaint when idle.

The [migration audit](migration-capabilities.md) records downstream consumers and the disposition of the old engine branch. Custom backend implementations of the shared `GpuContent` capability implement `frame_opaque`, `add_gpu_producer`, `add_frame_producer`, `bind_gpu_producer`, `submit_frame`, `retire_gpu_producer` and `drain_gpu_producers`; the GPU backend provides them.
