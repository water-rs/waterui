//! System-compositor planes on macOS (#90), through the public API a host
//! uses: an eligible external frame is realized on a display layer between
//! the engine's parts, the layer shows the frame's own `IOSurface` with the
//! colour the frame declares, ineligible frames stay in the engine, and the
//! system's composition of the realized tree matches the engine's own.
//!
//! The binary owns the process main thread: a view is main-thread state, and
//! a display layer makes its frames ready through the main queue. Every case
//! therefore runs on the main thread, one at a time.

use libtest_mimic::{Arguments, Trial};

fn main() {
    let mut args = Arguments::from_args();
    args.test_threads = Some(1);
    libtest_mimic::run(&args, trials()).exit();
}

#[cfg(not(target_os = "macos"))]
const fn trials() -> Vec<Trial> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn trials() -> Vec<Trial> {
    macos::trials()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ptr::NonNull;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use cherenkov::kurbo::{Affine, Rect, RoundedRect};
    use cherenkov::{
        Display, Draw as _, Engine, FrameTime, Layer, Offscreen, OffscreenFormat, Surface,
        WorkingColor,
    };
    use cherenkov_gpu::interop::wgpu::rwh::{
        AppKitWindowHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle,
        RawWindowHandle, WindowHandle,
    };
    use cherenkov_gpu::interop::{
        ExternalFrame, FrameColor, GpuContent, GpuContentBox, RgbAlpha, SharedDevice, YuvRange,
        metal::import_texture, wgpu,
    };
    use cherenkov_gpu::{DisplaySync, Gpu, GpuConfig, WindowTarget};
    use dispatch2::DispatchQueue;
    use libtest_mimic::Trial;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol as _, ProtocolObject};
    use objc2::{MainThreadMarker, MainThreadOnly as _};
    use objc2_app_kit::NSView;
    use objc2_av_foundation::{AVQueuedSampleBufferRenderingStatus, AVSampleBufferDisplayLayer};
    use objc2_core_foundation::{
        CFDictionary, CFRetained, CFRunLoop, CFString, CFType, CGAffineTransform, CGPoint, CGRect,
        CGSize, kCFRunLoopDefaultMode,
    };
    use objc2_core_video::{
        CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddressOfPlane,
        CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeightOfPlane,
        CVPixelBufferGetIOSurface, CVPixelBufferGetWidthOfPlane, CVPixelBufferLockBaseAddress,
        CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
        kCVImageBufferColorPrimaries_ITU_R_709_2, kCVImageBufferColorPrimariesKey,
        kCVImageBufferTransferFunction_sRGB, kCVImageBufferTransferFunctionKey,
        kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferMetalCompatibilityKey,
        kCVPixelFormatType_32BGRA, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
        kCVReturnSuccess,
    };
    use objc2_metal::{
        MTLCommandBuffer as _, MTLCommandQueue, MTLDevice, MTLPixelFormat, MTLRegion,
        MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
    };
    use objc2_quartz_core::{CALayer, CAMetalLayer, CARenderer, CATransaction};

    /// The surface in device pixels, its scale, and the video in pixels.
    const SIZE: (u32, u32) = (96, 64);
    const SCALE: f64 = 2.0;
    const VIDEO_SIZE: (u32, u32) = (48, 32);
    const VIDEO: (usize, usize) = (VIDEO_SIZE.0 as usize, VIDEO_SIZE.1 as usize);

    pub fn trials() -> Vec<Trial> {
        let case = |name: &str, run: fn()| {
            Trial::test(name, move || {
                run();
                Ok(())
            })
        };
        vec![
            case(
                "periodic_replacements_preserve_static_readmission_history",
                periodic_replacements_preserve_static_readmission_history,
            ),
            case(
                "static_pixels_are_captured_once_and_match_engine_composition",
                static_pixels_are_captured_once_and_match_engine_composition,
            ),
            case(
                "critical_trim_then_present_only_demotes_static_planes",
                || static_pixels(true),
            ),
            case(
                "promoted_translation_is_owned_by_core_animation",
                promoted_translation_is_owned_by_core_animation,
            ),
            case(
                "the_realized_tree_puts_the_plane_between_its_parts",
                the_realized_tree_puts_the_plane_between_its_parts,
            ),
            case(
                "a_promoted_frame_shows_its_own_surface_and_declared_colour",
                a_promoted_frame_shows_its_own_surface_and_declared_colour,
            ),
            case(
                "a_frame_whose_surface_disagrees_with_its_range_stays_in_the_engine",
                a_frame_whose_surface_disagrees_with_its_range_stays_in_the_engine,
            ),
            case(
                "planes_of_two_surfaces_stay_in_the_engine",
                planes_of_two_surfaces_stay_in_the_engine,
            ),
            case(
                "a_frame_without_an_iosurface_stays_in_the_engine",
                a_frame_without_an_iosurface_stays_in_the_engine,
            ),
            case(
                "only_opaque_rgb_frames_are_promoted",
                only_opaque_rgb_frames_are_promoted,
            ),
            case(
                "promoted_composition_matches_engine_composition",
                promoted_composition_matches_engine_composition,
            ),
            case(
                "a_rendered_producer_promotes_and_matches_composited",
                a_rendered_producer_promotes_and_matches_composited,
            ),
            case(
                "a_bt709_frame_stays_in_the_engine_and_matches",
                a_bt709_frame_stays_in_the_engine_and_matches,
            ),
            case(
                "a_translucent_layer_above_stays_in_the_engine_and_matches",
                a_translucent_layer_above_stays_in_the_engine_and_matches,
            ),
            case(
                "a_promoted_layer_painted_last_composes",
                a_promoted_layer_painted_last_composes,
            ),
            case(
                "two_promoted_layers_with_the_last_painted_last_compose",
                two_promoted_layers_with_the_last_painted_last_compose,
            ),
            case(
                "every_part_presents_with_the_requested_display_sync",
                every_part_presents_with_the_requested_display_sync,
            ),
        ]
    }

    /// The engine's device, shared with the test so frames live on it.
    struct Metal {
        shared: SharedDevice,
        raw: Retained<ProtocolObject<dyn MTLDevice>>,
    }

    fn metal() -> Metal {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("a Metal adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::PASSTHROUGH_SHADERS,
            ..wgpu::DeviceDescriptor::default()
        }))
        .expect("a Metal device");
        // SAFETY: the guard is dropped before the device.
        let raw = unsafe { device.as_hal::<wgpu::hal::metal::Api>() }
            .expect("a Metal device")
            .raw_device()
            .clone();
        Metal {
            shared: SharedDevice {
                instance,
                adapter,
                device,
                queue,
            },
            raw,
        }
    }

    /// A Metal-compatible, `IOSurface`-backed pixel buffer of `format`.
    fn surface_buffer(width: usize, height: usize, format: u32) -> CFRetained<CVPixelBuffer> {
        let empty = CFDictionary::<CFString, CFType>::from_slices(&[], &[]);
        // SAFETY: CoreVideo's attribute keys and the boolean are immutable
        // statics.
        let attributes = unsafe {
            CFDictionary::<CFString, CFType>::from_slices(
                &[
                    kCVPixelBufferIOSurfacePropertiesKey,
                    kCVPixelBufferMetalCompatibilityKey,
                ],
                &[
                    &empty,
                    objc2_core_foundation::kCFBooleanTrue.expect("kCFBooleanTrue"),
                ],
            )
        };
        let mut out = std::ptr::null_mut();
        // SAFETY: the attributes are a CoreVideo attribute dictionary and
        // `out` receives a +1 pixel buffer.
        let status = unsafe {
            CVPixelBufferCreate(
                None,
                width,
                height,
                format,
                Some(attributes.as_opaque()),
                NonNull::from(&mut out),
            )
        };
        assert_eq!(status, kCVReturnSuccess, "CVPixelBufferCreate");
        // SAFETY: the create call returned a +1 pixel buffer.
        unsafe { CFRetained::from_raw(NonNull::new(out).expect("a pixel buffer")) }
    }

    /// Writes each row of each of `planes` planes through `fill(plane, row)`.
    fn fill(buffer: &CVPixelBuffer, planes: usize, fill: impl Fn(usize, &mut [u8])) {
        // SAFETY: the buffer is unlocked and locked once here.
        unsafe { CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags(0)) };
        for plane in 0..planes {
            let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane);
            for y in 0..CVPixelBufferGetHeightOfPlane(buffer, plane) {
                // SAFETY: the plane is locked and `height * stride` bytes long.
                let row = unsafe { std::slice::from_raw_parts_mut(base.add(y * stride), stride) };
                fill(plane, row);
            }
        }
        // SAFETY: locked above with the same flags.
        unsafe { CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags(0)) };
    }

    /// Plane `plane` of `buffer`'s `IOSurface` as a texture on the engine's
    /// device.
    fn plane_texture(
        metal: &Metal,
        buffer: &CVPixelBuffer,
        plane: usize,
        (mtl, format): (MTLPixelFormat, wgpu::TextureFormat),
    ) -> wgpu::Texture {
        let surface = CVPixelBufferGetIOSurface(Some(buffer)).expect("an IOSurface-backed buffer");
        // SAFETY: the descriptor is fully specified.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                mtl,
                CVPixelBufferGetWidthOfPlane(buffer, plane),
                CVPixelBufferGetHeightOfPlane(buffer, plane),
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead);
        let raw = metal
            .raw
            .newTextureWithDescriptor_iosurface_plane(&descriptor, &surface, plane)
            .expect("an IOSurface plane texture");
        // SAFETY: the texture is on the engine's device and holds `format`.
        unsafe { import_texture(&metal.shared.device, raw, format) }
    }

    const LUMA: (MTLPixelFormat, wgpu::TextureFormat) =
        (MTLPixelFormat::R8Uint, wgpu::TextureFormat::R8Uint);
    const CHROMA: (MTLPixelFormat, wgpu::TextureFormat) =
        (MTLPixelFormat::RG8Uint, wgpu::TextureFormat::Rg8Uint);

    /// An 8-bit studio-range NV12 buffer: luma ramps left to right and chroma
    /// is a fixed warm tint, every pixel inside the BT.709 gamut.
    fn nv12_buffer((width, height): (usize, usize)) -> CFRetained<CVPixelBuffer> {
        let buffer = surface_buffer(
            width,
            height,
            kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
        );
        fill(&buffer, 2, |plane, row| {
            if plane == 0 {
                for (x, v) in row[..width].iter_mut().enumerate() {
                    *v = u8::try_from(48 + x * 152 / width).expect("studio luma");
                }
            } else {
                for pair in row[..width.div_ceil(2) * 2].as_chunks_mut::<2>().0 {
                    *pair = [118, 140];
                }
            }
        });
        buffer
    }

    /// The frame over `buffer`'s two planes, declared as `color`.
    fn nv12(metal: &Metal, buffer: &CVPixelBuffer, color: FrameColor) -> ExternalFrame {
        ExternalFrame::yuv(
            plane_texture(metal, buffer, 0, LUMA),
            plane_texture(metal, buffer, 1, CHROMA),
            color,
        )
        .expect("a valid NV12 frame")
    }

    /// A view that is never put in a window, as a window handle.
    struct View(dispatch2::MainThreadBound<Retained<NSView>>);

    impl HasWindowHandle for View {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(
                NonNull::from(
                    &**self
                        .0
                        .get(MainThreadMarker::new().expect("window capture on main")),
                )
                .cast(),
            ));
            // SAFETY: the view outlives the handle.
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    impl HasDisplayHandle for View {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::appkit())
        }
    }

    fn sublayers(layer: &CALayer) -> Vec<Retained<CALayer>> {
        // SAFETY: the sublayers array is read on the thread that owns it.
        unsafe { layer.sublayers() }.map_or_else(Vec::new, |a| a.to_vec())
    }

    /// Whether `display` is a candidate's probe — parked beside the
    /// engine root at zero bounds while its readiness is reported —
    /// rather than a promoted plane. `attach` names it
    /// `cherenkov-pending`; `place` clears the name on promotion.
    fn probing(display: &AVSampleBufferDisplayLayer) -> bool {
        display
            .name()
            .is_some_and(|name| *name == *objc2_foundation::ns_string!("cherenkov-pending"))
    }

    /// The display layers under `layer` matching `keep`.
    fn displays_matching(
        layer: &CALayer,
        keep: &dyn Fn(&AVSampleBufferDisplayLayer) -> bool,
    ) -> Vec<Retained<AVSampleBufferDisplayLayer>> {
        sublayers(layer)
            .into_iter()
            .flat_map(|l| match l.downcast::<AVSampleBufferDisplayLayer>() {
                Ok(display) => {
                    if keep(&display) {
                        vec![display]
                    } else {
                        Vec::new()
                    }
                }
                Err(l) => displays_matching(&l, keep),
            })
            .collect()
    }

    /// Every promoted display layer under `layer`.
    fn displays(layer: &CALayer) -> Vec<Retained<AVSampleBufferDisplayLayer>> {
        displays_matching(layer, &|display| !probing(display))
    }

    /// Every probing display layer under `layer`.
    fn probes(layer: &CALayer) -> Vec<Retained<AVSampleBufferDisplayLayer>> {
        displays_matching(layer, &probing)
    }

    /// Drives the main run loop until `done`, or fails once `deadline`
    /// passes.
    fn drive(deadline: Instant, done: &dyn Fn() -> bool, what: &dyn Fn() -> String) {
        // SAFETY: the mode is an immutable static.
        let mode = unsafe { kCFRunLoopDefaultMode };
        while !done() {
            assert!(Instant::now() < deadline, "{}", what());
            CFRunLoop::run_in_mode(mode, 0.005, true);
        }
    }

    /// Drives the main run loop until `flag` is set — the completion
    /// signal a queued block, like a display layer's attach, leaves
    /// behind.
    fn settle_flag(flag: &AtomicBool, what: &str) {
        drive(
            Instant::now() + Duration::from_secs(10),
            &|| flag.load(Ordering::Acquire),
            &|| what.into(),
        );
    }

    /// Commits this thread's implicit transaction, then drives the main run
    /// loop until every display layer has its first frame ready and has laid
    /// the frame out.
    ///
    /// A display layer reports ready before the layout of its video sublayer
    /// has run: that layout is queued on the main queue first, so a block
    /// queued behind the readiness runs after it.
    fn settle(layer: &CALayer) {
        CATransaction::flush();
        let deadline = Instant::now() + Duration::from_secs(10);
        for display in displays(layer) {
            drive(
                deadline,
                // SAFETY: the display layer is read on the main thread.
                &|| unsafe { display.isReadyForDisplay() },
                &|| {
                    // SAFETY: as above.
                    let r = unsafe { display.sampleBufferRenderer() };
                    format!(
                        "the display layer never became ready: {:?} error={:?} \
                        bounds={:?} hidden={:?}",
                        unsafe { r.status() },
                        unsafe { r.error() },
                        display.bounds(),
                        display.isHidden(),
                    )
                },
            );
        }
        drain_main();
        CATransaction::flush();
    }

    fn drain_main() {
        let drained = Arc::new(AtomicBool::new(false));
        let mark = Arc::clone(&drained);
        DispatchQueue::main().exec_async(move || mark.store(true, Ordering::Release));
        drive(
            Instant::now() + Duration::from_secs(10),
            &|| drained.load(Ordering::Acquire),
            &|| "the main queue never drained".into(),
        );
        CATransaction::flush();
    }

    /// Core Animation's own renderer drawing a layer tree into an extended
    /// sRGB texture, standing in for the window server.
    ///
    /// The target is sRGB because the engine's window parts present
    /// sRGB-encoded content with a layer's default colour space, which Core
    /// Animation reads in the target's space; frames on planes carry their
    /// own and are converted.
    struct SystemCompositor {
        renderer: Retained<CARenderer>,
        target: Retained<ProtocolObject<dyn MTLTexture>>,
        queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
        stage: Retained<CALayer>,
    }

    impl SystemCompositor {
        /// Stages `host`, sized in points, at the surface's scale so one unit
        /// of the renderer's bounds is one pixel, and commits it.
        ///
        /// Core Animation sends a renderer only what is committed after it is
        /// attached, so this runs before the engine builds its layers.
        fn attach(metal: &Metal, host: &CALayer) -> Self {
            // SAFETY: the descriptor is fully specified.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    MTLPixelFormat::RGBA16Float,
                    SIZE.0 as usize,
                    SIZE.1 as usize,
                    false,
                )
            };
            descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
            descriptor.setStorageMode(MTLStorageMode::Shared);
            let target = metal
                .raw
                .newTextureWithDescriptor(&descriptor)
                .expect("composite target");
            let queue = metal.raw.newCommandQueue().expect("queue");
            // SAFETY: the name is an immutable static.
            let space = objc2_core_graphics::CGColorSpace::with_name(Some(unsafe {
                objc2_core_graphics::kCGColorSpaceExtendedSRGB
            }))
            .expect("extended sRGB");
            // SAFETY: a `CGColorSpace` is toll-free bridged to an Objective-C
            // object.
            let space: &AnyObject = unsafe { &*CFRetained::as_ptr(&space).as_ptr().cast() };
            let queue_object: &AnyObject = AsRef::<AnyObject>::as_ref(&*queue);
            // SAFETY: the option keys are immutable statics.
            let keys = unsafe {
                [
                    objc2_quartz_core::kCARendererColorSpace,
                    objc2_quartz_core::kCARendererMetalCommandQueue,
                ]
            };
            let options = objc2_foundation::NSDictionary::<
                objc2_foundation::NSString,
                AnyObject,
            >::from_slices(&keys, &[space, queue_object]);
            // SAFETY: a string key is an object key; the target outlives the
            // renderer.
            let renderer = unsafe {
                CARenderer::rendererWithMTLTexture_options(&target, Some(options.cast_unchecked()))
            };
            let pixels = CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(f64::from(SIZE.0), f64::from(SIZE.1)),
            );
            // The view's layer keeps the geometry AppKit gives it; a layer
            // between it and the stage scales points to pixels.
            let stage = CALayer::new();
            stage.setBounds(pixels);
            stage.setAnchorPoint(CGPoint::new(0.0, 0.0));
            stage.setPosition(CGPoint::new(0.0, 0.0));
            let points = CALayer::new();
            points.setBounds(host.frame());
            points.setAnchorPoint(CGPoint::new(0.0, 0.0));
            points.setPosition(CGPoint::new(0.0, 0.0));
            points.setAffineTransform(CGAffineTransform {
                a: SCALE,
                b: 0.0,
                c: 0.0,
                d: SCALE,
                tx: 0.0,
                ty: 0.0,
            });
            stage.addSublayer(&points);
            points.addSublayer(host);
            renderer.setLayer(Some(&stage));
            renderer.setBounds(pixels);
            CATransaction::flush();
            Self {
                renderer,
                target,
                queue,
                stage,
            }
        }

        /// Composites what is committed, returning premultiplied linear
        /// Display P3 pixels, row 0 at the top of the screen.
        ///
        /// The renderer writes layer space bottom-up (row 0 is `y = 0`, the
        /// bottom of an unflipped layer), so rows are reversed.
        fn composite(&self) -> Vec<[f32; 4]> {
            settle(&self.stage);
            let bounds = self.renderer.bounds();
            // SAFETY: a null timestamp is allowed.
            unsafe {
                self.renderer.beginFrameAtTime_timeStamp(
                    objc2_quartz_core::CACurrentMediaTime(),
                    std::ptr::null_mut(),
                );
            }
            self.renderer.addUpdateRect(bounds);
            self.renderer.render();
            self.renderer.endFrame();
            let fence = self.queue.commandBuffer().expect("fence");
            fence.commit();
            fence.waitUntilCompleted();
            let mut halves = vec![0u16; SIZE.0 as usize * SIZE.1 as usize * 4];
            // SAFETY: `halves` holds the whole RGBA16Float target.
            unsafe {
                self.target.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                    NonNull::new(halves.as_mut_ptr().cast()).expect("bytes"),
                    SIZE.0 as usize * 8,
                    MTLRegion {
                        origin: objc2_metal::MTLOrigin { x: 0, y: 0, z: 0 },
                        size: objc2_metal::MTLSize {
                            width: SIZE.0 as usize,
                            height: SIZE.1 as usize,
                            depth: 1,
                        },
                    },
                    0,
                );
            }
            let decode = |c: f64| {
                let linear = if c.abs() <= 0.040_45 {
                    c.abs() / 12.92
                } else {
                    ((c.abs() + 0.055) / 1.055).powf(2.4)
                };
                linear.copysign(c)
            };
            let pixels: Vec<[f32; 4]> = halves
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| {
                    let [r, g, b, a] = p.map(|h| f64::from(half::f16::from_bits(h).to_f32()));
                    let straight = [r, g, b].map(|c| if a > 0.0 { decode(c / a) } else { 0.0 });
                    let [r, g, b] = cherenkov_oracle::color::linear_srgb_to_linear_p3(straight);
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "the target holds half floats"
                    )]
                    [r * a, g * a, b * a, a].map(|c| c as f32)
                })
                .collect();
            pixels
                .as_chunks::<{ SIZE.0 as usize }>()
                .0
                .iter()
                .rev()
                .flatten()
                .copied()
                .collect()
        }
    }

    /// An engine with a window surface over a view that is never shown, the
    /// view's layer staged for the system compositor.
    struct Fixture {
        metal: Metal,
        engine: Engine<Gpu>,
        window: Surface<Gpu>,
        system: SystemCompositor,
        view: Retained<NSView>,
        /// Set by the engine's wake callback: an attach landing on the
        /// main queue asks for the frame that promotes its candidate.
        woke: Arc<AtomicBool>,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with(|target| target)
        }

        /// The fixture over the window target `configure` returns.
        fn with(configure: impl FnOnce(WindowTarget) -> WindowTarget) -> Self {
            let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
            let metal = metal();
            let engine = Engine::<Gpu>::new(GpuConfig {
                device: Some(metal.shared.clone()),
                ..GpuConfig::default()
            })
            .expect("an engine");
            let view = NSView::initWithFrame(
                NSView::alloc(mtm),
                CGRect::new(
                    CGPoint::new(0.0, 0.0),
                    CGSize::new(f64::from(SIZE.0) / SCALE, f64::from(SIZE.1) / SCALE),
                ),
            );
            view.setWantsLayer(true);
            let host = view.layer().expect("a layer-backed view");
            host.setContentsScale(SCALE);
            let system = SystemCompositor::attach(&metal, &host);
            let window = engine
                .surface(configure(WindowTarget::new(
                    View(dispatch2::MainThreadBound::new(view.clone(), mtm)),
                    SIZE,
                )))
                .expect("a window surface");
            window
                .display(Display {
                    scale: SCALE,
                    headroom: 1.0,
                })
                .expect("the display");
            let woke = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&woke);
            engine.set_waker(move || flag.store(true, Ordering::Release));
            Self {
                metal,
                engine,
                window,
                system,
                view,
                woke,
            }
        }

        fn host(&self) -> Retained<CALayer> {
            self.view.layer().expect("a layer-backed view")
        }

        /// The engine's root layer under the host — selected by its name:
        /// a candidate's pending display layer attaches beside it.
        fn root(&self) -> Retained<CALayer> {
            let layers: Vec<_> = sublayers(&self.host())
                .into_iter()
                .filter(|layer| {
                    layer
                        .name()
                        .is_some_and(|name| *name == *objc2_foundation::ns_string!("cherenkov"))
                })
                .collect();
            let [root] = &layers[..] else {
                panic!("one engine root under the host, found {}", layers.len());
            };
            root.clone()
        }

        fn render(&self) {
            drain_main();
            self.engine.render(FrameTime::now()).expect("rendered");
            drain_main();
            settle(&self.host());
        }

        fn refusal_cycle(&self) {
            self.render();
            // The queue barrier observes every attach that the first render
            // submitted, even if the tested refusal prevents any attach.
            // The second frame exercises eligibility after readiness.
            self.render();
        }

        /// The frames a window produces while a candidate is attached and
        /// promoted: the first render composites it in-engine —
        /// asserted, the pending contract — the attach block's completion
        /// wake is the drain's done signal, and the probe's own
        /// `readyForDisplay` is the platform's report. `false` means the
        /// platform never reported it; once it does, promotion is owed —
        /// a ready probe left in the engine is a planner rejection of an
        /// eligible scene, which fails here rather than skipping.
        fn promote(&self) -> bool {
            drain_main();
            self.woke.store(false, Ordering::Relaxed);
            self.engine.render(FrameTime::now()).expect("rendered");
            assert!(
                displays(&self.root()).is_empty(),
                "the pending candidate stays engine-composited"
            );
            // The parts reply queued ahead of the attach can claim the
            // first wake: attach is done once its probe sits beside the
            // root — the hierarchy state `readyForDisplay` requires.
            settle_flag(&self.woke, "the queued attach never completed");
            drive(
                Instant::now() + Duration::from_secs(10),
                &|| !probes(&self.host()).is_empty(),
                &|| "the queued attach never parked a probe".into(),
            );
            // `readyForDisplay` posts its change notification, whose
            // handler re-reads the flag on main and wakes the loop. A
            // renderer that fails asynchronously reports it through a
            // failed status — the concrete "cannot show" (a synchronous
            // rejection already panics in `show`); the deadline is only
            // ever a test failure, never the answer.
            loop {
                if self.probe_failed() {
                    return false;
                }
                // Clear `woke` before the render: a bounce landing while
                // the waker was disarmed has still stored the flag this
                // render's `prepare` reads, and a bounce landing on the
                // armed waker leaves the callback's flag — no signal
                // from the notification is lost either way.
                self.woke.store(false, Ordering::Relaxed);
                drain_main();
                self.engine.render(FrameTime::now()).expect("rendered");
                drain_main();
                if !displays(&self.root()).is_empty() {
                    break;
                }
                drive(
                    Instant::now() + Duration::from_secs(10),
                    &|| self.woke.load(Ordering::Acquire) || self.probe_failed(),
                    &|| "the readiness signal never arrived".into(),
                );
                if self.probe_failed() {
                    return false;
                }
                // The wake means the flag was just re-evaluated; the
                // render reading it must promote — anything less with a
                // ready probe is a planner rejection of an eligible
                // scene; a not-ready probe's wake was a down-flip and
                // the loop waits for the next evaluation.
                drain_main();
                self.engine.render(FrameTime::now()).expect("rendered");
                drain_main();
                if !displays(&self.root()).is_empty() {
                    break;
                }
                assert!(
                    !probes(&self.host())
                        .iter()
                        .all(|d| unsafe { d.isReadyForDisplay() }),
                    "the probe reported ready but the plan kept the frame \
                    in the engine"
                );
            }
            settle(&self.host());
            true
        }

        /// Whether a parked probe's renderer failed asynchronously —
        /// the platform's "cannot show this"; a synchronous rejection at
        /// the enqueue is instead a `show` panic.
        fn probe_failed(&self) -> bool {
            probes(&self.host()).iter().any(|d| unsafe {
                d.sampleBufferRenderer().status()
            } == AVQueuedSampleBufferRenderingStatus::Failed)
        }
    }

    /// A full-surface backdrop, a holder translated into the surface with a
    /// rounded clip and the video scaled into it, and a translucent control
    /// bar painted above the video. The layers live as long as the handles.
    #[must_use = "dropping the handles removes the layers"]
    fn scene(engine: &Engine<Gpu>, surface: &Surface<Gpu>, frame: ExternalFrame) -> [Layer; 4] {
        scene_bar(engine, surface, frame, 0.5)
    }

    /// `scene` with the control bar's alpha: `bar_alpha` 1.0 makes it
    /// opaque — the composite then has no translucency for the platform
    /// to resolve differently.
    fn scene_bar(
        engine: &Engine<Gpu>,
        surface: &Surface<Gpu>,
        frame: ExternalFrame,
        bar_alpha: f32,
    ) -> [Layer; 4] {
        let below = surface.layer();
        let holder = surface.layer();
        let player = surface.layer();
        let above = surface.layer();
        let backdrop = surface.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 96.0, 64.0),
                WorkingColor::new([0.1, 0.3, 0.6, 1.0]),
            );
        });
        let bar_rect = if bar_alpha >= 1.0 {
            Rect::new(8.0, 58.0, 88.0, 64.0)
        } else {
            Rect::new(8.0, 44.0, 88.0, 58.0)
        };
        let bar = surface.record(|c| {
            c.fill(bar_rect, WorkingColor::new([0.5, 0.5, 0.5, bar_alpha]));
        });
        let (video, sink) = engine.frame_producer();
        sink.submit(frame);
        surface.update(|tx| {
            tx[surface.root()].push(&below).push(&holder).push(&above);
            tx[&below].content(backdrop);
            tx[&holder]
                .push(&player)
                .transform(Affine::translate((12.0, 8.0)))
                .clip(RoundedRect::new(0.0, 0.0, 72.0, 48.0, 6.0));
            tx[&player]
                .transform(Affine::scale(1.5))
                .content(video.at(VIDEO_SIZE));
            tx[&above].content(bar).clip(bar_rect);
        });
        [below, holder, player, above]
    }

    fn is<T: objc2::ClassType>(layer: &CALayer) -> bool {
        layer.isKindOfClass(T::class())
    }

    /// The layers directly under the engine root: parts and planes in paint
    /// order.
    fn stack(fixture: &Fixture) -> Vec<Retained<CALayer>> {
        sublayers(&fixture.root())
    }

    /// Every part's metal layer is configured with the present mode the
    /// window's `DisplaySync` resolves to on this Mac — the first part and
    /// the parts a promoted plane splits off above it. wgpu's Metal
    /// backend advertises FIFO and immediate on macOS and realizes them
    /// as the layer's `displaySyncEnabled` (#214).
    fn every_part_presents_with_the_requested_display_sync() {
        for (sync, display_sync) in [
            (DisplaySync::Synchronized, true),
            (DisplaySync::Unsynchronized, false),
        ] {
            let fixture = Fixture::with(|target| target.display_sync(sync));
            let buffer = bgra_buffer();
            let _scene = scene_bar(
                &fixture.engine,
                &fixture.window,
                bgra(&fixture.metal, &buffer, FrameColor::SRGB),
                1.0,
            );
            // Two parts when the platform promotes the frame, one when it
            // keeps it in the engine; either way every part is checked.
            let parts_expected = if fixture.promote() { 2 } else { 1 };
            let parts: Vec<_> = stack(&fixture)
                .into_iter()
                .filter_map(|layer| layer.downcast::<CAMetalLayer>().ok())
                .collect();
            assert_eq!(parts.len(), parts_expected, "{sync:?}: the window's parts");
            for part in parts {
                assert_eq!(
                    part.displaySyncEnabled(),
                    display_sync,
                    "{sync:?}: a part's display sync"
                );
            }
        }
    }

    /// The plane sits between the part painted below it and the part
    /// painted above it, nested in one layer per tree level, with the level's
    /// transform, rounded clip, and the frame's size and opacity.
    fn the_realized_tree_puts_the_plane_between_its_parts() {
        let fixture = Fixture::new();
        let buffer = bgra_buffer();
        let offscreen = fixture
            .engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16))
            .expect("offscreen");
        let _engine_scene = scene_bar(
            &fixture.engine,
            &offscreen,
            bgra(&fixture.metal, &buffer, FrameColor::SRGB),
            1.0,
        );
        let _window_scene = scene_bar(
            &fixture.engine,
            &fixture.window,
            bgra(&fixture.metal, &buffer, FrameColor::SRGB),
            1.0,
        );
        if !fixture.promote() {
            // The platform never reported the probe ready: the window
            // then shows the engine's own composition, still verified.
            engine_parity(&fixture, &offscreen, "engine-composited frame");
            return;
        }

        let root = fixture.root();
        assert!(root.isGeometryFlipped(), "engine space is y-down");
        let stack = stack(&fixture);
        assert_eq!(stack.len(), 3, "part, plane, part");
        assert!(is::<CAMetalLayer>(&stack[0]) && is::<CAMetalLayer>(&stack[2]));
        assert!(!is::<CAMetalLayer>(&stack[1]));
        // Pixel space, then root → holder → player, each transform → [clip]
        // → scroll.
        let top = &stack[1];
        let t = top.affineTransform();
        assert!((t.a - 1.0 / SCALE).abs() < 1e-12 && (t.d - 1.0 / SCALE).abs() < 1e-12);
        let root_node = &sublayers(top)[0];
        let root_scroll = &sublayers(root_node)[0];
        let holder_node = &sublayers(root_scroll)[0];
        assert!((holder_node.affineTransform().tx - 12.0).abs() < 1e-12);
        let holder_clip = &sublayers(holder_node)[0];
        assert!(holder_clip.masksToBounds());
        assert!((holder_clip.cornerRadius() - 6.0).abs() < 1e-12);
        let holder_scroll = &sublayers(holder_clip)[0];
        let player_node = &sublayers(holder_scroll)[0];
        assert!((player_node.affineTransform().a - 1.5).abs() < 1e-12);
        let player_scroll = &sublayers(player_node)[0];
        let [display] = &sublayers(player_scroll)[..] else {
            panic!("one display layer");
        };
        assert!(is::<AVSampleBufferDisplayLayer>(display));
        let bounds = display.bounds();
        assert_eq!((bounds.size.width, bounds.size.height), (48.0, 32.0));
        assert!(
            (display.opacity() - 1.0).abs() < f32::EPSILON,
            "the player's opacity"
        );
    }

    fn same(buffer: &CVPixelBuffer, key: &CFString, expected: &CFString) -> bool {
        // SAFETY: the attachment is read, not retained past the buffer.
        unsafe { buffer.attachment(key, std::ptr::null_mut()) }
            .is_some_and(|v| v.downcast_ref::<CFString>().is_some_and(|s| s == expected))
    }

    /// Admitted BGRA/sRGB retains the source `IOSurface` and colour tags.
    fn a_promoted_frame_shows_its_own_surface_and_declared_colour() {
        let fixture = Fixture::new();
        let buffer = bgra_buffer();
        let offscreen = fixture
            .engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16))
            .expect("offscreen");
        let _engine_scene = scene_bar(
            &fixture.engine,
            &offscreen,
            bgra(&fixture.metal, &buffer, FrameColor::SRGB),
            1.0,
        );
        let _window_scene = scene_bar(
            &fixture.engine,
            &fixture.window,
            bgra(&fixture.metal, &buffer, FrameColor::SRGB),
            1.0,
        );
        if !fixture.promote() {
            // The platform never reported the probe ready: the window
            // then shows the engine's own composition, still verified.
            engine_parity(&fixture, &offscreen, "engine-composited frame");
            return;
        }
        let _ = fixture.system.composite();
        let [display] = &displays(&fixture.root())[..] else {
            panic!("one display");
        };
        // SAFETY: renderer and buffer attachments are read on main.
        let shown = unsafe { display.sampleBufferRenderer().copyDisplayedPixelBuffer() }
            .expect("displayed buffer");
        assert_eq!(
            CVPixelBufferGetIOSurface(Some(&buffer))
                .expect("source")
                .id(),
            CVPixelBufferGetIOSurface(Some(&shown))
                .expect("displayed")
                .id()
        );
        unsafe {
            assert!(same(
                &shown,
                kCVImageBufferColorPrimariesKey,
                kCVImageBufferColorPrimaries_ITU_R_709_2
            ));
            assert!(same(
                &shown,
                kCVImageBufferTransferFunctionKey,
                kCVImageBufferTransferFunction_sRGB
            ));
        }
    }

    fn bgra_buffer() -> CFRetained<CVPixelBuffer> {
        let buffer = surface_buffer(VIDEO.0, VIDEO.1, kCVPixelFormatType_32BGRA);
        fill(&buffer, 1, |_, row| {
            for pixel in row[..VIDEO.0 * 4].as_chunks_mut::<4>().0 {
                *pixel = [64, 96, 128, 255];
            }
        });
        buffer
    }

    fn bgra(metal: &Metal, buffer: &CVPixelBuffer, color: FrameColor) -> ExternalFrame {
        ExternalFrame::rgb(
            plane_texture(
                metal,
                buffer,
                0,
                (MTLPixelFormat::BGRA8Unorm, wgpu::TextureFormat::Bgra8Unorm),
            ),
            RgbAlpha::Opaque,
            color,
        )
        .expect("valid BGRA")
    }

    /// Renders `frame` in the scene and asserts the engine composed it
    /// itself: one part, no plane.
    fn stays_in_the_engine(fixture: &Fixture, frame: ExternalFrame) {
        let _scene = scene_bar(&fixture.engine, &fixture.window, frame, 1.0);
        fixture.refusal_cycle();
        let stack = stack(fixture);
        assert_eq!(stack.len(), 1, "one part");
        assert!(is::<CAMetalLayer>(&stack[0]));
        assert!(displays(&fixture.root()).is_empty(), "no plane");
    }

    /// A studio-range surface declared full-range would be decoded
    /// differently by the system than by the engine.
    fn a_frame_whose_surface_disagrees_with_its_range_stays_in_the_engine() {
        let fixture = Fixture::new();
        let buffer = nv12_buffer(VIDEO);
        let full = FrameColor {
            range: YuvRange::Full,
            ..FrameColor::BT2020_PQ
        };
        let frame = nv12(&fixture.metal, &buffer, full);
        stays_in_the_engine(&fixture, frame);
    }

    fn planes_of_two_surfaces_stay_in_the_engine() {
        let fixture = Fixture::new();
        let one = nv12_buffer(VIDEO);
        let two = nv12_buffer(VIDEO);
        let frame = ExternalFrame::yuv(
            plane_texture(&fixture.metal, &one, 0, LUMA),
            plane_texture(&fixture.metal, &two, 1, CHROMA),
            FrameColor::BT2020_PQ,
        )
        .expect("valid");
        stays_in_the_engine(&fixture, frame);
    }

    fn a_frame_without_an_iosurface_stays_in_the_engine() {
        let fixture = Fixture::new();
        let plane = |format, (width, height): (usize, usize)| {
            fixture
                .metal
                .shared
                .device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: u32::try_from(width).expect("small"),
                        height: u32::try_from(height).expect("small"),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
        };
        let frame = ExternalFrame::yuv(
            plane(wgpu::TextureFormat::R8Uint, VIDEO),
            plane(wgpu::TextureFormat::Rg8Uint, (VIDEO.0 / 2, VIDEO.1 / 2)),
            FrameColor::BT709_VIDEO,
        )
        .expect("valid");
        stays_in_the_engine(&fixture, frame);
    }

    /// A display layer shows opaque video; alpha stays with the engine.
    fn only_opaque_rgb_frames_are_promoted() {
        let bgra = |fixture: &Fixture, alpha| {
            let buffer = surface_buffer(VIDEO.0, VIDEO.1, kCVPixelFormatType_32BGRA);
            let plane = plane_texture(
                &fixture.metal,
                &buffer,
                0,
                (MTLPixelFormat::BGRA8Unorm, wgpu::TextureFormat::Bgra8Unorm),
            );
            ExternalFrame::rgb(plane, alpha, FrameColor::SRGB).expect("valid")
        };
        let straight = Fixture::new();
        stays_in_the_engine(&straight, bgra(&straight, RgbAlpha::Straight));
        let opaque = Fixture::new();
        let offscreen = opaque
            .engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16))
            .expect("offscreen");
        let _engine_scene = scene_bar(
            &opaque.engine,
            &offscreen,
            bgra(&opaque, RgbAlpha::Opaque),
            1.0,
        );
        let _window_scene = scene_bar(
            &opaque.engine,
            &opaque.window,
            bgra(&opaque, RgbAlpha::Opaque),
            1.0,
        );
        if opaque.promote() {
            assert_eq!(displays(&opaque.root()).len(), 1, "promoted");
        } else {
            // The platform never reported the probe ready: the window
            // then shows the engine's own composition, still verified.
            engine_parity(&opaque, &offscreen, "engine-composited opaque frame");
        }
    }

    fn periodic_replacements_preserve_static_readmission_history() {
        let fixture = Fixture::new();
        let layer = fixture.window.layer();
        fixture.window.update(|tx| {
            tx[fixture.window.root()].push(&layer);
        });
        for cycle in 0..4 {
            let pixels = fixture.window.record(|c| {
                c.fill(Rect::new(8., 6., 80., 54.), WorkingColor::WHITE);
            });
            fixture.window.update(|tx| {
                tx[&layer].content(pixels);
            });
            fixture.render();
            for offset in 1..=2 {
                fixture.window.update(|tx| {
                    tx[&layer].transform(Affine::translate((f64::from(cycle * 3 + offset), 0.)));
                });
                fixture.render();
                if cycle > 0 {
                    assert_eq!(
                        fixture.engine.stats().passes,
                        1,
                        "a periodic edit must stay in one engine pass, without a capture pass"
                    );
                }
            }
        }
    }

    fn static_pixels_are_captured_once_and_match_engine_composition() {
        static_pixels(false);
    }

    fn static_headroom_completion(fixture: &Fixture) {
        fixture.woke.store(false, Ordering::Release);
        fixture
            .window
            .display(Display {
                scale: SCALE,
                headroom: 2.0,
            })
            .expect("new output headroom");
        assert_eq!(
            fixture.engine.render(FrameTime::now()).expect("recapture"),
            cherenkov::Next::Idle
        );
        // No render, readback, queue submission or device poll while waiting:
        // the conversion itself must wake an otherwise idle engine.
        drive(
            Instant::now() + Duration::from_secs(10),
            &|| fixture.woke.load(Ordering::Acquire),
            &|| "capture conversion failed to wake the idle engine".into(),
        );
        fixture.render();
        assert_eq!(stack(fixture).len(), 2);
    }

    fn static_pixels(trim: bool) {
        let fixture = Fixture::new();
        let offscreen = fixture
            .engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16))
            .expect("offscreen");
        let build = |surface: &Surface<Gpu>| {
            surface.clear_color(WorkingColor::BLACK);
            let layer = surface.layer();
            let pixels = surface.record(|c| {
                c.fill(
                    Rect::new(8., 6., 80., 54.),
                    WorkingColor::new([0.08, 0.6, 0.2, 1.]),
                );
                c.fill(
                    Rect::new(16., 12., 40., 36.),
                    WorkingColor::new([0.7, 0.1, 0.3, 1.]),
                );
            });
            surface.update(|tx| {
                tx[surface.root()].push(&layer);
                tx[&layer].content(pixels);
            });
            layer
        };
        let layer = build(&fixture.window);
        let reference = build(&offscreen);
        fixture.render();
        for x in [1., 2.] {
            for (surface, layer) in [(&fixture.window, &layer), (&offscreen, &reference)] {
                surface.update(|tx| {
                    tx[layer].transform(Affine::translate((x, 0.)));
                });
            }
            fixture.render();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while stack(&fixture).len() != 2 {
            fixture.woke.store(false, Ordering::Relaxed);
            fixture.render();
            if stack(&fixture).len() == 2 {
                break;
            }
            drive(deadline, &|| fixture.woke.load(Ordering::Acquire), &|| {
                "static plane completion never arrived".into()
            });
        }
        engine_parity(&fixture, &offscreen, "static IOSurface");
        if trim {
            fixture.engine.trim(cherenkov::Pressure::Critical);
            fixture
                .window
                .display(Display {
                    scale: SCALE,
                    headroom: 2.0,
                })
                .expect("headroom-only update");
            fixture.render();
            assert_eq!(stack(&fixture).len(), 1, "trim demotes before presentation");
            assert!(
                fixture.engine.stats().passes > 0,
                "trim reconstructs engine pixels"
            );
            engine_parity(&fixture, &offscreen, "trim then present only");
            fixture.window.display_moved().expect("display move");
            fixture.render();
            assert_eq!(stack(&fixture).len(), 1);
            return;
        }
        static_headroom_completion(&fixture);
        fixture
            .window
            .update_animated(cherenkov::Curve::linear(Duration::from_secs(2)), |tx| {
                tx[&layer].transform(Affine::translate((12., 0.)));
            });
        assert_eq!(
            fixture.engine.render(FrameTime::now()).expect("handoff"),
            cherenkov::Next::Idle
        );
        assert_eq!(fixture.engine.stats().passes, 0);
        drop(layer);
        fixture.render();
        assert_eq!(stack(&fixture).len(), 1);
    }

    /// A native translation owns scheduling and cancels on a snap.
    fn promoted_translation_is_owned_by_core_animation() {
        let fixture = Fixture::new();
        let buffer = bgra_buffer();
        let video = fixture.window.layer();
        let (video_prod, sink) = fixture.engine.frame_producer();
        sink.submit(bgra(&fixture.metal, &buffer, FrameColor::SRGB));
        fixture.window.update(|tx| {
            tx[fixture.window.root()].push(&video);
            tx[&video].content(video_prod.at(VIDEO_SIZE));
        });
        assert!(
            fixture.promote(),
            "the compositor must accept the BGRA plane"
        );
        fixture
            .window
            .update_animated(cherenkov::Curve::linear(Duration::from_secs(2)), |tx| {
                tx[&video].transform(Affine::translate((24., 16.)));
            });
        let next = fixture
            .engine
            .render(FrameTime::now())
            .expect("handoff frame");
        assert_eq!(next, cherenkov::Next::Idle);
        assert_eq!(
            fixture.engine.stats().passes,
            0,
            "a plane pose needs no engine pass"
        );
        drain_main();
        let display = displays(&fixture.root()).pop().expect("promoted display");
        let scroll = display.superlayer().expect("scroll layer");
        let node = scroll.superlayer().expect("transform layer");
        // SAFETY: the layer and its animation are confined to main.
        assert!(
            unsafe { node.animationForKey(&objc2_foundation::NSString::from_str("position.x")) }
                .is_some()
        );
        assert_eq!(node.position(), CGPoint::new(24., 16.));
        // A new video frame and a full present-only compose must leave
        // the compositor-owned affine/position pair intact.
        sink.submit(bgra(&fixture.metal, &buffer, FrameColor::SRGB));
        fixture.render();
        assert_eq!(node.position(), CGPoint::new(24., 16.));
        assert_eq!(node.affineTransform().tx, 0.0);
        fixture
            .window
            .display(Display {
                scale: SCALE,
                headroom: 2.0,
            })
            .expect("headroom-only compose");
        fixture.render();
        assert_eq!(node.position(), CGPoint::new(24., 16.));
        assert_eq!(node.affineTransform().tx, 0.0);
        fixture
            .window
            .visibility(cherenkov::Visibility::Hidden)
            .expect("hide animated plane");
        assert!(matches!(
            fixture.engine.render(FrameTime::now()),
            Err(cherenkov::RenderError::Hidden)
        ));
        fixture
            .window
            .visibility(cherenkov::Visibility::Visible)
            .expect("show animated plane");
        assert_eq!(
            fixture
                .engine
                .render(FrameTime::now())
                .expect("shown frame"),
            cherenkov::Next::Idle
        );
        fixture.window.update(|tx| {
            tx[&video].transform(Affine::translate((8., 4.)));
        });
        fixture.render();
        // SAFETY: the layer and its animation are confined to main.
        assert!(
            unsafe { node.animationForKey(&objc2_foundation::NSString::from_str("position.x")) }
                .is_none()
        );
        assert_eq!(node.position(), CGPoint::new(0., 0.));
        assert_eq!(node.affineTransform().tx, 8.);
        drop(video);
        fixture.render();
        assert_eq!(displays(&fixture.root()).len(), 0);
    }

    /// The system compositor's result for the promoted stack matches the
    /// engine's own composition within FLIP mean 0.05 and maximum 0.25.
    /// The opaque BGRA/sRGB frame isolates geometry and blending from YUV decoding.
    fn promoted_composition_matches_engine_composition() {
        let fixture = Fixture::new();
        let buffer = surface_buffer(VIDEO.0, VIDEO.1, kCVPixelFormatType_32BGRA);
        fill(&buffer, 1, |_, row| {
            for (x, px) in row[..VIDEO.0 * 4]
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .enumerate()
            {
                *px = [
                    u8::try_from(40 + x * 3).expect("blue"),
                    96,
                    u8::try_from(30 + x * 4).expect("red"),
                    255,
                ];
            }
        });
        let bgra = |metal: &Metal| {
            ExternalFrame::rgb(
                plane_texture(
                    metal,
                    &buffer,
                    0,
                    (MTLPixelFormat::BGRA8Unorm, wgpu::TextureFormat::Bgra8Unorm),
                ),
                RgbAlpha::Opaque,
                FrameColor::SRGB,
            )
            .expect("a valid BGRA frame")
        };
        let offscreen = fixture
            .engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16))
            .expect("offscreen");
        let _engine_scene = scene_bar(&fixture.engine, &offscreen, bgra(&fixture.metal), 1.0);
        let _window_scene = scene_bar(&fixture.engine, &fixture.window, bgra(&fixture.metal), 1.0);
        if fixture.promote() {
            assert_eq!(displays(&fixture.root()).len(), 1, "promoted");
        }
        // The parity holds either way: a host that never reports the
        // probe ready shows the engine's own composition in the window.
        engine_parity(&fixture, &offscreen, "promoted");
    }

    /// Clears the producer's frame to one opaque colour — the pixels a
    /// promoted plane and the engine's own quad both sample.
    struct Clearing(wgpu::Color);

    impl GpuContent for Clearing {
        async fn setup(&mut self, _: &wgpu::Context<'_>) {}

        fn render(&mut self, frame: &mut wgpu::Frame<'_>) {
            let mut encoder = frame
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.0),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            drop(pass);
            frame.queue.submit([encoder.finish()]);
        }
    }

    /// A rendered producer — `Engine::gpu_producer` — on an eligible
    /// layer promotes like a submitted frame: the plan names its binding
    /// (one display layer under the root), and the system's composition
    /// of the promoted stack matches the engine's own, the ring frame's
    /// declared linear-P3 decode being the identity.
    fn a_rendered_producer_promotes_and_matches_composited() {
        let fixture = Fixture::new();
        let video = fixture.engine.gpu_producer(GpuContentBox::new(
            Clearing(wgpu::Color {
                r: 0.25,
                g: 0.5,
                b: 0.2,
                a: 1.0,
            }),
            || {},
        ));
        // The window is opaque, so its bottom engine part shows black
        // where nothing is drawn while the offscreen stays transparent; an
        // opaque backdrop gives both the same pixels outside the video.
        let layer = |surface: &Surface<Gpu>| {
            let backdrop = surface.layer();
            let layer = surface.layer();
            let fill = surface.record(|c| {
                c.fill(
                    Rect::new(0.0, 0.0, f64::from(SIZE.0), f64::from(SIZE.1)),
                    WorkingColor::new([0.1, 0.3, 0.6, 1.0]),
                );
            });
            surface.update(|tx| {
                tx[surface.root()].push(&backdrop).push(&layer);
                tx[&backdrop].content(fill);
                tx[&layer].content(video.at(VIDEO_SIZE));
            });
            [backdrop, layer]
        };
        let _window_layer = layer(&fixture.window);
        let offscreen = fixture
            .engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16))
            .expect("offscreen");
        let _offscreen_layer = layer(&offscreen);
        if fixture.promote() {
            assert_eq!(
                displays(&fixture.root()).len(),
                1,
                "the plan names the producer's binding"
            );
        }
        // The parity holds either way: a host that never reports the
        // probe ready shows the engine's own composition in the window.
        engine_parity(&fixture, &offscreen, "promoted rendered producer");
    }

    /// The pixels a window and an offscreen engine surface produce from
    /// the same scene, compared within the perceptual tolerance used for
    /// promoted-vs-engine checks: FLIP mean at most 0.05 and no local
    /// error above 0.25.
    fn engine_parity(fixture: &Fixture, offscreen: &Surface<Gpu>, what: &str) {
        let engine = offscreen.readback().expect("engine composition");
        let system = fixture.system.composite();
        let image = |pixels: Vec<[f32; 4]>| cherenkov_oracle::F32Image {
            width: SIZE.0,
            height: SIZE.1,
            pixels,
        };
        let (metrics, _) =
            cherenkov_oracle::metrics::compare(&image(engine.pixels), &image(system));
        assert!(
            metrics.flip_mean <= 0.05 && metrics.max_local_error <= 0.25,
            "{what} vs engine composition: {metrics:?}"
        );
    }

    /// The platform decodes `ITU_R_709_2` with the inverse OETF while the
    /// engine applies BT.1886 gamma 2.4 and no colour tag reproduces
    /// gamma 2.4, so `shows` keeps a BT.709 frame in the engine: no plane,
    /// and the window shows the engine's own composition.
    fn a_bt709_frame_stays_in_the_engine_and_matches() {
        let fixture = Fixture::new();
        let buffer = nv12_buffer(VIDEO);
        let offscreen = fixture
            .engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16))
            .expect("offscreen");
        let _engine_scene = scene_bar(
            &fixture.engine,
            &offscreen,
            nv12(&fixture.metal, &buffer, FrameColor::BT709_VIDEO),
            1.0,
        );
        let _window_scene = scene_bar(
            &fixture.engine,
            &fixture.window,
            nv12(&fixture.metal, &buffer, FrameColor::BT709_VIDEO),
            1.0,
        );
        fixture.refusal_cycle();
        assert!(
            displays(&fixture.root()).is_empty(),
            "the plan keeps BT.709 in the engine"
        );
        engine_parity(&fixture, &offscreen, "engine-composited BT.709");
    }

    /// A layer painted above a plane that is not known to be opaque is
    /// blended by the platform in its own space, not the engine's linear
    /// blend, so the plan keeps the video in the engine: no plane, and
    /// the window shows the engine's own composition.
    fn a_translucent_layer_above_stays_in_the_engine_and_matches() {
        let fixture = Fixture::new();
        let buffer = surface_buffer(VIDEO.0, VIDEO.1, kCVPixelFormatType_32BGRA);
        let bgra = |metal: &Metal| {
            ExternalFrame::rgb(
                plane_texture(
                    metal,
                    &buffer,
                    0,
                    (MTLPixelFormat::BGRA8Unorm, wgpu::TextureFormat::Bgra8Unorm),
                ),
                RgbAlpha::Opaque,
                FrameColor::SRGB,
            )
            .expect("a valid BGRA frame")
        };
        let offscreen = fixture
            .engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16))
            .expect("offscreen");
        let _engine_scene = scene(&fixture.engine, &offscreen, bgra(&fixture.metal));
        let _window_scene = scene(&fixture.engine, &fixture.window, bgra(&fixture.metal));
        fixture.render();
        assert!(
            displays(&fixture.root()).is_empty(),
            "the plan keeps a video under a translucent layer in the engine"
        );
        engine_parity(
            &fixture,
            &offscreen,
            "engine-composited video under a translucent layer",
        );
    }

    /// One scaled video layer under the root and nothing else — the
    /// `apple_planes` `overlay` tree: the promoted layer is the last
    /// painted, so `Plan::trailing` is false and the surface has one
    /// part. The composite stack is the part, then the plane.
    fn a_promoted_layer_painted_last_composes() {
        let fixture = Fixture::new();
        let buffer = bgra_buffer();
        let (video, sink) = fixture.engine.frame_producer();
        sink.submit(bgra(&fixture.metal, &buffer, FrameColor::SRGB));
        let layer = fixture.window.layer();
        fixture.window.update(|tx| {
            tx[fixture.window.root()].push(&layer);
            tx[&layer]
                .transform(Affine::scale(f64::from(SIZE.0) / f64::from(VIDEO_SIZE.0)))
                .content(video.at(VIDEO_SIZE));
        });
        assert!(
            fixture.promote(),
            "the platform never reported the candidate ready"
        );
        // part 0, then the plane nested in its path's levels.
        let stack = stack(&fixture);
        let [part, plane] = &stack[..] else {
            panic!("one part under the plane, found {} layers", stack.len());
        };
        assert!(is::<CAMetalLayer>(part));
        assert!(!is::<CAMetalLayer>(plane));
        assert_eq!(displays(&fixture.root()).len(), 1, "one promoted plane");
    }

    /// Two video layers promoted together with the second painted last:
    /// the empty part between them still exists (the planes need a layer
    /// between them in the stack), and no part opens after the last one.
    /// The composite stack is part, plane, part, plane.
    fn two_promoted_layers_with_the_last_painted_last_compose() {
        let fixture = Fixture::new();
        let layer = |x: f64| {
            let buffer = bgra_buffer();
            let (video, sink) = fixture.engine.frame_producer();
            sink.submit(bgra(&fixture.metal, &buffer, FrameColor::SRGB));
            let layer = fixture.window.layer();
            fixture.window.update(|tx| {
                tx[fixture.window.root()].push(&layer);
                tx[&layer]
                    .transform(
                        Affine::translate((x, 0.0))
                            * Affine::scale(f64::from(SIZE.0 / 2) / f64::from(VIDEO_SIZE.0)),
                    )
                    .content(video.at(VIDEO_SIZE));
            });
            layer
        };
        let _first = layer(0.0);
        let _second = layer(f64::from(SIZE.0 / 2));
        assert!(
            fixture.promote(),
            "the platform never reported a candidate ready"
        );
        // Each candidate promotes on its own readiness round.
        let deadline = Instant::now() + Duration::from_secs(30);
        while displays(&fixture.root()).len() < 2 {
            assert!(Instant::now() < deadline, "the second plane never showed");
            drain_main();
            fixture.engine.render(FrameTime::now()).expect("rendered");
            drain_main();
        }
        // part 0, plane, the separator part, plane — the planes nest in
        // their paths' levels.
        let stack = stack(&fixture);
        let [first, plane_a, second, plane_b] = &stack[..] else {
            panic!("part, plane, part, plane — found {} layers", stack.len());
        };
        assert!(is::<CAMetalLayer>(first));
        assert!(is::<CAMetalLayer>(second));
        assert!(!is::<CAMetalLayer>(plane_a) && !is::<CAMetalLayer>(plane_b));
    }
}
