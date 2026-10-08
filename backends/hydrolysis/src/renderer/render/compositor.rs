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

/// The full frame description [`HydrolysisRenderer::render_scene_to_texture`]
/// works on: the public target plus the rendering parameters only an internal
/// host sets — the display's scale and HDR headroom, whether the window's
/// mounts and engine surface persist past the call, and the device-creation
/// chain the engine pool keys on.
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
    /// HDR headroom the display reaches; 1.0 for SDR.
    pub headroom: f32,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    pub base_color: waterui_graphics::draw::WorkingColor,
    /// The window the engine presents through — `Some` only for an
    /// engine-presented window (the macOS winit window, #2223): the surface
    /// is built as a `cherenkov_gpu::WindowTarget` from these inputs.
    #[cfg(all(target_os = "macos", hydrolysis_winit))]
    pub engine_window: Option<crate::platform::EngineWindowTarget>,
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
            headroom: 1.0,
            format: self.format,
            width: self.width,
            height: self.height,
            base_color: self.base_color,
            #[cfg(all(target_os = "macos", hydrolysis_winit))]
            engine_window: None,
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
            let texture = target.texture;
            crate::engine::engine_await!(
                self.render_scene_to_texture_with_alpha(
                    target.into_frame_target(),
                    texture,
                    cherenkov_gpu::interop::OutputAlpha::Straight,
                )
            )
        }
    }

    crate::engine::cfg_async_fn! {
        /// [`Self::render_scene_to_texture`] with the output's alpha
        /// convention made explicit: `alpha` is the `OutputAlpha` the
        /// presenter writes into `texture` — `surface_output_alpha`'s verdict
        /// for an OS surface's `CompositeAlphaMode`, `OutputAlpha::Straight`
        /// for an offscreen/readback target.
        ///
        /// Async on wasm32, where the engine calls inside await the browser
        /// device.
        pub(crate) fn render_scene_to_texture_with_alpha(
            &mut self,
            target: FrameRenderTarget<'_>,
            texture: &wgpu::Texture,
            alpha: cherenkov_gpu::interop::OutputAlpha,
        ) -> Result<(), cherenkov::RenderError> {
            let (device, queue) = (target.device, target.queue);
            let frame = crate::engine::engine_await!(
                self.render_engine_frame(target)
            )?;
            self.present_engine_frame(
                frame,
                device,
                queue,
                texture,
                crate::engine::format_output_color(texture.format()),
                alpha,
            );
            Ok(())
        }
    }

    /// Copies the frame [`Self::render_engine_frame`] rendered into
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
        /// renders the engine frame (§A). The window — engine surface plus
        /// mount — lives per device-creation chain: a new chain or a lost
        /// device replaces it, and the new mount remounts every node.
        ///
        /// Async on wasm32, where the engine calls inside await the browser
        /// device.
        ///
        /// # Errors
        ///
        /// Returns the engine's [`cherenkov::RenderError`] when the frame
        /// fails to render.
        pub(crate) fn render_engine_frame(
            &mut self,
            target: FrameRenderTarget<'_>,
        ) -> Result<EngineFrame, cherenkov::RenderError> {
        assert!(
            matches!(
                target.format.remove_srgb_suffix(),
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
            ) || matches!(
                target.format,
                wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
            ),
            "hydrolysis renderer: unsupported surface format {:?}",
            target.format
        );

        let _render_span = tracing::debug_span!("hydrolysis_render_scene").entered();
        let host_wake = self.host_redraw_handle.clone();
        let state = crate::engine::engine_await!(crate::engine::shared_engine_state(
            target.gpu_context_id,
            target.adapter,
            target.shared_device.clone(),
        ));
        let context_id = target.gpu_context_id;
        let replace = self.cherenkov_window.as_ref().is_none_or(|window| {
            let outdated = window.context_id != context_id || window.device_loss.is_lost();
            #[cfg(all(target_os = "macos", hydrolysis_winit))]
            // A transparency change is a different `WindowTarget`: the
            // engine surface is rebuilt with the new composite alpha.
            let outdated = outdated
                || window.engine_window_transparent
                    != target.engine_window.as_ref().map(|w| w.transparent);
            outdated
        });
        if replace {
            if self.cherenkov_window.is_some() {
                // §F's remount: the tree's and the hosts' layers were
                // mounted on the outgoing engine window's surface — drop
                // them before it is replaced (decision 3 applied to the
                // whole window). Each `Layer` drop queues its `Remove` on
                // the dead surface's shared state, and the commit below
                // remounts every cell on the new mount from its retained
                // program.
                if let Some(tree) = &self.render_tree {
                    tree.unmount();
                }
                self.presentation_hosts.unmount();
                self.core.root_cell().unmount();
            }
            self.cherenkov_window = None;
            let window = crate::engine::engine_await!(CherenkovWindow::new(
                Rc::clone(&state),
                &target,
                Arc::clone(&self.applied_filter_metrics),
                host_wake.clone(),
            ));
            self.cherenkov_window = Some(window);
        }
        let mut window = self
            .cherenkov_window
            .take()
            .expect("hydrolysis renderer: the window was just ensured");
        let _frame = window.surface.begin_frame();

        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(window.gpu_profiler.as_ref(), target.device, target.queue, 0);

        window.surface.resize((target.width, target.height));
        #[cfg(all(target_os = "macos", hydrolysis_winit))]
        let headroom = if target.engine_window.is_some() {
            // The engine's output probe reports the display's headroom;
            // sampled on main and fed back through `Surface::display`.
            window.surface.probed_headroom()
        } else {
            target.headroom
        };
        #[cfg(not(all(target_os = "macos", hydrolysis_winit)))]
        let headroom = target.headroom;
        window.surface.display(target.display_scale, headroom);
        window.surface.clear_color(target.base_color);

        let roots = self.mount_roots();
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

        self.applied_filter_metrics.reset();
        let rendered = crate::engine::engine_await!(window.surface.render());
        (
            self.frame_applied_filter_count,
            self.frame_applied_filter_effect,
        ) = self.applied_filter_metrics.snapshot();
        self.cherenkov_window = Some(window);
        self.engine_next = Some(rendered?);
        Ok(EngineFrame {
            headroom: target.headroom,
        })
    }
    }
}

