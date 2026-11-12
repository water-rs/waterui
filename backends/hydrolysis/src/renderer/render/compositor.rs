//! The renderer's engine frame: its one window mount (§A) committed and
//! rendered into the target — a GPU texture or window on the GPU painter, an
//! offscreen framebuffer on the CPU painter.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use std::rc::Rc;
#[cfg(hydrolysis_gpu_painter)]
use std::sync::Arc;

/// The frame description every GPU render path works on — the texture frame
/// a host presents and the engine-presented window frames: the device
/// generation's handles and engine, the display's scale, and the target's
/// size and base colour.
#[cfg(hydrolysis_gpu_painter)]
pub struct FrameRenderTarget<'a> {
    pub device: &'a wgpu::Device,
    pub device_loss: crate::platform::DeviceLoss,
    /// The device-creation chain the frame's handles belong to.
    pub gpu_context_id: u64,
    /// All four handles of that chain; the engine requires them to come
    /// from the same creation chain.
    pub shared_device: cherenkov_gpu::interop::SharedDevice,
    /// The engine the generation created when it opened; every surface on
    /// the generation renders through it.
    pub engine: &'a Rc<crate::engine::SharedEngineState>,
    /// Device pixels per logical unit on the display the target shows on.
    pub display_scale: f64,
    pub width: u32,
    pub height: u32,
    pub base_color: cherenkov_record::WorkingColor,
}

/// What a frame commit needs on either Cherenkov backend: the target's size
/// in device pixels, the display's scale, and the colour under the content.
#[derive(Clone, Copy, Debug)]
pub struct FrameGeometry {
    /// Device pixels per logical unit on the display the target shows on.
    pub display_scale: f64,
    pub width: u32,
    pub height: u32,
    pub base_color: cherenkov_record::WorkingColor,
}

#[cfg(hydrolysis_gpu_painter)]
impl<'a> FrameRenderTarget<'a> {
    /// The frame `generation` renders at `size` (device pixels) on a display
    /// of `display_scale`, over `base_color`.
    pub(crate) fn new(
        generation: &'a crate::platform::GpuGeneration,
        display_scale: f64,
        (width, height): (u32, u32),
        base_color: cherenkov_record::WorkingColor,
    ) -> Self {
        let context = generation.gpu_context();
        Self {
            device: &context.shared_device.device,
            device_loss: generation.loss().clone(),
            gpu_context_id: context.context_id,
            shared_device: context.shared_device.clone(),
            engine: generation.engine(),
            display_scale,
            width,
            height,
            base_color,
        }
    }

    /// The frame's geometry, as the engine-generic commit takes it.
    const fn geometry(&self) -> FrameGeometry {
        FrameGeometry {
            display_scale: self.display_scale,
            width: self.width,
            height: self.height,
            base_color: self.base_color,
        }
    }
}

#[cfg(hydrolysis_gpu_painter)]
impl HydrolysisRenderer<crate::text::ParleyEngine> {
    crate::engine::cfg_async_fn! {
        /// Renders the frame into `texture`, a frame `surface` handed out,
        /// over `base_color`: an SDR, single-scale render at the surface's
        /// size and format through the engine of the surface's device
        /// generation — the headless contract `waterui-testing` drives.
        ///
        /// Async on wasm32, where the surface render inside awaits the
        /// browser device.
        ///
        /// # Errors
        ///
        /// [`FrameError`] when the engine cannot create the frame's surface
        /// or the frame fails to render.
        pub fn render_scene_to_texture(
            &mut self,
            surface: &crate::platform::OffscreenSurface,
            texture: &wgpu::Texture,
            base_color: cherenkov_record::WorkingColor,
        ) -> Result<(), FrameError> {
            crate::engine::engine_await!(self.render_generation_to_texture(
                surface.generation(),
                texture,
                surface.format(),
                surface.size(),
                base_color,
            ))
        }
    }

