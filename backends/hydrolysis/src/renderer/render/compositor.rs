//! The renderer's engine frame: its one window mount (§A) committed and
//! rendered into the target.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use std::rc::Rc;
use std::sync::Arc;

/// presentation texture (or `None` for a readback-only render) and the
/// colour under the scene's content.
///
/// The render commits through the renderer's window mount and targets an
/// SDR, single-scale display, the headless contract `waterui-testing`
/// drives.
#[derive(Debug)]
pub struct HydrolysisRenderTarget<'a> {
    /// The adapter the frame's device was requested on; the shared engine is
    /// created against what it can actually run.
    pub adapter: &'a wgpu::Adapter,
    /// The device the surface presents through.
    pub device: &'a wgpu::Device,
    /// The submission queue the surface presents through.
    pub queue: &'a wgpu::Queue,
    /// Reports this device lost; taken when the device was opened. Carries
    /// the device-creation chain the engine pool keys on.
    pub device_loss: crate::platform::DeviceLoss,
    /// The presentation attachment the frame is copied into.
    pub texture: &'a wgpu::Texture,
    /// The attachment's format: `Rgba8`/`Bgra8` unorm or an `Rgba` float
    /// format.
    pub format: wgpu::TextureFormat,
    /// Attachment size in device pixels.
    pub width: u32,
    /// The render target's height in pixels.
    pub height: u32,
    /// The colour under the scene's content.
    pub base_color: waterui_graphics::draw::WorkingColor,
}

/// The frame description both render paths work on — the texture frame a
/// host presents and the macOS window frame the engine presents: the
/// device-creation chain the engine pool keys on, the display's scale, and
/// the target's size and base colour.
pub struct FrameRenderTarget<'a> {
    pub adapter: &'a wgpu::Adapter,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub device_loss: crate::platform::DeviceLoss,
    /// The device-creation chain the frame's handles belong to — the engine
    /// pool key.
    pub gpu_context_id: u64,
    /// All four handles of that chain; the shared engine requires them to
    /// come from the same creation chain.
    pub shared_device: cherenkov_gpu::interop::SharedDevice,
    /// Device pixels per logical unit on the display the target shows on.
    pub display_scale: f64,
    pub width: u32,
    pub height: u32,
    pub base_color: waterui_graphics::draw::WorkingColor,
}

impl<'a> HydrolysisRenderTarget<'a> {
    /// The transient, SDR, single-scale [`FrameRenderTarget`] a public
    /// [`HydrolysisRenderTarget`] describes, with the device-creation chain
    /// taken from `device_loss`'s context.
    fn into_frame_target(self) -> FrameRenderTarget<'a> {
        let context = self.device_loss.gpu_context();
        FrameRenderTarget {
            adapter: self.adapter,
            device: self.device,
            queue: self.queue,
            device_loss: self.device_loss,
            gpu_context_id: context.context_id,
            shared_device: context.shared_device,
            display_scale: 1.0,
            width: self.width,
            height: self.height,
            base_color: self.base_color,
        }
    }
}

