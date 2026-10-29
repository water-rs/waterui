//! The engine-presented surface of an Apple view.
//!
//! The engine presents the window itself, through a
//! `cherenkov_gpu::WindowTarget` over the view's backing layer: it acquires
//! the drawable and presents inside `Engine::render`, so this surface owns
//! no swapchain, only the GPU context, the view the target is built from and
//! the lazily created engine window.

use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;

use cocoa_ui::PlatformView;
use cocoa_ui::Retained;
use wgpu::rwh::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle,
    UiKitWindowHandle, WindowHandle,
};

use crate::platform::{
    DeviceLoss, EngineGpuContext, EnginePresented, EngineWindowHandle, EngineWindowTarget,
    PresentationSurface,
};
use crate::renderer::{CherenkovWindow, FrameRenderTarget};
use crate::runner::window::{GpuSurfaceFrame, SurfaceRenderResult};
use crate::text::ParleyEngine;

/// The view an engine window presents on, and the refresh range the display
/// it sits on offers.
#[derive(Clone)]
pub(super) struct AppleViewHandle {
    view: Retained<PlatformView>,
    refresh: cherenkov::RefreshRange,
}

#[expect(
    clippy::non_send_fields_in_send_ty,
    reason = "the retained view is only ever used and released on the main thread"
)]
// SAFETY: `WindowTarget::new` captures the view's layer on the main thread
// and keeps the handle inside a main-thread-owned scene, so the retained view
// is only used and released there. The type crosses threads solely because
// `wgpu::WindowHandle` demands it of every window handle.
unsafe impl Send for AppleViewHandle {}
// SAFETY: see the `Send` impl; no method of the handle is reached off the
// main thread.
unsafe impl Sync for AppleViewHandle {}

impl AppleViewHandle {
    /// A handle on `view`, presenting at up to `max_refresh_hz`.
    ///
    /// # Panics
    /// When `max_refresh_hz` is zero.
    pub(super) fn new(view: Retained<PlatformView>, max_refresh_hz: u32) -> Self {
        assert!(max_refresh_hz > 0, "a display refreshes at a positive rate");
        Self {
            view,
            refresh: 1..=max_refresh_hz,
        }
    }

    /// The view as an opaque pointer, valid while the handle lives.
    fn view_ptr(&self) -> NonNull<std::ffi::c_void> {
        NonNull::from(&*self.view).cast()
    }
}

impl HasWindowHandle for AppleViewHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = UiKitWindowHandle::new(self.view_ptr());
        // SAFETY: the pointer names the view this handle retains, which
        // outlives the borrow.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::UiKit(handle)) })
    }
}

impl HasDisplayHandle for AppleViewHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::uikit())
    }
}

impl EngineWindowHandle for AppleViewHandle {
    fn window_target(&self, size: (u32, u32)) -> cherenkov_gpu::WindowTarget {
        cherenkov_gpu::WindowTarget::new(self.clone(), size).rate(self.refresh.clone())
    }
}

/// One view's presentation state: the process GPU context, the device
/// generation this view renders on, and the typed engine window created on
/// the first frame.
pub(super) struct AppleViewSurface {
    gpu: EngineGpuContext,
    /// The device generation this surface renders on — a clone of the
    /// context's live generation, adopted in `ensure_live`
    /// (water-rs/waterui#2742).
    handles: DeviceLoss,
    view: AppleViewHandle,
    /// The pixel size reported to the pump; the engine surface resizes to it.
    size: (u32, u32),
    /// Whether the view composites with what lies behind it: the engine
    /// target is created transparent or opaque to match, and recreated when
    /// the background switches.
    transparent: bool,
    /// Set when the view crossed displays or changed scale; the next frame
    /// announces it through `cherenkov::Surface::display_moved` exactly once.
    display_moved: Rc<Cell<bool>>,
    engine_window: Option<CherenkovWindow<crate::engine::WindowCherenkovSurface, ParleyEngine>>,
}

impl core::fmt::Debug for AppleViewSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AppleViewSurface")
            .field("size", &self.size)
            .field("transparent", &self.transparent)
            .finish_non_exhaustive()
    }
}

impl AppleViewSurface {
    /// A surface presenting `view` at `size` physical pixels on `gpu`'s live
    /// device generation. `display_moved` is the flag the host raises when
    /// the view changes display or scale.
    pub(super) fn new(
        gpu: EngineGpuContext,
        view: AppleViewHandle,
        size: (u32, u32),
        display_moved: Rc<Cell<bool>>,
    ) -> Self {
        Self {
            handles: gpu.gpu.generation(),
            gpu,
            view,
            size: (size.0.max(1), size.1.max(1)),
            transparent: false,
            display_moved,
            engine_window: None,
        }
    }

    /// Records the window's new transparency; the next frame recreates the
    /// engine target with it (the target's composite alpha is fixed when it
    /// is created).
    pub(super) const fn set_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
    }
}

impl PresentationSurface for AppleViewSurface {
    fn adapter(&self) -> &wgpu::Adapter {
        &self.handles.gpu_context().shared_device.adapter
    }

    fn device(&self) -> &wgpu::Device {
        &self.handles.gpu_context().shared_device.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.handles.gpu_context().shared_device.queue
    }

    fn device_loss(&self) -> &DeviceLoss {
        &self.handles
    }

    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
    }

    /// Adopts the context's live device generation — rebuilding it when the
    /// adopted one was lost. The engine's `WindowTarget` is rebuilt on the
    /// new context id, so nothing here reconfigures.
    fn ensure_live(&mut self) {
        let gpu = self.gpu.clone();
        let generation = gpu
            .gpu
            .ensure_live(|instance| pollster::block_on(gpu.recreate(instance, None)));
        if generation.gpu_context().context_id != self.handles.gpu_context().context_id {
            self.handles = generation;
        }
    }
}

impl EnginePresented for AppleViewSurface {
    fn render_targets(
        &mut self,
        display_scale: f64,
        base_color: waterui_graphics::draw::WorkingColor,
    ) -> (FrameRenderTarget<'_>, EngineWindowTarget<'_>) {
        let context = self.handles.gpu_context();
        (
            FrameRenderTarget {
                adapter: &context.shared_device.adapter,
                device: &context.shared_device.device,
                queue: &context.shared_device.queue,
                device_loss: self.handles.clone(),
                gpu_context_id: context.context_id,
                shared_device: context.shared_device.clone(),
                display_scale,
                width: self.size.0,
                height: self.size.1,
                base_color,
            },
            EngineWindowTarget {
                window: &self.view,
                window_slot: &mut self.engine_window,
                transparent: self.transparent,
                display_moved: self.display_moved.replace(false),
            },
        )
    }
}

impl GpuSurfaceFrame for AppleViewSurface {
    fn render_frame(
        &mut self,
        renderer: &mut crate::renderer::HydrolysisRenderer<ParleyEngine>,
        clear_color: peniko::Color,
        display_scale: f64,
    ) -> Result<SurfaceRenderResult, crate::platform::SurfaceError> {
        Ok(crate::runner::window::render_engine_presented_frame(
            self,
            renderer,
            clear_color,
            display_scale,
        ))
    }
}