    crate::engine::cfg_async_fn! {
        /// Renders an SDR, single-scale frame of `size` (device pixels)
        /// through `generation`'s engine and copies it into `texture`, whose
        /// format is `format`.
        ///
        /// Async on wasm32, where the surface render inside awaits the browser
        /// device.
        ///
        /// # Errors
        ///
        /// [`FrameError`] when the engine cannot create the frame's surface
        /// or the frame fails to render.
        pub(crate) fn render_generation_to_texture(
            &mut self,
            generation: &crate::platform::GpuGeneration,
            texture: &wgpu::Texture,
            format: wgpu::TextureFormat,
            size: (u32, u32),
            base_color: cherenkov_record::WorkingColor,
        ) -> Result<(), FrameError> {
            let target = FrameRenderTarget::new(generation, 1.0, size, base_color);
            let (device, queue) = (target.device, &generation.gpu_context().shared_device.queue);
            let frame = crate::engine::engine_await!(
                self.render_texture_frame(target, format, 1.0)
            )?;
            self.present_engine_frame(
                frame,
                device,
                queue,
                texture,
                crate::engine::format_output_color(texture.format()),
                cherenkov_gpu::interop::OutputAlpha::Straight,
            );
            Ok(())
        }
    }

    /// Copies the frame [`Self::render_texture_frame`] rendered into
    /// `texture`. Synchronous on every target, so a host presenting a
    /// swapchain image acquires it only after the engine render, and the
    /// image is never held across an await: a browser expires its canvas
    /// texture when the task that acquired it ends.
    pub(crate) fn present_engine_frame(
        &mut self,
        EngineFrame { headroom, .. }: EngineFrame,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        color: cherenkov_gpu::interop::OutputColor,
        alpha: cherenkov_gpu::interop::OutputAlpha,
    ) {
        // The window map leaves `self` for the call, as in the render, so the
        // profiler mark may borrow the renderer.
        let Some(mut window) = self.cherenkov_window.take() else {
            // The `EngineFrame` is proof its window rendered it; a host that
            // replaced the window in between presents nothing this frame.
            waterui_core::contract_violation!(
                "hydrolysis renderer: the engine frame's window left the renderer before its present; the frame is not presented"
            );
            return;
        };
        window
            .surface
            .present_into(device, queue, texture, color, alpha, headroom);

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(window.gpu_profiler.as_ref(), device, queue, 2);

        self.cherenkov_window = Some(window);
    }

    crate::engine::cfg_async_fn! {
        /// Commits the window's pending node programs through its mount and
        /// renders the engine frame (§A) into the engine texture a host
        /// presents as a `format` frame at `headroom`. The window — engine
        /// surface plus mount — lives per device-creation chain: a new chain
        /// or a lost device replaces it, and the new mount remounts every
        /// node.
        ///
        /// Async on wasm32, where the engine calls inside await the browser
        /// device.
        ///
        /// # Errors
        ///
        /// [`FrameError`] when the format is not one the engine presents
        /// into, the engine cannot create the window's surface, or the frame
        /// fails to render.
        pub(crate) fn render_texture_frame(
            &mut self,
            target: FrameRenderTarget<'_>,
            format: wgpu::TextureFormat,
            headroom: f32,
        ) -> Result<EngineFrame, FrameError> {
        let supported = matches!(
            format.remove_srgb_suffix(),
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
        ) || matches!(
            format,
            wgpu::TextureFormat::Rgba16Float
                | wgpu::TextureFormat::Rgba32Float
                | wgpu::TextureFormat::Rgb10a2Unorm
        );
        if !supported {
            return Err(FrameError::UnsupportedFormat(format));
        }

        let _render_span = tracing::debug_span!("hydrolysis_render_scene").entered();
        if !self
            .cherenkov_window
            .as_ref()
            .is_some_and(|window| window.renders_on(&target))
        {
            if self.cherenkov_window.is_some() {
                self.unmount_for_window_replacement();
            }
            self.cherenkov_window = None;
            let surface = crate::engine::engine_await!(crate::engine::TextureCherenkovSurface::new(
                target.engine,
                target.device,
                (target.width, target.height),
                self.host_wake(),
            ))
            .map_err(FrameError::Surface)?;
            self.cherenkov_window = Some(CherenkovWindow::new(
                surface,
                &target,
                Arc::clone(&self.applied_filter_metrics),
                Rc::clone(&self.material_registry),
            ));
        }
        let mut window = self
            .cherenkov_window
            .take()
            .expect("hydrolysis renderer: the window was just ensured");
        let _frame = self.commit_window_frame(&mut window, &target, headroom);

        self.applied_filter_metrics.reset();
        let rendered = crate::engine::engine_await!(window.surface.render(self.frame_time()));
        self.take_applied_filter_metrics();
        self.engine_tracks = crate::engine::EngineSurface::running_tracks(&window.surface);
        self.cherenkov_window = Some(window);
        self.engine_next = Some(rendered?.into());
        let content_range = self.cherenkov_window.as_ref().expect("rendered window").surface.content_range();
        Ok(EngineFrame { headroom, content_range })
    }
    }