impl HydrolysisRenderer {
    crate::engine::cfg_async_fn! {
        /// Renders the frame into `target`'s texture.
        ///
        /// Async on wasm32, where the surface render inside awaits the browser
        /// device.
        ///
        /// # Errors
        ///
        /// Returns the engine's [`cherenkov::RenderError`] when the frame
        /// fails to render.
        pub fn render_scene_to_texture(
            &mut self,
            target: HydrolysisRenderTarget<'_>,
        ) -> Result<(), cherenkov::RenderError> {
            let (texture, format) = (target.texture, target.format);
            let (device, queue) = (target.device, target.queue);
            let frame = crate::engine::engine_await!(
                self.render_texture_frame(target.into_frame_target(), format, 1.0)
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
        EngineFrame { headroom }: EngineFrame,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        color: cherenkov_gpu::interop::OutputColor,
        alpha: cherenkov_gpu::interop::OutputAlpha,
    ) {
        // The window map leaves `self` for the call, as in the render, so the
        // profiler mark may borrow the renderer.
        let mut window = self.cherenkov_window.take().expect(
            "hydrolysis renderer: the engine frame's window left the renderer before its present",
        );
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
        /// Returns the engine's [`cherenkov::RenderError`] when the frame
        /// fails to render.
        pub(crate) fn render_texture_frame(
            &mut self,
            target: FrameRenderTarget<'_>,
            format: wgpu::TextureFormat,
            headroom: f32,
        ) -> Result<EngineFrame, cherenkov::RenderError> {
        assert!(
            matches!(
                format.remove_srgb_suffix(),
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
            ) || matches!(
                format,
                wgpu::TextureFormat::Rgba16Float
                    | wgpu::TextureFormat::Rgba32Float
                    | wgpu::TextureFormat::Rgb10a2Unorm
            ),
            "hydrolysis renderer: unsupported surface format {format:?}"
        );

        let _render_span = tracing::debug_span!("hydrolysis_render_scene").entered();
        let state = crate::engine::engine_await!(crate::engine::shared_engine_state(
            target.gpu_context_id,
            target.adapter,
            target.shared_device.clone(),
        ));
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
                Rc::clone(&state.engine),
                target.device,
                target.adapter.get_info().backend,
                (target.width, target.height),
                self.host_wake(),
            ));
            self.cherenkov_window = Some(CherenkovWindow::new(
                surface,
                &target,
                state,
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
        let rendered = crate::engine::engine_await!(window.surface.render());
        self.take_applied_filter_metrics();
        self.cherenkov_window = Some(window);
        self.engine_next = Some(rendered?);
        Ok(EngineFrame { headroom })
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
    /// Returns the engine's [`cherenkov::RenderError`] when the frame fails
    /// to render.
    #[cfg(all(target_os = "macos", hydrolysis_winit))]
    pub(crate) fn render_window_frame(
        &mut self,
        target: &FrameRenderTarget<'_>,
        engine_target: crate::platform::EngineWindowTarget<'_>,
    ) -> Result<(), cherenkov::RenderError> {
        let crate::platform::EngineWindowTarget {
            window: native_window,
            window_slot,
            transparent,
            display_moved,
        } = engine_target;
        let _render_span = tracing::debug_span!("hydrolysis_render_scene").entered();
        let state = crate::engine::shared_engine_state(
            target.gpu_context_id,
            target.adapter,
            target.shared_device.clone(),
        );
        if !window_slot.as_ref().is_some_and(|window| {
            window.renders_on(target) && window.surface.transparent() == transparent
        }) {
            if window_slot.is_some() {
                self.unmount_for_window_replacement();
            }
            *window_slot = None;
            let surface = crate::engine::WindowCherenkovSurface::new(
                Rc::clone(&state.engine),
                Arc::clone(native_window),
                (target.width, target.height),
                crate::platform::hydrolysis_output_request(transparent),
                self.host_wake(),
            );
            *window_slot = Some(CherenkovWindow::new(
                surface,
                target,
                state,
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

        self.applied_filter_metrics.reset();
        let rendered = window.surface.render(target.display_scale);
        self.take_applied_filter_metrics();
        // The engine presented inside the render: the frame's last marker
        // and its resolve follow it here, where a host-presented frame
        // marks after its copy and resolves after its present.
        #[cfg(feature = "frame-profile")]
        {
            self.gpu_profile_mark(window.gpu_profiler.as_ref(), target.device, target.queue, 2);
            self.finish_gpu_frame_profile_with(
                window.gpu_profiler.as_ref(),
                target.device,
                target.queue,
            );
        }
        *window_slot = Some(window);
        self.engine_next = Some(rendered?);
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
    /// Returns the engine's [`cherenkov::RenderError`] when the frame fails
    /// to render.
    #[cfg(all(target_arch = "wasm32", feature = "web"))]
    #[allow(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread"
    )]
    pub(crate) async fn render_dom_frame(
        &mut self,
        target: &FrameRenderTarget<'_>,
        root: &web_sys::HtmlElement,
        window_slot: &mut Option<CherenkovWindow<crate::engine::DomCherenkovSurface>>,
    ) -> Result<(), cherenkov::RenderError> {
        let _render_span = tracing::debug_span!("hydrolysis_render_scene").entered();
        let state = crate::engine::shared_engine_state(
            target.gpu_context_id,
            target.adapter,
            target.shared_device.clone(),
        )
        .await;
        if !window_slot
            .as_ref()
            .is_some_and(|window| window.renders_on(target))
        {
            if window_slot.is_some() {
                self.unmount_for_window_replacement();
            }
            *window_slot = None;
            let surface = crate::engine::DomCherenkovSurface::new(
                Rc::clone(&state.engine),
                root.clone(),
                (target.width, target.height),
                self.host_wake(),
            )
            .await;
            *window_slot = Some(CherenkovWindow::new(
                surface,
                target,
                state,
                Arc::clone(&self.applied_filter_metrics),
                Rc::clone(&self.material_registry),
            ));
        }
        let mut window = window_slot
            .take()
            .expect("hydrolysis renderer: the window was just ensured");
        let _frame = self.commit_window_frame(&mut window, target, 1.0);

        self.applied_filter_metrics.reset();
        let rendered = window.surface.render().await;
        self.take_applied_filter_metrics();
        *window_slot = Some(window);
        self.engine_next = Some(rendered?);
        Ok(())
    }

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

    /// Opens `window`'s frame and commits it: the surface takes the
    /// target's size, scale, `headroom` and base colour, and the mount
    /// commits the pending node programs. The returned scope is held
    /// across the frame's render.
    fn commit_window_frame<S: crate::engine::EngineSurface>(
        &mut self,
        window: &mut CherenkovWindow<S>,
        target: &FrameRenderTarget<'_>,
        headroom: f32,
    ) -> cherenkov::FrameScope {
        let frame = window.surface.core().begin_frame();

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(window.gpu_profiler.as_ref(), target.device, target.queue, 0);

        let surface = window.surface.core_mut();
        surface.resize((target.width, target.height));
        surface.display(cherenkov::Display {
            scale: target.display_scale,
            headroom,
        });
        surface.clear_color(target.base_color);

        let roots = self.mount_roots();
        let host_wake = self.host_redraw_handle.clone();
        let window_material = self.core.window_material;
        let core = &mut self.core;
        let mut wakes = |cell: &Rc<NodeCell>| core.producer_wake(cell, host_wake.clone());
        window.mount.commit(
            &window.host,
            self.window_display_transform,
            target.display_scale,
            &roots,
            &mut wakes,
            window_material.as_ref(),
        );
        self.core.clear_commit_marks();
        let taken = window.mount.take_stats();
        self.last_mount_stats = mount::layers::census(&roots, taken);
        self.state.counters.recorded_view_contents += taken.installs;
        self.state.counters.layer_creations += taken.created;
        self.state.counters.layer_removals += taken.removed;
        let (fonts, images) = window.state.resources.take_registration_stats();
        self.state.counters.font_registrations += fonts;
        self.state.counters.image_registrations += images;

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(window.gpu_profiler.as_ref(), target.device, target.queue, 1);

        frame
    }

    /// Records the applied-filter work the frame's render reported.
    fn take_applied_filter_metrics(&mut self) {
        (
            self.frame_applied_filter_count,
            self.frame_applied_filter_effect,
        ) = self.applied_filter_metrics.snapshot();
    }
}

/// A rendered engine frame, waiting for its present.
#[must_use = "an engine frame is shown only once present_engine_frame copies it out"]
pub struct EngineFrame {
    headroom: f32,
}

/// The renderer's engine window on one device-creation chain: the engine
/// surface and the one retained [`mount::Mount`] its node layers live in.
pub struct CherenkovWindow<S> {
    pub surface: S,
    pub(crate) mount: mount::Mount<cherenkov_gpu::Gpu>,
    pub(crate) host: mount::CherenkovHost,
    /// The shared engine state the surface renders through.
    pub(crate) state: Rc<crate::engine::SharedEngineState>,
    context_id: u64,
    device_loss: crate::platform::DeviceLoss,
    #[cfg(feature = "frame-profile")]
    pub(crate) gpu_profiler: Option<GpuFrameProfiler>,
}

impl<S> CherenkovWindow<S> {
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

impl<S: crate::engine::EngineSurface> CherenkovWindow<S> {
    /// Wraps `surface` with its window mount. `materials` is the theme's
    /// material registry: attaching this window's engine registers every
    /// shader it names, so a chrome member's group binds an engine handle
    /// (water-rs/waterui#1788). A rejected source panics at attach,
    /// naming the key.
    fn new(
        surface: S,
        target: &FrameRenderTarget<'_>,
        state: Rc<crate::engine::SharedEngineState>,
        metrics: Arc<crate::renderer::effects::AppliedFilterMetrics>,
        materials: Rc<cherenkov_record::MaterialRegistry>,
    ) -> Self {
        let core = surface.core();
        let mount = mount::Mount::new(Rc::clone(&core.engine_surface().shared), &materials);
        let host = mount::CherenkovHost {
            engine: Rc::clone(&state.engine),
            resources: Rc::clone(&state.resources),
            metrics,
            surface: core.engine_surface_weak(),
            materials: mount::MaterialTerms::resolve(&state.engine, materials),
            device: target.device.clone(),
            queue: target.queue.clone(),
        };
        Self {
            surface,
            mount,
            host,
            state,
            context_id: target.gpu_context_id,
            device_loss: target.device_loss.clone(),
            #[cfg(feature = "frame-profile")]
            gpu_profiler: GpuFrameProfiler::new(target.device),
        }
    }
}
