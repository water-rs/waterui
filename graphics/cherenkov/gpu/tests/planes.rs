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

    use cherenkov::kurbo::{Affine, Rect, RoundedRect, Size, Vec2};
    use cherenkov::{
        Display, Draw as _, Engine, FrameTime, Hosted, Layer, Offscreen, OffscreenFormat,
        RenderError, Surface, WorkingColor,
    };
    use cherenkov_gpu::interop::wgpu::rwh::{
        AppKitWindowHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle,
        RawWindowHandle, WindowHandle,
    };
    use cherenkov_gpu::interop::{
        ExternalFrame, FrameColor, GpuContent, GpuContentBox, RgbAlpha, SharedDevice, YuvRange,
        apple::HostedView, metal::import_texture, wgpu,
    };
    use cherenkov_gpu::{DisplaySync, Gpu, GpuConfig, WindowTarget};
    use dispatch2::DispatchQueue;
    use libtest_mimic::Trial;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AnyThread as _, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
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
    use objc2_quartz_core::{CALayer, CALayerDelegate, CAMetalLayer, CARenderer, CATransaction};

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
        .into_iter()
        .chain(hosted_trials())
        .collect()
    }

    /// The hosted-plane cases.
    fn hosted_trials() -> Vec<Trial> {
        let case = |name: &str, run: fn()| {
            Trial::test(name, move || {
                run();
                Ok(())
            })
        };
        vec![
            case(
                "a_hosted_layer_sits_between_its_parts_and_moves_in_place",
                a_hosted_layer_sits_between_its_parts_and_moves_in_place,
            ),
            case(
                "engine_views_are_transparent_to_hits",
                engine_views_are_transparent_to_hits,
            ),
            case(
                "a_part_between_two_hosted_views_is_transparent_to_hits",
                a_part_between_two_hosted_views_is_transparent_to_hits,
            ),
            case(
                "a_steady_frame_mutates_no_views_and_keeps_the_first_responder",
                a_steady_frame_mutates_no_views_and_keeps_the_first_responder,
            ),
            case(
                "a_plane_inserted_below_the_responder_leaves_its_views_in_place",
                a_plane_inserted_below_the_responder_leaves_its_views_in_place,
            ),
            case(
                "a_rebuilt_hosted_path_gives_first_responder_back",
                a_rebuilt_hosted_path_gives_first_responder_back,
            ),
            case(
                "contents_orientation_updates_engine_layer_geometry_in_place",
                contents_orientation_updates_engine_layer_geometry_in_place,
            ),
            case(
                "engine_ordering_preserves_foreign_siblings_for_all_plane_shapes",
                engine_ordering_preserves_foreign_siblings_for_all_plane_shapes,
            ),
            case(
                "a_layer_switching_between_frame_and_hosted_rebuilds_its_nodes",
                a_layer_switching_between_frame_and_hosted_rebuilds_its_nodes,
            ),
            case(
                "a_scrolled_hosted_view_hits_only_where_it_is_visible",
                a_scrolled_hosted_view_hits_only_where_it_is_visible,
            ),
            case(
                "a_negative_or_zero_scale_on_the_path_is_unplaceable",
                a_negative_or_zero_scale_on_the_path_is_unplaceable,
            ),
            case(
                "a_rotation_and_a_skew_on_the_path_are_unplaceable",
                a_rotation_and_a_skew_on_the_path_are_unplaceable,
            ),
            case(
                "opacity_below_one_renders_with_alpha_value",
                opacity_below_one_renders_with_alpha_value,
            ),
            case(
                "scroll_and_scale_update_the_bounds_in_place",
                scroll_and_scale_update_the_bounds_in_place,
            ),
            case(
                "an_unplaceable_hosted_layer_fails_every_render_until_placeable",
                an_unplaceable_hosted_layer_fails_every_render_until_placeable,
            ),
            case(
                "a_hosted_layer_without_planes_fails_the_render",
                a_hosted_layer_without_planes_fails_the_render,
            ),
            case(
                "back_to_back_frames_present_without_a_runloop_turn",
                back_to_back_frames_present_without_a_runloop_turn,
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

    objc2::define_class!(
        // SAFETY: the subclass adds a main-thread-only orientation flag.
        #[unsafe(super(CALayer))]
        #[name = "OrientationBackingLayer"]
        #[thread_kind = MainThreadOnly]
        #[ivars = std::cell::Cell<bool>]
        struct OrientationBackingLayer;

        impl OrientationBackingLayer {
            #[unsafe(method(contentsAreFlipped))]
            fn contents_are_flipped(&self) -> bool {
                self.ivars().get()
            }
        }
    );

    impl OrientationBackingLayer {
        fn set_contents_flipped(&self, flipped: bool) {
            self.ivars().set(flipped);
        }
    }

    objc2::define_class!(
        // SAFETY: `NSObject` has no subclassing requirements; the class
        // implements no Drop.
        #[unsafe(super(objc2::runtime::NSObject))]
        #[name = "CherenkovWindowlessSurfaceHost"]
        #[ivars = ()]
        struct WindowlessHost;

        impl WindowlessHost {
            // wgpu's macOS acquire probes the hosting window's
            // `occlusionState` and refuses the drawable while it is not
            // visible: correct for a real window, but the test window is
            // never ordered on screen, so every acquire would be
            // occluded forever. A delegate answering `window` with nil
            // reads as "no hosting window" — the windowless behaviour
            // the harness had before — letting parts present inside the
            // hidden window.
            #[unsafe(method(window))]
            fn window(&self) -> *mut NSWindow {
                std::ptr::null_mut()
            }
        }

        unsafe impl NSObjectProtocol for WindowlessHost {}
        unsafe impl CALayerDelegate for WindowlessHost {}
    );

    /// Gives every `CAMetalLayer` under `layer` a delegate whose
    /// `window` is nil, so wgpu's occlusion probe never finds the
    /// never-shown test window (see [`WindowlessHost`]).
    fn unocclude(layer: &CALayer, stub: &ProtocolObject<dyn CALayerDelegate>) {
        for sub in sublayers(layer) {
            match sub.downcast::<CAMetalLayer>() {
                Ok(metal) => metal.setDelegate(Some(stub)),
                Err(layer) => unocclude(&layer, stub),
            }
        }
    }

    /// A windowless-host delegate for part surfaces (see
    /// [`WindowlessHost`]).
    fn windowless_stub() -> Retained<ProtocolObject<dyn CALayerDelegate>> {
        let this = WindowlessHost::alloc().set_ivars(());
        // SAFETY: `init` on the freshly allocated delegate; documented
        // `NSObject` init pattern.
        let this: Retained<WindowlessHost> = unsafe { objc2::msg_send![super(this), init] };
        ProtocolObject::from_retained(this)
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
                        // SAFETY: the renderer is read on the main thread.
                        unsafe { r.status() },
                        // SAFETY: the renderer is read on the main thread.
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

    objc2::define_class!(
        // SAFETY: `NSView` has no subclassing requirements; the class
        // implements no Drop.
        #[unsafe(super(NSView))]
        #[name = "CountingHostView"]
        #[thread_kind = MainThreadOnly]
        #[ivars = (
            std::cell::Cell<usize>,
            std::cell::Cell<usize>,
            std::cell::RefCell<Vec<(bool, usize)>>,
            std::cell::RefCell<Option<Retained<OrientationBackingLayer>>>
        )]
        /// The fixture's host view: a plain `NSView` counting the
        /// hierarchy writes `addSubview:` and removals issue against it,
        /// so a steady frame can be proved to mutate nothing.
        struct CountingHostView;

        impl CountingHostView {
            #[unsafe(method_id(makeBackingLayer))]
            fn make_backing_layer(&self) -> Retained<CALayer> {
                let mtm = MainThreadMarker::new().expect("main thread");
                let layer =
                    OrientationBackingLayer::alloc(mtm).set_ivars(std::cell::Cell::new(false));
                // SAFETY: `CALayer`'s designated initializer is `init`.
                let layer: Retained<OrientationBackingLayer> =
                    unsafe { objc2::msg_send![super(layer), init] };
                *self.ivars().3.borrow_mut() = Some(layer.clone());
                Retained::into_super(layer)
            }

            /// `AppKit` reports a view added below this one.
            #[unsafe(method(didAddSubview:))]
            fn did_add_subview(&self, view: Option<&NSView>) {
                self.ivars().0.set(self.ivars().0.get() + 1);
                self.ivars().2.borrow_mut().push((
                    true,
                    view.map_or(0, |view| std::ptr::from_ref(view).addr()),
                ));
            }

            /// `AppKit` reports a view about to leave this one.
            #[unsafe(method(willRemoveSubview:))]
            fn will_remove_subview(&self, view: Option<&NSView>) {
                self.ivars().1.set(self.ivars().1.get() + 1);
                self.ivars().2.borrow_mut().push((
                    false,
                    view.map_or(0, |view| std::ptr::from_ref(view).addr()),
                ));
            }
        }
    );

    impl CountingHostView {
        /// The `addSubview:` and removal calls seen since `reset_writes`.
        fn hierarchy_writes(&self) -> (usize, usize) {
            let ivars = self.ivars();
            (ivars.0.get(), ivars.1.get())
        }

        fn hierarchy_changes(&self) -> Vec<(bool, usize)> {
            self.ivars().2.borrow().clone()
        }

        fn reset_writes(&self) {
            self.ivars().0.set(0);
            self.ivars().1.set(0);
            self.ivars().2.borrow_mut().clear();
        }

        fn orientation_layer(&self) -> Retained<OrientationBackingLayer> {
            self.ivars()
                .3
                .borrow()
                .as_ref()
                .expect("the view made its backing layer")
                .clone()
        }
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
        // Keeps the renderer's root alive; all staging goes through
        // `points`, which the display pass cannot reclaim.
        _stage: Retained<CALayer>,
        /// The points-scaled layer `host` parents under: kept so a
        /// composite can re-stage the layer after `AppKit`'s display pass
        /// reclaims it for the never-shown window.
        points: Retained<CALayer>,
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
                _stage: stage,
                points,
            }
        }

        /// Composites what is committed, returning premultiplied linear
        /// Display P3 pixels, row 0 at the top of the screen.
        ///
        /// `AppKit`'s display pass reclaims a windowed view's layer for the
        /// window's own context, which a `CARenderer` cannot see — so the
        /// host is re-staged under `points` here before rendering. Its
        /// `AppKit`-wired subtree comes along whole; the next
        /// `displayIfNeeded` takes it back and syncs the view order
        /// again.
        ///
        /// The first frame after a layer joins the renderer's context from
        /// another context composites nearly-transparent content, so one
        /// warm-up frame is rendered and discarded before the read below.
        ///
        /// The renderer writes layer space bottom-up (row 0 is `y = 0`, the
        /// bottom of an unflipped layer), so rows are reversed.
        fn composite(&self, host: &CALayer) -> Vec<[f32; 4]> {
            // No runloop turn before `render()`: any `drive` lets the
            // window's display pass reclaim `host` for its own context,
            // leaving `stage` empty again.
            let bounds = self.renderer.bounds();
            let frame = || {
                self.points.addSublayer(host);
                CATransaction::flush();
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
            };
            frame();
            frame();
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

    /// An engine with a window surface over a view inside an `NSWindow`
    /// that is never ordered on screen — `AppKit` wires the subview layers
    /// itself — the host layer staged for the system compositor.
    struct Fixture {
        metal: Metal,
        engine: Engine<Gpu>,
        window: Surface<Gpu>,
        system: SystemCompositor,
        /// Never ordered on screen; `displayIfNeeded` makes `AppKit` wire
        /// and mirror the view hierarchy's layers.
        app_window: Retained<NSWindow>,
        windowless: Retained<ProtocolObject<dyn CALayerDelegate>>,
        /// `view` as its concrete class, for the hierarchy-write counts.
        counting: Retained<CountingHostView>,
        view: Retained<NSView>,
        /// Set by the window surface's wake: an attach landing on the
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
            let counting: Retained<CountingHostView> = {
                let this = CountingHostView::alloc(mtm).set_ivars((
                    std::cell::Cell::new(0),
                    std::cell::Cell::new(0),
                    std::cell::RefCell::new(Vec::new()),
                    std::cell::RefCell::new(None),
                ));
                // SAFETY: `msg_send!` to `super.initWithFrame:` is the
                // designated initializer.
                unsafe {
                    objc2::msg_send![
                        super(this),
                        initWithFrame: CGRect::new(
                            CGPoint::new(0.0, 0.0),
                            CGSize::new(f64::from(SIZE.0) / SCALE, f64::from(SIZE.1) / SCALE),
                        )
                    ]
                }
            };
            let view = Retained::into_super(counting.clone());
            view.setWantsLayer(true);
            // Never ordered on screen: no `makeKeyAndOrderFront`, no
            // `orderFront` — a window off-screen still runs AppKit's own
            // layer wiring for the view hierarchy.
            // SAFETY: init with a content rect; the window is never shown.
            let app_window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    CGRect::new(
                        CGPoint::new(0.0, 0.0),
                        CGSize::new(f64::from(SIZE.0) / SCALE, f64::from(SIZE.1) / SCALE),
                    ),
                    NSWindowStyleMask::empty(),
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            app_window.setContentView(Some(&view));
            let host = view.layer().expect("a layer-backed view");
            host.setContentsScale(SCALE);
            let system = SystemCompositor::attach(&metal, &host);
            let woke = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&woke);
            let window = engine
                .surface(
                    configure(WindowTarget::new(
                        View(dispatch2::MainThreadBound::new(view.clone(), mtm)),
                        SIZE,
                    )),
                    move || flag.store(true, Ordering::Release),
                )
                .expect("a window surface");
            window
                .display(Display {
                    scale: SCALE,
                    headroom: 1.0,
                })
                .expect("the display");
            Self {
                metal,
                engine,
                window,
                system,
                app_window,
                windowless: windowless_stub(),
                counting,
                view,
                woke,
            }
        }

        fn host(&self) -> Retained<CALayer> {
            self.view.layer().expect("a layer-backed view")
        }

        /// The host's backing layer: `AppKit` wires the engine's part and
        /// plane views' layers into it in subview order, and a
        /// candidate's pending display parks inside the probes view's
        /// layer below them.
        fn root(&self) -> Retained<CALayer> {
            self.host()
        }

        /// Drains queued main work, lets `AppKit` wire any new subview
        /// layers, and stubs the window probe on every part surface:
        /// `addSubview` wires a part's layer under the window's layer
        /// synchronously, so the stub has to be re-applied after every
        /// drain, before the next render can acquire.
        fn sync(&self) {
            drain_main();
            unocclude(&self.host(), &self.windowless);
            self.app_window.displayIfNeeded();
            unocclude(&self.host(), &self.windowless);
        }

        fn render(&self) {
            self.sync();
            self.engine.render(FrameTime::now()).expect("rendered");
            self.sync();
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
            self.sync();
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
                self.sync();
                self.engine.render(FrameTime::now()).expect("rendered");
                self.sync();
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
                self.sync();
                self.engine.render(FrameTime::now()).expect("rendered");
                self.sync();
                if !displays(&self.root()).is_empty() {
                    break;
                }
                let probes = probes(&self.host());
                assert!(
                    !probes
                        .iter()
                        // SAFETY: the display layers are read on the main
                        // thread.
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
            // SAFETY: the display layers and their renderers are read on
            // the main thread.
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

    /// The engine's top-level elements under the host: parts and plane
    /// tops in paint order — the host view's subview order is the paint
    /// order, and `AppKit` orders their layers to match. The probes view,
    /// whose layer holds parked pending displays, is not an element.
    fn stack(fixture: &Fixture) -> Vec<Retained<CALayer>> {
        fixture
            .view
            .subviews()
            .iter()
            .filter_map(|view| view.layer())
            .filter(|layer| {
                layer
                    .name()
                    .is_none_or(|name| *name != *objc2_foundation::ns_string!("cherenkov-probes"))
            })
            .collect()
    }

    /// A top-level part: a container whose sublayer is the metal layer.
    fn is_part(layer: &CALayer) -> bool {
        is::<CAMetalLayer>(layer)
            || sublayers(layer)
                .iter()
                .any(|inner| is::<CAMetalLayer>(inner))
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
                .flat_map(|layer| std::iter::once(layer.clone()).chain(sublayers(&layer)))
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
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
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
        assert!(
            fixture.promote(),
            "the platform never reported the probe ready"
        );

        let stack = stack(&fixture);
        assert!(
            stack.iter().all(|l| l.isGeometryFlipped()),
            "engine space is y-down"
        );
        assert_eq!(stack.len(), 3, "part, plane, part");
        assert!(is_part(&stack[0]) && is_part(&stack[2]));
        assert!(!is_part(&stack[1]));
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
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
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
        assert!(
            fixture.promote(),
            "the platform never reported the probe ready"
        );
        let _ = fixture.system.composite(&fixture.host());
        let [display] = &displays(&fixture.root())[..] else {
            panic!("one display");
        };
        // SAFETY: renderer and buffer attachments are read on main.
        let mut shown = unsafe { display.sampleBufferRenderer().copyDisplayedPixelBuffer() };
        let deadline = Instant::now() + Duration::from_secs(10);
        while shown.is_none() {
            assert!(Instant::now() < deadline, "displayed buffer never arrived");
            // SAFETY: the mode is an immutable static.
            CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0.005, true);
            // SAFETY: as above.
            shown = unsafe { display.sampleBufferRenderer().copyDisplayedPixelBuffer() };
        }
        let shown = shown.expect("displayed buffer");
        assert_eq!(
            CVPixelBufferGetIOSurface(Some(&buffer))
                .expect("source")
                .id(),
            CVPixelBufferGetIOSurface(Some(&shown))
                .expect("displayed")
                .id()
        );
        // SAFETY: the attachment keys and expected values are immutable
        // CoreVideo statics, read on the main thread.
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
        assert!(is_part(&stack[0]));
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
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
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
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
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
        assert!(
            // SAFETY: the layer and its animation are confined to main.
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
        assert!(
            // SAFETY: the layer and its animation are confined to main.
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
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
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
        let video = fixture
            .engine
            .gpu_producer(GpuContentBox::new(Clearing(wgpu::Color {
                r: 0.25,
                g: 0.5,
                b: 0.2,
                a: 1.0,
            })));
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
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
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
        let system = fixture.system.composite(&fixture.host());
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
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
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
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
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
        assert!(is_part(part));
        assert!(!is_part(plane));
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
        assert!(is_part(first));
        assert!(is_part(second));
        assert!(!is_part(plane_a) && !is_part(plane_b));
    }

    /// An `NSView` a host would supply: layer-backed, the way a
    /// `WKWebView` is — `AppKit` owns its layer; the engine may never
    /// touch it. The colour on the backing layer is the app's own.
    fn hosted_view(mtm: MainThreadMarker, color: (f64, f64, f64)) -> Retained<NSView> {
        let view = NSView::new(mtm);
        view.setWantsLayer(true);
        view.layer()
            .expect("a layer-backed view")
            .setBackgroundColor(Some(&objc2_core_graphics::CGColor::new_srgb(
                color.0, color.1, color.2, 1.0,
            )));
        view
    }

    /// A host's view, as the content of `web` at `EXTENT`, between a
    /// backdrop painted below it and two red bars painted above it in
    /// the same part. The holder's clip is smaller than the hosted view
    /// — px (12,8)-(42,26) against the hosted (12,8)-(52.5,32) — with a
    /// corner radius of 8 that a pixel inside the hosted rect escapes;
    /// the bars' own clips bound where they paint.
    fn hosted_scene(fixture: &Fixture, web: &Retained<NSView>) -> [Layer; 5] {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let surface = &fixture.window;
        let below = surface.layer();
        let holder = surface.layer();
        let hosted = surface.layer();
        let above = surface.layer();
        let above2 = surface.layer();
        let backdrop = surface.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 96.0, 64.0),
                WorkingColor::new([0.1, 0.3, 0.6, 1.0]),
            );
        });
        let bar = |x0: f64, x1: f64| {
            surface.record(|c| {
                c.fill(
                    Rect::new(x0, 10.0, x1, 26.0),
                    WorkingColor::new([0.7, 0.1, 0.1, 1.0]),
                );
            })
        };
        let object = Hosted::<Gpu>::new(HostedView::new(web.clone(), mtm));
        surface.update(|tx| {
            tx[surface.root()]
                .push(&below)
                .push(&holder)
                .push(&above)
                .push(&above2);
            tx[&below].content(backdrop);
            tx[&holder]
                .push(&hosted)
                .transform(Affine::translate((12.0, 8.0)))
                .clip(RoundedRect::new(0.0, 0.0, 30.0, 18.0, 8.0));
            tx[&hosted].content(object.at(EXTENT));
            tx[&above]
                .content(bar(30.0, 40.0))
                .clip(RoundedRect::new(30.0, 10.0, 40.0, 26.0, 0.0));
            tx[&above2]
                .content(bar(24.0, 28.0))
                .clip(RoundedRect::new(24.0, 10.0, 28.0, 26.0, 0.0));
        });
        [below, holder, hosted, above, above2]
    }

    /// Whether `a` and `b` are the same Core Animation layer.
    fn same_layer(a: &CALayer, b: &CALayer) -> bool {
        std::ptr::from_ref(a) == std::ptr::from_ref(b)
    }

    /// The hosted extent, in the layer's content units.
    const EXTENT: Size = Size::new(40.5, 24.0);

    /// Renders until `shown` holds. A frame that needs parts the window
    /// does not have yet, or whose plan adds a static capture still being
    /// converted, presents nothing and keeps the committed scene; the part
    /// creation or the conversion wakes the engine for the frame that
    /// presents.
    fn render_until(fixture: &Fixture, shown: &dyn Fn() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            fixture.woke.store(false, Ordering::Relaxed);
            fixture.render();
            if shown() {
                return;
            }
            drive(deadline, &|| fixture.woke.load(Ordering::Acquire), &|| {
                format!("{what}: no wake for the frame that presents it")
            });
        }
    }

    /// The superview of `view`, as a raw pointer for identity checks.
    fn view_superview(view: &NSView) -> Option<*const NSView> {
        // SAFETY: the view hierarchy is read on the main thread.
        unsafe { view.superview() }.map(|v| Retained::as_ptr(&v))
    }

    /// The engine-owned view immediately below the host in the hosted
    /// view's ancestor chain.
    fn top_ancestor(view: &NSView, host: &NSView) -> *const NSView {
        // SAFETY: Cocoa's view hierarchy is read on the main thread.
        let mut current = unsafe { view.superview() }.expect("a hosted leaf parent");
        loop {
            // SAFETY: Cocoa's view hierarchy is read on the main thread.
            let parent = unsafe { current.superview() }.expect("the host is an ancestor");
            if Retained::as_ptr(&parent) == std::ptr::from_ref(host) {
                return Retained::as_ptr(&current);
            }
            current = parent;
        }
    }

    fn direct_subviews(host: &NSView) -> Vec<*const NSView> {
        host.subviews()
            .iter()
            .map(|view| Retained::as_ptr(&view))
            .collect()
    }

    /// The view at `ptr`, read on the main thread while the engine's
    /// scene owns it.
    const fn as_view<'a>(ptr: *const NSView) -> &'a NSView {
        // SAFETY: `ptr` names a live view in the hierarchy.
        unsafe { &*ptr }
    }

    /// The system composition proves the hosted plane's place in the
    /// stack: the bar paints over the hosted view inside their overlap,
    /// the hosted colour shows outside it, the holder's rounded clip
    /// cuts the hosted rect's corner to the backdrop, and `alphaValue`
    /// below one blends the hosted pixels onto it.
    fn assert_hosted_composite(fixture: &Fixture, hosted: &Layer, web: &NSView) {
        let pixels = fixture.system.composite(&fixture.host());
        let at = |x: usize, y: usize| pixels[y * SIZE.0 as usize + x];
        // Inside the bar's clip overlapping the hosted rect: the bar's
        // opaque red — the part above paints over the hosted view.
        let overlap_px = at(34, 18);
        assert!(
            overlap_px[0] > 0.5 && overlap_px[1] < 0.3,
            "the bar paints over the hosted view: {overlap_px:?}"
        );
        // Inside the clip, outside the bars: the hosted green.
        let hosted_px = at(18, 15);
        assert!(
            hosted_px[1] > 0.4 && hosted_px[0] < 0.3,
            "hosted colour at its rect: {hosted_px:?}"
        );
        // Inside the hosted rect but outside the rounded corner —
        // px (13,9)'s centre is (13.5,9.5), clip-space (1.5,1.5), 9.19
        // from the corner's centre (8,8): past the radius 8, so the
        // backdrop's blue shows.
        let corner_px = at(13, 9);
        assert!(
            corner_px[2] > 0.4 && corner_px[1] < 0.45,
            "the rounded corner shows the backdrop: {corner_px:?}"
        );
        // Inside the hosted rect but past the clip's right edge — the
        // hosted view is clipped away, the backdrop shows.
        let clipped_px = at(46, 20);
        assert!(
            clipped_px[2] > 0.4 && clipped_px[1] < 0.45,
            "the clip bounds the hosted view: {clipped_px:?}"
        );
        // `alphaValue` 0.5 on the hosted layer blends the hosted pixels
        // onto the backdrop: AppKit mixes in the encoded (sRGB-gamma)
        // domain, so the blend of the linear premultiplied endpoints is
        // the sRGB-weighted mix back into linear — half hosted over
        // half backdrop, within a tolerance of 0.06 that also covers the
        // Display P3 primary shift.
        let hosted_lin = at(18, 15);
        let backdrop_lin = at(70, 40);
        fixture.window.update(|tx| {
            tx[hosted].opacity(0.5f32);
        });
        let leaf = as_view(view_superview(web).expect("the leaf"));
        render_until(
            fixture,
            &|| (leaf.alphaValue() - 0.5).abs() < 0.01,
            "the opacity",
        );
        let pixels = fixture.system.composite(&fixture.host());
        let at = |x: usize, y: usize| pixels[y * SIZE.0 as usize + x];
        let blended = at(18, 15);
        let srgb = |l: f32| -> f32 {
            if l <= 0.003_130_8 {
                12.92 * l
            } else {
                1.055f32.mul_add(l.powf(1.0 / 2.4), -0.055)
            }
        };
        let linear = |s: f32| -> f32 {
            if s <= 0.040_45 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        let expected: Vec<f32> = hosted_lin
            .iter()
            .zip(&backdrop_lin)
            .map(|(h, b)| linear(0.5f32.mul_add(srgb(*b), 0.5 * srgb(*h))))
            .collect();
        assert!(
            blended
                .iter()
                .zip(expected)
                .all(|(got, want)| (got - want).abs() < 0.06),
            "alphaValue 0.5 blends onto the backdrop: {blended:?}"
        );
    }

    /// The host's view is shown on a plane between the part painted below
    /// it and the part painted above it — translucent controls included —
    /// inside the engine's own views at its extent; a move on its path
    /// updates the same views, and clearing the content takes the view
    /// and its plane out of the engine's tree.
    fn a_hosted_layer_sits_between_its_parts_and_moves_in_place() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, holder, hosted, _above, _above2] = hosted_scene(&fixture, &web);
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the hosted view",
        );
        // Part, hosted plane, part — and, once the bar above has been quiet
        // long enough, its static capture on a plane of its own.
        let layers = stack(&fixture);
        let [first, top, second, ..] = &layers[..] else {
            panic!("part, plane, part — found {} layers", layers.len());
        };
        assert!(is_part(first) && is_part(second));
        let top = top.clone();
        // The plane's top is the outermost engine view's backing layer,
        // which `AppKit` wires under the host view's layer — the view
        // itself is a subview of the host between the parts' views.
        assert_eq!(
            top.superlayer().map(|s| Retained::as_ptr(&s)),
            Some(Retained::as_ptr(&fixture.host()))
        );
        let container = fixture
            .view
            .subviews()
            .iter()
            .find(|v| {
                v.layer()
                    .is_some_and(|l| Retained::as_ptr(&l) == Retained::as_ptr(&top))
            })
            .expect("the container is a host subview");
        // Its chain: a clip view for the one clipped level, carrying the
        // path's translation, then the leaf, then the host's own view.
        let leaf_ptr = view_superview(&web).expect("the leaf");
        let leaf = as_view(leaf_ptr);
        let clip_ptr = view_superview(leaf).expect("the clip");
        let clip = as_view(clip_ptr);
        assert_eq!(view_superview(clip), Some(Retained::as_ptr(&container)));
        // The hosted view is layer-backed: `AppKit` owns and wires its
        // layer — under the leaf's layer, untransformed, alone. The
        // engine never touches it.
        let hosted_layer = web.layer().expect("a layer-backed hosted view");
        assert_eq!(
            hosted_layer.superlayer().map(|s| Retained::as_ptr(&s)),
            leaf.layer().map(|l| Retained::as_ptr(&l)),
            "AppKit wired the hosted layer under the leaf's"
        );
        assert!(sublayers(&hosted_layer).is_empty(), "no engine sublayers");
        assert_eq!(hosted_layer.affineTransform().tx, 0.0);
        assert_eq!(hosted_layer.affineTransform().ty, 0.0);
        assert_eq!(
            (
                hosted_layer.affineTransform().a,
                hosted_layer.affineTransform().d
            ),
            (1.0, 1.0)
        );
        let clip_frame = clip.frame();
        assert!(
            (clip_frame.origin.x - 12.0).abs() < 1e-12 && (clip_frame.origin.y - 8.0).abs() < 1e-12,
            "the clip view carries the translation"
        );
        assert!(
            clip.layer().expect("layer-backed").masksToBounds(),
            "the clip view masks"
        );
        let frame = web.frame();
        assert_eq!(
            (frame.size.width, frame.size.height),
            (EXTENT.width, EXTENT.height)
        );
        assert_hosted_composite(&fixture, &hosted, &web);

        fixture.window.update(|tx| {
            tx[&holder].transform(Affine::translate((20.0, 4.0)));
        });
        render_until(
            &fixture,
            &|| (clip.frame().origin.x - 20.0).abs() < 1e-12,
            "the move",
        );
        assert!(same_layer(&stack(&fixture)[1], &top), "the plane stays");
        assert_eq!(view_superview(&web), Some(leaf_ptr), "the leaf stays");
        assert_eq!(view_superview(leaf), Some(clip_ptr), "the clip stays");

        fixture.window.update(|tx| {
            tx[&hosted].clear_content();
        });
        render_until(&fixture, &|| view_superview(&web).is_none(), "the release");
        assert!(
            fixture
                .view
                .subviews()
                .iter()
                .all(|v| Retained::as_ptr(&v) != Retained::as_ptr(&container)),
            "the hosted plane left the stack"
        );
    }

    fn engine_ordering_preserves_foreign_siblings_for_all_plane_shapes() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        for (plane_count, trailing_part) in
            [(0, false), (1, false), (1, true), (2, false), (2, true)]
        {
            assert_engine_ordering_case(mtm, plane_count, trailing_part);
        }
    }

    fn assert_engine_ordering_case(mtm: MainThreadMarker, plane_count: usize, trailing_part: bool) {
        const VIEW_COLORS: [(f64, f64, f64); 2] = [(0.2, 0.7, 0.3), (0.3, 0.7, 0.3)];
        const PART_COLORS: [[f32; 4]; 3] = [
            [0.1, 0.3, 0.6, 1.0],
            [0.2, 0.3, 0.6, 1.0],
            [0.3, 0.3, 0.6, 1.0],
        ];
        let fixture = Fixture::new();
        fixture.render();
        let foreign = NSView::new(mtm);
        fixture.view.addSubview(&foreign);

        let part_count = if plane_count == 0 {
            1
        } else {
            plane_count + usize::from(trailing_part)
        };
        let parts: Vec<_> = (0..part_count).map(|_| fixture.window.layer()).collect();
        let holders: Vec<_> = (0..plane_count).map(|_| fixture.window.layer()).collect();
        let planes: Vec<_> = (0..plane_count).map(|_| fixture.window.layer()).collect();
        let views: Vec<_> = VIEW_COLORS
            .into_iter()
            .take(plane_count)
            .map(|color| hosted_view(mtm, color))
            .collect();
        let hosted: Vec<_> = views
            .iter()
            .map(|view| Hosted::<Gpu>::new(HostedView::new(view.clone(), mtm)))
            .collect();

        fixture.window.update(|tx| {
            let root = fixture.window.root();
            tx[root].push(&parts[0]);
            let part_content = |index: usize| {
                fixture.window.record(|c| {
                    c.fill(
                        Rect::new(0.0, 0.0, 96.0, 64.0),
                        WorkingColor::new(PART_COLORS[index]),
                    );
                })
            };
            tx[&parts[0]].content(part_content(0));
            for index in 0..plane_count {
                tx[root].push(&holders[index]);
                tx[&holders[index]].push(&planes[index]);
                tx[&planes[index]].content(hosted[index].at(EXTENT));
                if index + 1 < part_count {
                    tx[root].push(&parts[index + 1]);
                    tx[&parts[index + 1]].content(part_content(index + 1));
                }
            }
        });
        render_until(
            &fixture,
            &|| views.iter().all(|view| view_superview(view).is_some()),
            "the hosted planes",
        );
        assert_engine_sequence(&fixture, &foreign, plane_count, trailing_part);
        recompose_without_hierarchy_writes(&fixture, &parts[0], &foreign);
    }

    fn assert_engine_sequence(
        fixture: &Fixture,
        foreign: &NSView,
        plane_count: usize,
        trailing_part: bool,
    ) {
        let direct = fixture.view.subviews();
        let engine_stack: Vec<_> = direct
            .iter()
            .filter(|view| Retained::as_ptr(view) != std::ptr::from_ref(foreign))
            .filter_map(|view| view.layer())
            .filter(|layer| {
                layer
                    .name()
                    .is_none_or(|name| *name != *objc2_foundation::ns_string!("cherenkov-probes"))
            })
            .collect();
        let mut expected = Vec::new();
        for _ in 0..plane_count {
            expected.extend([true, false]);
        }
        if plane_count == 0 || trailing_part {
            expected.push(true);
        }
        assert_eq!(
            engine_stack
                .iter()
                .map(|layer| is_part(layer))
                .collect::<Vec<_>>(),
            expected,
            "engine sequence for {plane_count} planes, trailing={trailing_part}"
        );

        let probes_index = direct
            .iter()
            .position(|view| {
                view.layer().is_some_and(|layer| {
                    layer.name().is_some_and(|name| {
                        *name == *objc2_foundation::ns_string!("cherenkov-probes")
                    })
                })
            })
            .expect("the probes view");
        let first_part = engine_stack.first().expect("part zero");
        let first_part_index = direct
            .iter()
            .position(|view| {
                view.layer()
                    .is_some_and(|layer| same_layer(&layer, first_part))
            })
            .expect("part zero is an installed view");
        assert_eq!(first_part_index, probes_index + 1);
        let foreign_index = direct
            .iter()
            .position(|view| Retained::as_ptr(&view) == std::ptr::from_ref(foreign))
            .expect("the foreign sibling remains in the host");
        assert!(first_part_index < foreign_index);
    }

    fn recompose_without_hierarchy_writes(fixture: &Fixture, part: &Layer, foreign: &NSView) {
        let changed_content = fixture.window.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 96.0, 64.0),
                WorkingColor::new([0.9, 0.2, 0.1, 1.0]),
            );
        });
        fixture.counting.reset_writes();
        fixture.window.update(|tx| {
            tx[part].content(changed_content);
        });
        fixture.render();
        assert_eq!(fixture.counting.hierarchy_writes(), (0, 0));
        assert!(
            fixture
                .view
                .subviews()
                .iter()
                .any(|view| Retained::as_ptr(&view) == std::ptr::from_ref(foreign))
        );
    }

    /// The engine's views never answer a hit: a part painted above the
    /// hosted view lets every point through to it — which content
    /// occludes a hosted view is the host's decision, made in the view
    /// it hosts — and outside the hosted view the host view answers.
    fn engine_views_are_transparent_to_hits() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, holder, _hosted, _above, _above2] = hosted_scene(&fixture, &web);
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the hosted view",
        );
        let hit = |x: f64, y: f64| fixture.view.hitTest(CGPoint::new(x, y));
        let is_host = |v: Option<Retained<NSView>>| {
            v.as_ref().map(Retained::as_ptr) == Some(Retained::as_ptr(&fixture.view))
        };
        let is_web = |v: Option<Retained<NSView>>| {
            v.as_ref().map(Retained::as_ptr) == Some(Retained::as_ptr(&web))
        };
        // The host view is y-up: engine point (x,y) is host (x, 32-y).
        // The hosted view's clip is px (12,8)-(42,26) — pt (6,4)-(21,13)
        // — and the bar above it paints pt (15,5)-(20,13).
        assert!(
            is_web(hit(16.0, 22.0)),
            "the part painted over the hosted view lets the hit through"
        );
        assert!(
            is_web(hit(10.0, 22.0)),
            "the hosted view answers beside the bar"
        );
        assert!(
            is_host(hit(46.0, 17.0)),
            "outside the hosted view the host view takes the hit"
        );
        // Reorder the hosted plane above the part: the answers stay.
        let surface = &fixture.window;
        surface.update(|tx| {
            tx[surface.root()].push(&holder);
        });
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the hosted view on top",
        );
        assert!(
            is_web(hit(16.0, 22.0)),
            "the hosted view on top answers the hit"
        );
        assert!(
            is_host(hit(46.0, 17.0)),
            "the host view still takes outside hits"
        );
    }

    /// A part bounded below and above by hosted planes is transparent
    /// the same way: over its painted bar the hosted view below it
    /// answers, and the hosted view above it answers inside its own leaf.
    fn a_part_between_two_hosted_views_is_transparent_to_hits() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let web2 = hosted_view(mtm, (0.2, 0.2, 0.9));
        let surface = &fixture.window;
        let first = surface.layer();
        let mid = surface.layer();
        let second = surface.layer();
        let hosted1 = surface.layer();
        let hosted2 = surface.layer();
        let bar = surface.record(|c| {
            c.fill(
                Rect::new(30.0, 10.0, 40.0, 26.0),
                WorkingColor::new([0.7, 0.1, 0.1, 1.0]),
            );
        });
        let object1 = Hosted::<Gpu>::new(HostedView::new(web.clone(), mtm));
        let object2 = Hosted::<Gpu>::new(HostedView::new(web2.clone(), mtm));
        surface.update(|tx| {
            tx[surface.root()].push(&first).push(&mid).push(&second);
            tx[&first]
                .push(&hosted1)
                .transform(Affine::translate((12.0, 8.0)))
                .clip(RoundedRect::new(0.0, 0.0, 30.0, 18.0, 4.0));
            tx[&hosted1].content(object1.at(EXTENT));
            tx[&mid]
                .content(bar)
                .clip(RoundedRect::new(30.0, 10.0, 40.0, 26.0, 0.0));
            tx[&second]
                .push(&hosted2)
                .transform(Affine::translate((60.0, 8.0)))
                .clip(RoundedRect::new(0.0, 0.0, 30.0, 18.0, 4.0));
            tx[&hosted2].content(object2.at(EXTENT));
        });
        render_until(
            &fixture,
            &|| view_superview(&web).is_some() && view_superview(&web2).is_some(),
            "both hosted views",
        );
        let hit = |x: f64, y: f64| fixture.view.hitTest(CGPoint::new(x, y));
        // The bar between the planes paints pt (15,5)-(20,13), host
        // (16,22): the lower hosted view under it answers.
        assert_eq!(
            hit(16.0, 22.0).as_ref().map(Retained::as_ptr),
            Some(Retained::as_ptr(&web)),
            "the bar between the hosted views lets the hit through"
        );
        // The upper hosted view's leaf — pt (30,4)-(36,13), engine
        // (32,10) — answers above the part.
        assert_eq!(
            hit(32.0, 22.0).as_ref().map(Retained::as_ptr),
            Some(Retained::as_ptr(&web2)),
            "the upper hosted view answers"
        );
    }

    fn contents_orientation_updates_engine_layer_geometry_in_place() {
        let fixture = Fixture::new();
        let buffer = bgra_buffer();
        let offscreen = fixture
            .engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
            .expect("offscreen");
        let _engine_scene = scene_bar(
            &fixture.engine,
            &offscreen,
            bgra(&fixture.metal, &buffer, FrameColor::SRGB),
            1.0,
        );
        let window_scene = scene_bar(
            &fixture.engine,
            &fixture.window,
            bgra(&fixture.metal, &buffer, FrameColor::SRGB),
            1.0,
        );
        assert!(fixture.promote(), "the frame never became ready");
        fixture.render();

        let engine_layers = stack(&fixture);
        assert_eq!(engine_layers.len(), 3, "two parts surround the frame");
        assert_eq!(
            engine_layers.iter().filter(|layer| is_part(layer)).count(),
            2
        );
        let engine_pointers: Vec<_> = engine_layers.iter().map(Retained::as_ptr).collect();
        let initial_geometry: Vec<_> = engine_layers
            .iter()
            .map(|layer| layer.isGeometryFlipped())
            .collect();
        let probe_layer = fixture
            .view
            .subviews()
            .iter()
            .find_map(|view| {
                let layer = view.layer()?;
                layer
                    .name()
                    .is_some_and(|name| *name == *objc2_foundation::ns_string!("cherenkov-probes"))
                    .then_some(layer)
            })
            .expect("the engine probes view");
        let probe_pointer = Retained::as_ptr(&probe_layer);
        let initial_probe_geometry = probe_layer.isGeometryFlipped();

        fixture
            .counting
            .orientation_layer()
            .set_contents_flipped(true);
        let changed = fixture.window.record(|c| {
            c.fill(
                Rect::new(30.0, 10.0, 40.0, 26.0),
                WorkingColor::new([0.8, 0.2, 0.1, 1.0]),
            );
        });
        fixture.window.update(|tx| {
            tx[&window_scene[3]].content(changed);
        });
        fixture.render();

        let updated_layers = stack(&fixture);
        assert_eq!(
            updated_layers
                .iter()
                .map(Retained::as_ptr)
                .collect::<Vec<_>>(),
            engine_pointers
        );
        assert_eq!(
            updated_layers
                .iter()
                .map(|layer| layer.isGeometryFlipped())
                .collect::<Vec<_>>(),
            initial_geometry
                .iter()
                .map(|flipped| !flipped)
                .collect::<Vec<_>>()
        );
        let updated_probe_layer = fixture
            .view
            .subviews()
            .iter()
            .find_map(|view| {
                let layer = view.layer()?;
                layer
                    .name()
                    .is_some_and(|name| *name == *objc2_foundation::ns_string!("cherenkov-probes"))
                    .then_some(layer)
            })
            .expect("the engine probes view remains installed");
        assert_eq!(Retained::as_ptr(&updated_probe_layer), probe_pointer);
        assert_eq!(
            updated_probe_layer.isGeometryFlipped(),
            !initial_probe_geometry
        );
    }

    /// A steady frame writes nothing to the view hierarchy: `order`
    /// moves only the views out of place, and a hosted view that is the
    /// window's first responder keeps the status across renders and a
    /// reorder — the diff never touches its ancestors.
    fn a_steady_frame_mutates_no_views_and_keeps_the_first_responder() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, _holder, _hosted, above, _above2] = hosted_scene(&fixture, &web);
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the hosted view",
        );
        let responder: &objc2_app_kit::NSResponder = &web;
        assert!(fixture.app_window.makeFirstResponder(Some(responder)));
        fixture.counting.reset_writes();
        let changed_content = fixture.window.record(|c| {
            c.fill(
                Rect::new(30.0, 10.0, 40.0, 26.0),
                WorkingColor::new([0.1, 0.2, 0.8, 1.0]),
            );
        });
        fixture.window.update(|tx| {
            tx[&above].content(changed_content);
        });
        fixture.render();
        assert_eq!(
            fixture.counting.hierarchy_writes(),
            (0, 0),
            "a real content compose made no hierarchy writes"
        );
        assert!(
            fixture
                .app_window
                .firstResponder()
                .is_some_and(|r| r.isEqual(Some(&*web))),
            "a real content compose kept first responder"
        );
        let second_web = hosted_view(mtm, (0.2, 0.2, 0.9));
        let second_plane = fixture.window.layer();
        let trailing_part = fixture.window.layer();
        let second_content = Hosted::<Gpu>::new(HostedView::new(second_web.clone(), mtm));
        let trailing_content = fixture.window.record(|c| {
            c.fill(
                Rect::new(70.0, 40.0, 80.0, 50.0),
                WorkingColor::new([0.1, 0.8, 0.2, 1.0]),
            );
        });
        fixture.window.update(|tx| {
            tx[fixture.window.root()]
                .push(&second_plane)
                .push(&trailing_part);
            tx[&second_plane].content(second_content.at(EXTENT));
            tx[&trailing_part].content(trailing_content);
        });
        render_until(
            &fixture,
            &|| view_superview(&second_web).is_some(),
            "the trailing hosted plane",
        );
        reorder_trailing_part_around_responder(&fixture, mtm, &web, &second_web, &trailing_part);
    }

    fn reorder_trailing_part_around_responder(
        fixture: &Fixture,
        mtm: MainThreadMarker,
        web: &Retained<NSView>,
        second_web: &Retained<NSView>,
        trailing_part: &Layer,
    ) {
        let foreign = NSView::new(mtm);
        fixture.view.addSubview(&foreign);
        let responder: &objc2_app_kit::NSResponder = web;
        assert!(
            fixture.app_window.makeFirstResponder(Some(responder)),
            "the hosted view takes first responder"
        );
        let stable_host = top_ancestor(web, &fixture.view);
        let second_host = top_ancestor(second_web, &fixture.view);
        let before = direct_subviews(&fixture.view);
        let stable_index = before
            .iter()
            .position(|view| std::ptr::eq(*view, stable_host))
            .expect("the first responder's ancestor is installed");
        let second_index = before
            .iter()
            .position(|view| std::ptr::eq(*view, second_host))
            .expect("the trailing plane is installed");
        assert!(before.contains(&Retained::as_ptr(&foreign)));
        let trailing_view = before[second_index + 1];
        fixture.counting.reset_writes();
        fixture.window.update(|tx| {
            tx[fixture.window.root()]
                .remove(trailing_part)
                .insert(4, trailing_part);
        });
        fixture.render();
        let hierarchy_writes = fixture.counting.hierarchy_changes();
        assert!(
            hierarchy_writes
                .iter()
                .all(|(_, identity)| *identity != stable_host.addr()),
            "the reorder never moved the first-responder view's ancestor"
        );
        let after = direct_subviews(&fixture.view);
        assert_ne!(before, after, "the engine view order changed");
        assert_eq!(
            after[stable_index], stable_host,
            "the first responder's ancestor stayed in place"
        );
        assert!(
            before.contains(&trailing_view) && !after.contains(&trailing_view),
            "moving content across the trailing plane detached its engine suffix"
        );
        assert!(
            hierarchy_writes
                .iter()
                .any(|(added, identity)| !added && *identity == trailing_view.addr()),
            "the detached engine part identity was recorded"
        );
        assert!(after.contains(&Retained::as_ptr(&foreign)));
        fixture.window.update(|tx| {
            tx[fixture.window.root()]
                .remove(trailing_part)
                .push(trailing_part);
        });
        fixture.render();
        let restored = direct_subviews(&fixture.view);
        assert!(
            restored.contains(&trailing_view),
            "the trailing part reuses its previous view"
        );
        assert!(
            fixture
                .counting
                .hierarchy_changes()
                .iter()
                .any(|(added, identity)| *added && *identity == trailing_view.addr()),
            "the reinserted engine part identity was recorded"
        );
        assert_eq!(restored[stable_index], stable_host);
        assert!(
            fixture
                .app_window
                .firstResponder()
                .is_some_and(|r| r.isEqual(Some(&**web))),
            "the hosted view kept first responder"
        );
    }

    /// A plane inserted below the hosted plane that holds first responder
    /// moves the engine's views around the hosted plane's own, which
    /// stay where they are: the responder keeps its status through the
    /// reorder, with no resignation in between.
    fn a_plane_inserted_below_the_responder_leaves_its_views_in_place() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, _holder, _hosted, _above, _above2] = hosted_scene(&fixture, &web);
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the hosted view",
        );
        let responder: &objc2_app_kit::NSResponder = &web;
        assert!(fixture.app_window.makeFirstResponder(Some(responder)));
        let stable_host = top_ancestor(&web, &fixture.view);
        let inserted_web = hosted_view(mtm, (0.2, 0.2, 0.9));
        let inserted = fixture.window.layer();
        let object = Hosted::<Gpu>::new(HostedView::new(inserted_web.clone(), mtm));
        fixture.counting.reset_writes();
        fixture.window.update(|tx| {
            tx[fixture.window.root()].insert(1, &inserted);
            tx[&inserted].content(object.at(EXTENT));
        });
        render_until(
            &fixture,
            &|| view_superview(&inserted_web).is_some(),
            "the inserted plane",
        );
        let order = direct_subviews(&fixture.view);
        let position = |view: *const NSView| {
            order
                .iter()
                .position(|installed| std::ptr::eq(*installed, view))
                .expect("an installed engine view")
        };
        assert!(
            position(top_ancestor(&inserted_web, &fixture.view)) < position(stable_host),
            "the inserted plane sits below the responder's"
        );
        assert!(
            fixture
                .counting
                .hierarchy_changes()
                .iter()
                .all(|(_, identity)| *identity != stable_host.addr()),
            "the reorder never moved the responder's hosted plane"
        );
        assert!(
            fixture
                .app_window
                .firstResponder()
                .is_some_and(|r| r.isEqual(Some(&*web))),
            "the hosted view kept first responder"
        );
    }

    /// A clip added on the hosted layer's own level changes its path's
    /// shape: the plane's views are rebuilt and the leaf moves under the
    /// new clip view, which resigns a first responder inside it — the
    /// frame's commit hands the status back.
    fn a_rebuilt_hosted_path_gives_first_responder_back() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, _holder, hosted, _above, _above2] = hosted_scene(&fixture, &web);
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the hosted view",
        );
        let responder: &objc2_app_kit::NSResponder = &web;
        assert!(fixture.app_window.makeFirstResponder(Some(responder)));
        let leaf = as_view(view_superview(&web).expect("the leaf"));
        let clip = view_superview(leaf).expect("the holder's clip view");
        fixture.window.update(|tx| {
            tx[&hosted].clip(RoundedRect::new(0.0, 0.0, 20.0, 12.0, 0.0));
        });
        render_until(
            &fixture,
            &|| view_superview(leaf).is_some_and(|parent| parent != clip),
            "the rebuilt path",
        );
        assert!(
            fixture
                .app_window
                .firstResponder()
                .is_some_and(|r| r.isEqual(Some(&*web))),
            "the hosted view has first responder back"
        );
    }

    /// A layer holding an external frame then a hosted view — and back
    /// — rebuilds its native nodes each way: the reuse key includes the
    /// plane kind, so a `Layers` path never keeps serving a `Views`
    /// plane or the reverse.
    fn a_layer_switching_between_frame_and_hosted_rebuilds_its_nodes() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let surface = &fixture.window;
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let layer = surface.layer();
        let (video, sink) = fixture.engine.frame_producer();
        sink.submit(bgra(&fixture.metal, &bgra_buffer(), FrameColor::SRGB));
        surface.update(|tx| {
            tx[surface.root()].push(&layer);
            tx[&layer].content(video.at(VIDEO_SIZE));
        });
        // The frame plane exists before the switch: the kind change then
        // exercises real reuse, not a first build.
        assert!(
            fixture.promote(),
            "the platform never reported the frame plane ready"
        );
        assert_eq!(displays(&fixture.root()).len(), 1, "the frame plane exists");
        assert!(view_superview(&web).is_none(), "a frame plane is layers");

        let object = Hosted::<Gpu>::new(HostedView::new(web.clone(), mtm));
        surface.update(|tx| {
            tx[&layer].content(object.at(EXTENT));
        });
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the frame-to-hosted switch",
        );

        let (video, sink) = fixture.engine.frame_producer();
        sink.submit(bgra(&fixture.metal, &bgra_buffer(), FrameColor::SRGB));
        surface.update(|tx| {
            tx[&layer].content(video.at(VIDEO_SIZE));
        });
        assert!(
            fixture.promote(),
            "the hosted-to-frame switch promotes again"
        );
        assert_eq!(
            view_superview(&web),
            None,
            "the hosted-to-frame switch released the view"
        );
    }

    /// A scrolled hosted view is hittable exactly where it is visible:
    /// the leaf's frame is the extent at its scrolled position — the
    /// scroll rides the leaf, not the hosted view — so the hosted view
    /// always sits at the leaf's origin at its own extent, and the
    /// ancestor clip bounds the visible strip on either scroll sign.
    fn a_scrolled_hosted_view_hits_only_where_it_is_visible() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, holder, hosted, _above, _above2] = hosted_scene(&fixture, &web);
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the hosted view",
        );
        let leaf = as_view(view_superview(&web).expect("the leaf"));
        let surface = &fixture.window;
        for scroll in [10.0, -10.0] {
            fixture.window.update(|tx| {
                tx[surface.root()].push(&holder);
                tx[&hosted].scroll_offset(Vec2::new(scroll, 0.0));
            });
            // The leaf's frame carries the scroll's shift — `-scroll`
            // inside the clip view, in the content's points.
            render_until(
                &fixture,
                &|| (leaf.frame().origin.x + scroll).abs() < 1e-6,
                "the scrolled leaf",
            );
            // The leaf's bounds are the whole extent and the hosted view
            // sits at its origin — never shifted or resized.
            let bounds = leaf.bounds();
            assert_eq!((bounds.origin.x, bounds.origin.y), (0.0, 0.0));
            assert_eq!((bounds.size.width, bounds.size.height), (40.5, 24.0));
            let frame = web.frame();
            assert_eq!((frame.origin.x, frame.origin.y), (0.0, 0.0));
            assert_eq!((frame.size.width, frame.size.height), (40.5, 24.0));
            // A point inside the shifted strip reaches the hosted view.
            // Scroll +10 leaves the leaf at clip-space x=-10, so the
            // hosted content covers px (2,8)-(42.5,32); scroll -10 puts
            // it at +10 — px (22,8)-(62.5,32). Engine px (32,20) — pt
            // (16,10), host (16,22) — is inside the strip either way.
            let inside = fixture.view.hitTest(CGPoint::new(16.0, 22.0));
            assert_eq!(
                inside.as_ref().map(Retained::as_ptr),
                Some(Retained::as_ptr(&web)),
                "the shifted strip is hittable at scroll {scroll}"
            );
            // Off the strip: past the clip's right edge (px 44) for
            // positive scroll, and inside the clip but left of the leaf
            // (px 20 < 22) for negative — the host view.
            let off = fixture
                .view
                .hitTest(CGPoint::new(if scroll > 0.0 { 22.0 } else { 10.0 }, 22.0));
            assert_eq!(
                off.as_ref().map(Retained::as_ptr),
                Some(Retained::as_ptr(&fixture.view)),
                "outside the shifted strip at scroll {scroll} is the host's"
            );
        }
    }

    /// A negative or a zero scale on the hosted plane's path fails the
    /// render naming the transform rule: a view's bounds size carries
    /// only a positive axis-aligned scale, and a scale animating through
    /// zero is unplaceable for its duration.
    fn a_negative_or_zero_scale_on_the_path_is_unplaceable() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, holder, hosted, _above, _above2] = hosted_scene(&fixture, &web);
        for transform in [
            Affine::scale_non_uniform(-1.0, 1.0),
            Affine::scale_non_uniform(1.0, -1.0),
            Affine::scale_non_uniform(0.0, 1.0),
        ] {
            fixture.window.update(|tx| {
                tx[&holder].transform(transform);
            });
            drain_main();
            match fixture.engine.render(FrameTime::now()) {
                Err(RenderError::Unplaceable { layer, reason }) => {
                    assert_eq!(layer, hosted.id());
                    assert!(reason.contains("transform"), "{reason}");
                }
                other => panic!("an unplaceable hosted view, got {other:?}"),
            }
            assert!(view_superview(&web).is_none(), "never shown");
        }
    }

    /// A rotation and a skew on the hosted plane's path each fail the
    /// render naming the transform rule: a view carries only a
    /// translation and an axis-aligned scale.
    fn a_rotation_and_a_skew_on_the_path_are_unplaceable() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, holder, hosted, _above, _above2] = hosted_scene(&fixture, &web);
        for transform in [Affine::rotate(0.4), Affine::skew(0.2, 0.0)] {
            fixture.window.update(|tx| {
                tx[&holder].transform(transform);
            });
            drain_main();
            match fixture.engine.render(FrameTime::now()) {
                Err(RenderError::Unplaceable { layer, reason }) => {
                    assert_eq!(layer, hosted.id());
                    assert!(reason.contains("transform"), "{reason}");
                }
                other => panic!("an unplaceable hosted view, got {other:?}"),
            }
            assert!(view_superview(&web).is_none(), "never shown");
        }
    }

    /// Opacity below one on the hosted layer renders with `alphaValue`
    /// on the leaf, the engine's own view: `AppKit` owns the backing
    /// layer, the engine owns the view.
    fn opacity_below_one_renders_with_alpha_value() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, _holder, hosted, _above, _above2] = hosted_scene(&fixture, &web);
        fixture.window.update(|tx| {
            tx[&hosted].opacity(0.5f32);
        });
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the hosted view",
        );
        let leaf = as_view(view_superview(&web).expect("the leaf"));
        assert!((leaf.alphaValue() - 0.5).abs() < 1e-6);
    }

    /// Scroll moves the bounds origin and a scale the bounds size of the
    /// clip view, with the same view objects and no rebuild; a second
    /// binding of the same view elsewhere moves it.
    fn scroll_and_scale_update_the_bounds_in_place() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, holder, hosted, _above, _above2] = hosted_scene(&fixture, &web);
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the hosted view",
        );
        let leaf_ptr = view_superview(&web).expect("the leaf");
        let leaf = as_view(leaf_ptr);
        let clip = as_view(view_superview(leaf).expect("the clip"));

        fixture.window.update(|tx| {
            tx[&holder].scroll_offset(Vec2::new(3.0, 5.0));
        });
        render_until(
            &fixture,
            &|| (clip.bounds().origin.x - 3.0).abs() < 1e-12,
            "the scroll",
        );
        let bounds = clip.bounds();
        assert!((bounds.origin.x - 3.0).abs() < 1e-12);
        assert!((bounds.origin.y - 5.0).abs() < 1e-12);
        assert!((bounds.size.width - 30.0).abs() < 1e-12);

        fixture.window.update(|tx| {
            tx[&holder].transform(Affine::scale(1.5));
        });
        render_until(
            &fixture,
            &|| (clip.frame().size.width - 45.0).abs() < 1e-12,
            "the scale",
        );
        assert!((clip.frame().size.width - 45.0).abs() < 1e-12);
        assert!((clip.bounds().size.width - 30.0).abs() < 1e-12);
        assert_eq!(view_superview(&web), Some(leaf_ptr));

        // Binding the same view on a second layer moves it to the new
        // leaf; the first plane's views leave with it.
        let first_leaf_ptr = leaf_ptr;
        let second = fixture.window.layer();
        let object = Hosted::<Gpu>::new(HostedView::new(web.clone(), mtm));
        fixture.window.update(|tx| {
            tx[fixture.window.root()].push(&second);
            tx[&hosted].clear_content();
            tx[&second].content(object.at(EXTENT));
        });
        render_until(
            &fixture,
            &|| view_superview(&web).is_some_and(|p| p != first_leaf_ptr),
            "the move to a second binding",
        );
    }

    /// A hosted layer under an isolating ancestor cannot be placed: the
    /// render fails naming it and the rule, every render after that fails
    /// too, and the first render after the ancestor stops isolating shows
    /// it.
    fn an_unplaceable_hosted_layer_fails_every_render_until_placeable() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let fixture = Fixture::new();
        let web = hosted_view(mtm, (0.1, 0.9, 0.2));
        let [_below, holder, hosted, _above, _above2] = hosted_scene(&fixture, &web);
        fixture.window.update(|tx| {
            tx[&holder].opacity(0.5f32);
        });
        for _ in 0..2 {
            drain_main();
            match fixture.engine.render(FrameTime::now()) {
                Err(RenderError::Unplaceable { layer, reason }) => {
                    assert_eq!(layer, hosted.id());
                    assert!(reason.contains("isolates"), "{reason}");
                }
                other => panic!("an unplaceable hosted layer, got {other:?}"),
            }
        }
        assert!(view_superview(&web).is_none(), "never shown");
        fixture.window.update(|tx| {
            tx[&holder].opacity(1.0f32);
        });
        render_until(
            &fixture,
            &|| view_superview(&web).is_some(),
            "the placeable view",
        );
        let layers = stack(&fixture);
        assert!(
            matches!(&layers[..], [first, plane, second, ..]
                if is_part(first)
                    && !is_part(plane)
                    && is_part(second)),
            "part, plane, part"
        );
    }

    /// Every frame's present lands inside its own `engine.render`, on this
    /// thread: #2261's flood — renders back to back with no runloop turn
    /// between them — so a frame's acquire never meets its predecessor's
    /// drawable still acquired while a present block sits queued on the
    /// main queue. `Next::Idle` per frame asserts each presented: a
    /// `Retry` would have re-armed the surface's redraw.
    fn back_to_back_frames_present_without_a_runloop_turn() {
        let fixture = Fixture::new();
        let layer = fixture.window.layer();
        let fill = fixture.window.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 16.0, 16.0),
                WorkingColor::new([0.2, 0.4, 0.8, 1.0]),
            );
        });
        fixture.window.update(|tx| {
            tx[fixture.window.root()].push(&layer);
            tx[&layer].content(fill);
        });
        for i in 0..32 {
            fixture.window.update(|tx| {
                tx[&layer].transform(Affine::translate((f64::from(i), 0.0)));
            });
            assert_eq!(
                fixture.engine.render(FrameTime::now()).expect("rendered"),
                cherenkov::Next::Idle,
                "frame {i} did not present"
            );
        }
    }

    /// A surface with no system-compositor parent cannot show a hosted
    /// layer: its render fails, and nothing composites the layer instead.
    fn a_hosted_layer_without_planes_fails_the_render() {
        let mtm = MainThreadMarker::new().expect("the cases run on the main thread");
        let metal = metal();
        let engine = Engine::<Gpu>::new(GpuConfig {
            device: Some(metal.shared),
            ..GpuConfig::default()
        })
        .expect("an engine");
        let surface = engine
            .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
            .expect("offscreen");
        let web = surface.layer();
        let object = Hosted::<Gpu>::new(HostedView::new(hosted_view(mtm, (0.0, 1.0, 0.0)), mtm));
        surface.update(|tx| {
            tx[surface.root()].push(&web);
            tx[&web].content(object.at(EXTENT));
        });
        match engine.render(FrameTime::now()) {
            Err(RenderError::Unplaceable { layer, reason }) => {
                assert_eq!(layer, web.id());
                assert_eq!(reason, "the surface has no system-compositor parent");
            }
            other => panic!("an unplaceable hosted layer, got {other:?}"),
        }
    }
}