    /// Commits the window's pending node programs through the macOS winit
    /// window's mount and renders the engine frame (§A), which the engine
    /// presents through the window's `WindowTarget`. The engine window
    /// lives in `engine_target`'s slot per device-creation chain and
    /// transparency: a new chain, a lost device or a transparency switch
    /// replaces it, and the new mount remounts every node.
    ///
    /// # Errors
    ///
    /// [`FrameError`] when the engine cannot create the window's surface or
    /// the frame fails to render.
    #[cfg(all(target_os = "macos", hydrolysis_winit))]
    pub(crate) fn render_window_frame(
        &mut self,
        target: &FrameRenderTarget<'_>,
        engine_target: crate::platform::EngineWindowTarget<'_>,
    ) -> Result<(), FrameError> {
        let crate::platform::EngineWindowTarget {
            window: native_window,
            window_slot,
            transparent,
            display_moved,
        } = engine_target;
        let _render_span = tracing::debug_span!("hydrolysis_render_scene").entered();
        if !window_slot.as_ref().is_some_and(|window| {
            window.renders_on(target) && window.surface.transparent() == transparent
        }) {
            if window_slot.is_some() {
                self.unmount_for_window_replacement();
            }
            *window_slot = None;
            let surface = crate::engine::WindowCherenkovSurface::new(
                Rc::clone(&target.engine.engine),
                Arc::clone(native_window),
                (target.width, target.height),
                crate::platform::hydrolysis_output_request(transparent),
                self.host_wake(),
            )
            .map_err(FrameError::Surface)?;
            *window_slot = Some(CherenkovWindow::new(
                surface,
                target,
                Arc::clone(&self.applied_filter_metrics),
                Rc::clone(&self.material_registry),
            ));
        }
        let mut window = window_slot
            .take()
            .expect("hydrolysis renderer: the window was just ensured");
        let (headroom, _) = window.surface.headroom();
        let _frame = self.commit_window_frame(&mut window, target, headroom);
        if display_moved {
            window.surface.display_moved();
        }

        let at = self.frame_time();
        let rendered = self.render_presented_window(&mut window, target, |surface| {
            surface.render(at, target.display_scale)
        });
        *window_slot = Some(window);
        Ok(rendered?)
    }

    /// Commits directly into the Android compositor target, without a
    /// host-acquired texture or a full-window presentation copy.
    ///
    /// # Errors
    ///
    /// [`FrameError`] when the window's compositor surface cannot be
    /// created or the frame fails to render.
    #[cfg(all(target_os = "android", not(hydrolysis_hwui)))]
    pub(crate) fn render_android_frame(
        &mut self,
        target: &FrameRenderTarget<'_>,
        engine_target: crate::platform::EngineAndroidTarget<'_>,
    ) -> Result<(), FrameError> {
        let crate::platform::EngineAndroidTarget {
            window: native_window,
            window_slot,
            transparent,
            band_commits,
        } = engine_target;
        if !window_slot
            .as_ref()
            .is_some_and(|window| window.renders_on(target))
        {
            // The attachment may already have dropped the old window when
            // SurfaceHolder destroyed it. Remount its surviving node tree.
            self.unmount_for_window_replacement();
            *window_slot = None;
            let surface = crate::engine::AndroidCherenkovSurface::new(
                Rc::clone(&target.engine.engine),
                native_window,
                (target.width, target.height),
                self.host_wake(),
                transparent,
                band_commits.clone(),
            )?;
            *window_slot = Some(CherenkovWindow::new(
                surface,
                target,
                Arc::clone(&self.applied_filter_metrics),
                Rc::clone(&self.material_registry),
            ));
        }
        let window = window_slot
            .as_mut()
            .expect("Android engine window was just ensured");
        // The Vulkan host has no display luminance query. Preserve its
        // existing display range; native buffers retain source HDR metadata.
        let _frame =
            self.commit_window_frame(window, target, cherenkov::Display::default().headroom);
        let time = self.frame_time();
        Ok(self.render_presented_window(window, target, |surface| surface.render(time))?)
    }