/// A rendered engine frame, waiting for its present.
#[must_use = "an engine frame is shown only once present_engine_frame copies it out"]
pub struct EngineFrame {
    headroom: f32,
}

/// The renderer's engine window on one device-creation chain: the engine
/// surface and the one retained [`mount::Mount`] its node layers live in.
pub struct CherenkovWindow {
    pub surface: crate::engine::CherenkovSurface,
    pub(crate) mount: mount::Mount<cherenkov_gpu::Gpu>,
    pub(crate) host: mount::CherenkovHost,
    /// The shared engine state the surface renders through.
    pub(crate) state: Rc<crate::engine::SharedEngineState>,
    context_id: u64,
    device_loss: crate::platform::DeviceLoss,
    /// The `WindowTarget::transparent` input the surface was built with on
    /// an engine-presented window; a change rebuilds the surface.
    #[cfg(all(target_os = "macos", hydrolysis_winit))]
    engine_window_transparent: Option<bool>,
    #[cfg(feature = "frame-profile")]
    pub(crate) gpu_profiler: Option<GpuFrameProfiler>,
}

impl CherenkovWindow {
    /// The device-creation chain this window renders on.
    #[cfg(feature = "frame-profile")]
    pub(crate) const fn context_id(&self) -> u64 {
        self.context_id
    }
}

crate::engine::cfg_async_fn! {
    impl CherenkovWindow {
        /// Creates the engine surface for `target` and its window mount.
        pub(crate) fn new(
            state: Rc<crate::engine::SharedEngineState>,
            target: &FrameRenderTarget<'_>,
            metrics: Arc<crate::renderer::effects::AppliedFilterMetrics>,
            wake: Option<RedrawHandle>,
        ) -> Self {
            #[cfg(all(target_os = "macos", hydrolysis_winit))]
            let surface = if let Some(window) = &target.engine_window {
                let target_window = cherenkov_gpu::WindowTarget::new(
                    std::sync::Arc::clone(&window.window),
                    (target.width, target.height),
                )
                .transparent(window.transparent);
                crate::engine::CherenkovSurface::new_window(
                    Rc::clone(&state.engine),
                    target.device,
                    target.adapter.get_info().backend,
                    target_window,
                    move || {
                        if let Some(wake) = &wake {
                            wake.request_redraw();
                        }
                    },
                )
            } else {
                crate::engine::engine_await!(crate::engine::CherenkovSurface::new(
                    Rc::clone(&state.engine),
                    target.device,
                    target.adapter.get_info().backend,
                    (target.width, target.height),
                    move || {
                        if let Some(wake) = &wake {
                            wake.request_redraw();
                        }
                    },
                ))
            };
            #[cfg(not(all(target_os = "macos", hydrolysis_winit)))]
            let surface = crate::engine::engine_await!(crate::engine::CherenkovSurface::new(
                Rc::clone(&state.engine),
                target.device,
                target.adapter.get_info().backend,
                (target.width, target.height),
                move || {
                    if let Some(wake) = &wake {
                        wake.request_redraw();
                    }
                },
            ));
            let mount = mount::Mount::new(Rc::clone(&surface.engine_surface().shared));
            let host = mount::CherenkovHost {
                engine: Rc::clone(&state.engine),
                resources: Rc::clone(&state.resources),
                metrics,
                surface: surface.engine_surface_weak(),
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
                #[cfg(all(target_os = "macos", hydrolysis_winit))]
                engine_window_transparent: target.engine_window.as_ref().map(|w| w.transparent),
                #[cfg(feature = "frame-profile")]
                gpu_profiler: GpuFrameProfiler::new(target.device),
            }
        }
    }
}
