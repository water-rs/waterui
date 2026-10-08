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

/// Common render inputs shared by texture- and window-presented frames.
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
        /// Renders into and presents a host-provided texture.
        ///
        /// # Errors
        /// Returns the Cherenkov render error when the engine cannot render
        /// the scene.
        pub fn render_scene_to_texture(
            &mut self,
            target: HydrolysisRenderTarget<'_>,
        ) -> Result<(), cherenkov::RenderError> {
            let texture = target.texture;
            let device = target.device;
            let queue = target.queue;
            let format = target.format;
            let target = target.into_frame_target();
            let frame = crate::engine::engine_await!(
                self.render_texture_frame(target, format, 1.0)
            )?;
            self.present_engine_frame(
                frame,
                device,
                queue,
                texture,
                crate::engine::format_output_color(format),
                cherenkov_gpu::interop::OutputAlpha::Straight,
            );
            Ok(())
        }
    }

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
                    wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
                ),
                "hydrolysis renderer: unsupported surface format {format:?}",
            );
            let _span = tracing::debug_span!("hydrolysis_render_scene").entered();
            let state = crate::engine::engine_await!(crate::engine::shared_engine_state(
                target.gpu_context_id,
                target.adapter,
                target.shared_device.clone(),
            ));
            let replace = self.cherenkov_window.as_ref().is_none_or(|window| {
                window.context_id != target.gpu_context_id || window.device_loss.is_lost()
            });
            if replace {
                if self.cherenkov_window.is_some() {
                    self.unmount_current_tree();
                }
                let wake = self.host_redraw_handle.clone();
                let surface = crate::engine::engine_await!(
                    crate::engine::TextureCherenkovSurface::new(
                        Rc::clone(&state.engine),
                        target.device,
                        target.adapter.get_info().backend,
                        (target.width, target.height),
                        move || {
                            if let Some(wake) = &wake {
                                wake.request_redraw();
                            }
                        },
                    )
                );
                self.cherenkov_window = Some(CherenkovWindow::from_surface(
                    surface,
                    &target,
                    state,
                    Arc::clone(&self.applied_filter_metrics),
                ));
            }
            let mut window = self.cherenkov_window.take().expect("texture window ensured");
            let frame_scope = self.prepare_frame(&mut window, &target, headroom);
            self.applied_filter_metrics.reset();
            let rendered = crate::engine::engine_await!(window.surface.render());
            drop(frame_scope);
            (
                self.frame_applied_filter_count,
                self.frame_applied_filter_effect,
            ) = self.applied_filter_metrics.snapshot();
            self.cherenkov_window = Some(window);
            self.engine_next = Some(rendered?);
            Ok(EngineFrame { headroom })
        }
    }

    #[cfg(all(target_os = "macos", hydrolysis_winit))]
    pub(crate) fn render_window_frame(
        &mut self,
        target: &FrameRenderTarget<'_>,
        engine_target: crate::platform::EngineWindowTarget<'_>,
    ) -> Result<(), cherenkov::RenderError> {
        let state = crate::engine::shared_engine_state(
            target.gpu_context_id,
            target.adapter,
            target.shared_device.clone(),
        );
        let crate::platform::EngineWindowTarget {
            window,
            window_slot,
            display_moved,
            redraw,
        } = engine_target;
        let replace = window_slot_needs_replacement(window_slot.as_ref(), target.gpu_context_id);
        if replace {
            if window_slot.is_some() {
                self.unmount_current_tree();
            }
            let wake = redraw;
            let target_window =
                cherenkov_gpu::WindowTarget::new(window, (target.width, target.height))
                    .transparent(true);
            let surface = crate::engine::WindowCherenkovSurface::new(
                Rc::clone(&state.engine),
                target_window,
                move || {
                    if let Some(wake) = &wake {
                        wake.request_redraw();
                    }
                },
            );
            *window_slot = Some(CherenkovWindow::from_surface(
                surface,
                target,
                state,
                Arc::clone(&self.applied_filter_metrics),
            ));
        }
        let mut window = window_slot.take().expect("macOS engine window ensured");
        if display_moved {
            window.surface.display_moved();
        }
        let frame_scope = self.prepare_frame(&mut window, target, 1.0);
        self.applied_filter_metrics.reset();
        let rendered = window.surface.render(target.display_scale);
        drop(frame_scope);
        #[cfg(feature = "frame-profile")]
        if rendered.is_ok() {
            self.finish_gpu_frame_profile_with(
                window.gpu_profiler.as_ref(),
                target.device,
                target.queue,
            );
        }
        (
            self.frame_applied_filter_count,
            self.frame_applied_filter_effect,
        ) = self.applied_filter_metrics.snapshot();
        *window_slot = Some(window);
        self.engine_next = Some(rendered?);
        Ok(())
    }

    fn unmount_current_tree(&self) {
        if let Some(tree) = &self.render_tree {
            tree.unmount();
        }
        self.presentation_hosts.unmount();
        self.core.root_cell().unmount();
    }

    fn prepare_frame<S: crate::engine::cherenkov::SurfaceCoreAccess>(
        &mut self,
        window: &mut CherenkovWindow<S>,
        target: &FrameRenderTarget<'_>,
        headroom: f32,
    ) -> cherenkov::FrameScope {
        let frame_scope = window.surface.begin_frame();
        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(window.gpu_profiler.as_ref(), target.device, target.queue, 0);
        window.surface.resize((target.width, target.height));
        window.surface.display(target.display_scale, headroom);
        window.surface.clear_color(target.base_color);
        self.commit_mount(window, target.display_scale);
        #[cfg(feature = "frame-profile")]
        self.gpu_profile_mark(window.gpu_profiler.as_ref(), target.device, target.queue, 1);
        frame_scope
    }

    fn commit_mount<S: crate::engine::cherenkov::SurfaceCoreAccess>(
        &mut self,
        window: &mut CherenkovWindow<S>,
        display_scale: f64,
    ) {
        let roots = self.mount_roots();
        let host_wake = self.host_redraw_handle.clone();
        let window_material = self.core.window_material;
        let core = &mut self.core;
        let mut wakes = |cell: &Rc<NodeCell>| core.producer_wake(cell, host_wake.clone());
        window.mount.commit(
            &window.host,
            self.window_display_transform,
            display_scale,
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

impl<S: crate::engine::cherenkov::SurfaceCoreAccess> CherenkovWindow<S> {
    fn from_surface(
        surface: S,
        target: &FrameRenderTarget<'_>,
        state: Rc<crate::engine::SharedEngineState>,
        metrics: Arc<crate::renderer::effects::AppliedFilterMetrics>,
    ) -> Self {
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
            #[cfg(feature = "frame-profile")]
            gpu_profiler: GpuFrameProfiler::new(target.device),
        }
    }
}

#[cfg(all(target_os = "macos", hydrolysis_winit))]
trait WindowSlotLifetime {
    fn context_id(&self) -> u64;
    fn device_is_lost(&self) -> bool;
}

#[cfg(all(target_os = "macos", hydrolysis_winit))]
impl<S> WindowSlotLifetime for CherenkovWindow<S> {
    fn context_id(&self) -> u64 {
        self.context_id
    }

    fn device_is_lost(&self) -> bool {
        self.device_loss.is_lost()
    }
}

#[cfg(all(target_os = "macos", hydrolysis_winit))]
fn window_slot_needs_replacement<T: WindowSlotLifetime>(slot: Option<&T>, context_id: u64) -> bool {
    slot.is_none_or(|window| window.context_id() != context_id || window.device_is_lost())
}

#[cfg(all(test, target_os = "macos", hydrolysis_winit))]
mod tests {
    use std::rc::Rc;

    use super::{WindowSlotLifetime, window_slot_needs_replacement};

    struct WindowState {
        context_id: u64,
        device_lost: bool,
        size: (u32, u32),
        transparent: bool,
        mount: Rc<()>,
    }

    impl WindowSlotLifetime for WindowState {
        fn context_id(&self) -> u64 {
            self.context_id
        }

        fn device_is_lost(&self) -> bool {
            self.device_lost
        }
    }

    #[test]
    fn resize_and_opacity_changes_keep_engine_window_mount() {
        let mut slot = Some(WindowState {
            context_id: 17,
            device_lost: false,
            size: (800, 600),
            transparent: true,
            mount: Rc::new(()),
        });
        let mount = Rc::clone(&slot.as_ref().expect("slot initialized").mount);

        for (size, transparent) in [((900, 600), false), ((900, 700), true)] {
            let window = slot.as_mut().expect("slot initialized");
            window.size = size;
            window.transparent = transparent;
            assert!(!window_slot_needs_replacement(slot.as_ref(), 17));
            assert!(Rc::ptr_eq(
                &mount,
                &slot.as_ref().expect("slot retained").mount
            ));
        }
    }
}