    /// The common completion path for engine-presented native windows.
    #[cfg(any(
        all(target_os = "android", not(hydrolysis_hwui)),
        all(target_os = "macos", hydrolysis_winit)
    ))]
    fn render_presented_window<S: crate::engine::EngineSurface>(
        &mut self,
        window: &mut CherenkovWindow<S, crate::text::ParleyEngine>,
        target: &FrameRenderTarget<'_>,
        render: impl FnOnce(&mut S) -> Result<cherenkov::Next, cherenkov::RenderError>,
    ) -> Result<(), cherenkov::RenderError> {
        self.applied_filter_metrics.reset();
        let rendered = render(&mut window.surface);
        self.take_applied_filter_metrics();
        #[cfg(feature = "frame-profile")]
        {
            let queue = &target.shared_device.queue;
            self.gpu_profile_mark(window.gpu_profiler.as_ref(), target.device, queue, 2);
            self.finish_gpu_frame_profile_with(window.gpu_profiler.as_ref(), target.device, queue);
        }
        #[cfg(not(feature = "frame-profile"))]
        let _ = target;
        self.engine_tracks = crate::engine::EngineSurface::running_tracks(&window.surface);
        self.engine_next = Some(rendered?.into());
        Ok(())
    }

    /// Commits the window's pending node programs through the browser
    /// page's mount and renders the engine frame (§A), which the engine
    /// presents through the page's `DomTarget` under `root`. The engine
    /// window lives in `window_slot` per device-creation chain: a new chain
    /// or a lost device replaces it, and the new mount remounts every node.
    ///
    /// # Errors
    ///
    /// [`FrameError`] when the engine cannot create the page's surface or
    /// the frame fails to render.
    #[cfg(all(target_arch = "wasm32", feature = "web"))]
    #[allow(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread"
    )]
    pub(crate) async fn render_dom_frame(
        &mut self,
        target: &FrameRenderTarget<'_>,
        root: &web_sys::HtmlElement,
        window_slot: &mut Option<
            CherenkovWindow<crate::engine::DomCherenkovSurface, crate::text::ParleyEngine>,
        >,
    ) -> Result<(), FrameError> {
        let _render_span = tracing::debug_span!("hydrolysis_render_scene").entered();
        if !window_slot
            .as_ref()
            .is_some_and(|window| window.renders_on(target))
        {
            if window_slot.is_some() {
                self.unmount_for_window_replacement();
            }
            *window_slot = None;
            let surface = crate::engine::DomCherenkovSurface::new(
                Rc::clone(&target.engine.engine),
                root.clone(),
                (target.width, target.height),
                self.host_wake(),
            )
            .await
            .map_err(FrameError::Surface)?;
            *window_slot = Some(CherenkovWindow::new(
                surface,
                target,
                Arc::clone(&self.applied_filter_metrics),
                Rc::clone(&self.material_registry),
            ));
        }
        let mut window = window_slot
            .take()
            .expect("hydrolysis renderer: the window was just ensured");
        let _frame = self.commit_window_frame(&mut window, target, 1.0);

        self.applied_filter_metrics.reset();
        let rendered = window.surface.render(self.frame_time()).await;
        self.take_applied_filter_metrics();
        self.engine_tracks = crate::engine::EngineSurface::running_tracks(&window.surface);
        *window_slot = Some(window);
        self.engine_next = Some(rendered?.into());
        Ok(())
    }

    /// Opens `window`'s frame and commits it: the surface takes the
    /// target's size, scale, `headroom` and base colour, and the mount
    /// commits the pending node programs. The returned scope is held
    /// across the frame's render.
    fn commit_window_frame<S: crate::engine::EngineSurface<Backend = cherenkov_gpu::Gpu>>(
        &mut self,
        window: &mut CherenkovWindow<S, crate::text::ParleyEngine>,
        target: &FrameRenderTarget<'_>,
        headroom: f32,
    ) -> cherenkov::FrameScope {
        let frame = window.surface.core().begin_frame();

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(
            window.gpu_profiler.as_ref(),
            target.device,
            &target.shared_device.queue,
            0,
        );

        self.commit_engine_frame(
            &mut window.surface,
            &mut window.mount,
            &window.host,
            target.geometry(),
            headroom,
        );

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(
            window.gpu_profiler.as_ref(),
            target.device,
            &target.shared_device.queue,
            1,
        );

        frame
    }
}

impl HydrolysisRenderer<crate::text::ParleyEngine> {
    /// The host's display-link wake, as an engine surface takes it: called
    /// from whichever thread the cause lands on when the surface's content
    /// asks for a frame between the renderer's own.
    fn host_wake(&self) -> impl Fn() + Send + Sync + 'static {
        let wake = self.host_redraw_handle.clone();
        move || {
            if let Some(wake) = &wake {
                wake.request_redraw();
            }
        }
    }

    /// §F's remount: the tree's and the hosts' layers were mounted on the
    /// outgoing engine window's surface — drop them before it is replaced
    /// (decision 3 applied to the whole window). Each `Layer` drop queues
    /// its `Remove` on the dead surface's shared state, and the next commit
    /// remounts every cell on the new mount from its retained program.
    fn unmount_for_window_replacement(&self) {
        if let Some(tree) = &self.render_tree {
            tree.unmount();
        }
        self.presentation_hosts.unmount();
        self.core.root_cell().unmount();
    }

    /// Commits a frame on either Cherenkov backend: `surface` takes the
    /// frame's size, scale, `headroom` and base colour, and `mount` commits
    /// the pending node programs against `host`.
    fn commit_engine_frame<S>(
        &mut self,
        surface: &mut S,
        mount: &mut mount::Mount<S::Backend, crate::text::ParleyEngine>,
        host: &<S::Backend as mount::target::LayerTarget<crate::text::ParleyEngine>>::Host,
        frame: FrameGeometry,
        headroom: f32,
    ) where
        S: crate::engine::EngineSurface,
        S::Backend: mount::target::LayerTarget<crate::text::ParleyEngine>,
    {
        let surface = surface.core_mut();
        surface.resize((frame.width, frame.height));
        surface.display(cherenkov::Display {
            scale: frame.display_scale,
            headroom,
        });
        surface.clear_color(frame.base_color);

        self.commit_mount(mount, host, frame.display_scale);
    }

    /// Records the applied-filter work the frame's render reported.
    fn take_applied_filter_metrics(&mut self) {
        (
            self.frame_applied_filter_count,
            self.frame_applied_filter_effect,
        ) = self.applied_filter_metrics.snapshot();
    }
}

#[cfg(hydrolysis_cpu_painter)]
impl HydrolysisRenderer<crate::text::ParleyEngine> {
    /// Commits the window's pending node programs through its CPU mount and
    /// renders the frame (§A) into the offscreen framebuffer on `state`'s
    /// engine; with `capture`, reads the frame back as sRGB RGBA8 rows in
    /// that alpha convention. The window — framebuffer surface plus mount —
    /// lives per engine: a frame on another runtime's engine replaces it,
    /// and the new mount remounts every node.
    ///
    /// # Errors
    ///
    /// [`FrameError`] when the engine cannot create the window's framebuffer
    /// surface, or the frame fails to render or to read back.
    pub(crate) fn render_raster_frame(
        &mut self,
        state: &Rc<crate::engine::RasterEngineState>,
        frame: FrameGeometry,
        capture: Option<cherenkov::OutputAlpha>,
    ) -> Result<Option<Vec<u8>>, FrameError> {
        let _render_span = tracing::debug_span!("hydrolysis_render_scene").entered();
        if !self
            .raster_window
            .as_ref()
            .is_some_and(|window| Rc::ptr_eq(&window.state, state))
        {
            if self.raster_window.is_some() {
                self.unmount_for_window_replacement();
            }
            self.raster_window = None;
            let surface = crate::engine::RasterCherenkovSurface::new(
                Rc::clone(&state.engine),
                (frame.width, frame.height),
                self.host_wake(),
            )
            .map_err(FrameError::Surface)?;
            self.raster_window = Some(RasterWindow::new(
                surface,
                Rc::clone(state),
                &self.material_registry,
            ));
        }
        let mut window = self
            .raster_window
            .take()
            .expect("hydrolysis renderer: the CPU window was just ensured");
        let scope = crate::engine::EngineSurface::core(&window.surface).begin_frame();
        // An SDR destination: the presentation pass tone-maps to its
        // ceiling, as the GPU offscreen target's sRGB texture does.
        let headroom = 1.0;
        self.commit_engine_frame(
            &mut window.surface,
            &mut window.mount,
            &window.host,
            frame,
            headroom,
        );
        self.applied_filter_metrics.reset();
        let rendered = window.surface.render(self.frame_time());
        drop(scope);
        self.take_applied_filter_metrics();
        self.engine_tracks = crate::engine::EngineSurface::running_tracks(&window.surface);
        let pixels = match rendered {
            Ok(next) => {
                self.engine_next = Some(next.into());
                capture
                    .map(|alpha| window.surface.readback_rgba8(headroom, alpha))
                    .transpose()
            }
            Err(error) => Err(error),
        };
        self.raster_window = Some(window);
        Ok(pixels?)
    }
}

/// The renderer's engine window on the CPU backend: the offscreen
/// framebuffer surface and the one retained [`mount::Mount`] its node layers
/// live in.
#[cfg(hydrolysis_cpu_painter)]
pub struct RasterWindow<E: crate::text::TextEngine> {
    pub surface: crate::engine::RasterCherenkovSurface,
    pub(crate) mount: mount::Mount<cherenkov_cpu::Raster, E>,
    pub(crate) host: mount::target::RasterHost,
    /// The engine this window renders on; a frame on another one replaces
    /// the window.
    state: Rc<crate::engine::RasterEngineState>,
}

#[cfg(hydrolysis_cpu_painter)]
impl<E: crate::text::TextEngine> RasterWindow<E> {
    /// Wraps `surface` with its window mount. The CPU backend has no backdrop
    /// shaders, so a theme `materials` registry that registers any is
    /// reported at the mount's attach, naming them, and its material chrome
    /// is not drawn (water-rs/waterui#4065).
    #[expect(
        clippy::zero_sized_map_values,
        reason = "the CPU target's shader map is typed `Infallible`: it registers no backdrop shaders and stays empty (water-rs/waterui#4065)"
    )]
    fn new(
        surface: crate::engine::RasterCherenkovSurface,
        state: Rc<crate::engine::RasterEngineState>,
        materials: &Rc<cherenkov_record::MaterialRegistry>,
    ) -> Self {
        let core = crate::engine::EngineSurface::core(&surface);
        let mount = mount::Mount::new(Rc::clone(&core.engine_surface().shared), materials);
        let host = mount::target::RasterHost {
            engine: Rc::clone(&state.engine),
            resources: Rc::clone(&state.resources),
            surface: core.engine_surface_weak(),
            materials: mount::MaterialTerms {
                registry: Rc::clone(materials),
                shaders: rustc_hash::FxHashMap::default(),
            },
        };
        Self {
            surface,
            mount,
            host,
            state,
        }
    }
}

/// Why a frame could not be rendered.
///
/// Every variant but [`Self::Render`] is terminal for the surface: retrying
/// the frame on the same device generation, or the same CPU engine, fails
/// the same way, so the host stops rendering it instead of asking for
/// another frame.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FrameError {
    /// The engine could not create the window's render surface: the target
    /// is unsupported, larger than the device allows, or the engine's render
    /// thread is gone.
    #[error("the Cherenkov engine could not create the window's render surface")]
    Surface(#[source] cherenkov::SurfaceError),
    /// The render target's texture format is not one the engine presents
    /// into.
    #[cfg(hydrolysis_gpu_painter)]
    #[error("the render target's format {0:?} is not one the engine presents into")]
    UnsupportedFormat(wgpu::TextureFormat),
    /// The GPU device was lost more often than the recovery budget allows,
    /// and is not rebuilt again.
    #[cfg(hydrolysis_gpu_painter)]
    #[error(transparent)]
    DeviceUnrecoverable(#[from] crate::platform::DeviceLostError),
    /// The Android compositor could not create the window's parent surface
    /// control.
    #[cfg(all(target_os = "android", not(hydrolysis_hwui)))]
    #[error("the Android compositor could not create the window's surface control")]
    Compositor(#[source] cherenkov_gpu::interop::android::CreateFailed),
    /// The Android engine surface was created on a thread without a Looper
    /// — a host contract violation.
    #[cfg(all(target_os = "android", not(hydrolysis_hwui)))]
    #[error("the Android engine surface was created off the host's Looper thread")]
    NoLooper,
    /// The engine rejected or failed the frame. The frame is skipped and the
    /// next one retries: a lost device recovers through the frame after it.
    #[error(transparent)]
    Render(#[from] cherenkov::RenderError),
}

impl FrameError {
    /// Whether the surface can render again on a later frame: only a render
    /// failure is transient.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        !matches!(self, Self::Render(_))
    }
}

/// A rendered engine frame, waiting for its present.
#[cfg(hydrolysis_gpu_painter)]
#[must_use = "an engine frame is shown only once present_engine_frame copies it out"]
pub struct EngineFrame {
    headroom: f32,
    pub(crate) content_range: cherenkov_gpu::interop::ColorRange,
}

/// The renderer's engine window on one device-creation chain: the engine
/// surface and the one retained [`mount::Mount`] its node layers live in.
#[cfg(hydrolysis_gpu_painter)]
pub struct CherenkovWindow<S, E: crate::text::TextEngine> {
    pub surface: S,
    pub(crate) mount: mount::Mount<cherenkov_gpu::Gpu, E>,
    pub(crate) host: mount::CherenkovHost,
    /// The generation's engine state, kept with the mount registered on it.
    _state: Rc<crate::engine::SharedEngineState>,
    context_id: u64,
    device_loss: crate::platform::DeviceLoss,
    #[cfg(feature = "frame-profile")]
    pub(crate) gpu_profiler: Option<GpuFrameProfiler>,
}

#[cfg(hydrolysis_gpu_painter)]
impl<S, E: crate::text::TextEngine> CherenkovWindow<S, E> {
    /// The device-creation chain this window renders on.
    #[cfg(feature = "frame-profile")]
    pub(crate) const fn context_id(&self) -> u64 {
        self.context_id
    }

    /// Whether this window still renders `target`'s frames: the same
    /// device-creation chain, with its device not reported lost.
    fn renders_on(&self, target: &FrameRenderTarget<'_>) -> bool {
        self.context_id == target.gpu_context_id && !self.device_loss.is_lost()
    }
}

#[cfg(hydrolysis_gpu_painter)]
impl<S: crate::engine::EngineSurface<Backend = cherenkov_gpu::Gpu>, E: crate::text::TextEngine>
    CherenkovWindow<S, E>
{
    /// Wraps `surface` with its window mount. `materials` is the theme's
    /// material registry: attaching this window's engine registers every
    /// shader it names, so a chrome member's group binds an engine handle
    /// (water-rs/waterui#1788). A rejected source is reported at attach,
    /// naming the key, and the chrome that names it is not drawn.
    fn new(
        surface: S,
        target: &FrameRenderTarget<'_>,
        metrics: Arc<crate::renderer::effects::AppliedFilterMetrics>,
        materials: Rc<cherenkov_record::MaterialRegistry>,
    ) -> Self {
        let state = Rc::clone(target.engine);
        let core = surface.core();
        let mount = mount::Mount::new(Rc::clone(&core.engine_surface().shared), &materials);
        let host = mount::CherenkovHost {
            engine: Rc::clone(&state.engine),
            resources: Rc::clone(&state.resources),
            metrics,
            surface: core.engine_surface_weak(),
            materials: mount::MaterialTerms::resolve(&state.engine, materials),
            shared_device: target.shared_device.clone(),
        };
        Self {
            surface,
            mount,
            host,
            _state: state,
            context_id: target.gpu_context_id,
            device_loss: target.device_loss.clone(),
            #[cfg(feature = "frame-profile")]
            gpu_profiler: GpuFrameProfiler::new(target.device),
        }
    }
}
