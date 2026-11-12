//! The retained state a `GpuContentView` leaf carries through the frame.
//!
//! `GpuContentView` owns its producer — a `Send` `GpuContent` the engine runs
//! on its render thread — plus the UI-side hooks (input, per-frame pump, ime
//! caret, accessibility label) that stay on this thread. The compositor holds
//! the view behind this node so input routing and the per-frame
//! [`waterui_graphics::gpu::GpuContentView::frame`] pump keep working after
//! the producer moves to the engine.
//!
//! The producer installs exactly once: [`GpuContentView::take_engine_content`]
//! moves it into an engine `GpuContentHandle`, registered as a
//! `GpuProducer`. The producer outlives any one layer: a keyed mount that is
//! dropped (a navigation page that is covered) and comes back binds the same
//! producer to its new layer. Transient (capture) windows never install it at
//! all: a capture cannot consume the one install the producer gets.

use waterui_graphics::gpu::{ExternalFrameView, FrameReceiver, GpuContentView};

/// The `GpuContentView` a [`crate::renderer::tree::GpuContentNode`] owns, and
/// its engine producer once installed.
///
/// `producer` is set when the node's install layer first commits through the
/// window's mount (`LayerTarget::mount_gpu_content`).
pub struct GpuContentRuntime {
    pub(crate) view: GpuContentView,
    /// The size and scale the view's input handler was last told about.
    pub(crate) resize: crate::renderer::ResizeTracker,
    /// The engine producer the content was registered as: `Some` once
    /// `take_engine_content` has run, retained so it can be bound again and
    /// retired on drop.
    #[cfg_attr(all(hydrolysis_hwui, not(hydrolysis_gpu)), expect(dead_code))]
    pub(crate) producer: Option<cherenkov::GpuProducer<cherenkov_gpu::Gpu>>,
    /// The engine layer and pixel size the producer is currently bound at.
    pub(crate) binding: Option<(cherenkov::LayerId, (u32, u32))>,
    /// The HWUI painter's realization: the view's own platform surface and
    /// the pump that renders into it.
    #[cfg(hydrolysis_hwui_gpu)]
    pub(crate) hosted: crate::runner::android::hosted_gpu::HostedLayer,
}

impl std::fmt::Debug for GpuContentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuContentRuntime").finish_non_exhaustive()
    }
}

/// What a flush reads off a GPU-backed view's runtime every pass: the
/// view's accessibility name and value, and whether it takes input. Both
/// [`GpuContentRuntime`] and [`ExternalFrameRuntime`] answer it, and
/// both route input and the caret through the renderer's
/// `SurfaceInputReceiver`.
pub trait HostedGpuView {
    /// The accessibility label, the accessibility value, and whether the
    /// view takes input events.
    fn semantics(&self) -> (Option<String>, Option<String>, bool);

    /// The platform-view table and placement the HWUI painter mounts the
    /// view's surface through.
    #[cfg(hydrolysis_hwui_gpu)]
    fn placement(
        &self,
    ) -> (
        std::rc::Rc<std::cell::RefCell<crate::platform_view::PlatformViewTable>>,
        crate::platform_view::PlatformViewPlacement,
    );
}

impl HostedGpuView for GpuContentRuntime {
    fn semantics(&self) -> (Option<String>, Option<String>, bool) {
        (
            self.view.accessibility_label().map(str::to_owned),
            self.view.accessibility_value().map(str::to_owned),
            self.view.wants_input_events(),
        )
    }

    #[cfg(hydrolysis_hwui_gpu)]
    fn placement(
        &self,
    ) -> (
        std::rc::Rc<std::cell::RefCell<crate::platform_view::PlatformViewTable>>,
        crate::platform_view::PlatformViewPlacement,
    ) {
        (
            std::rc::Rc::clone(self.hosted.table()),
            self.hosted.placement(),
        )
    }
}

impl HostedGpuView for ExternalFrameRuntime {
    fn semantics(&self) -> (Option<String>, Option<String>, bool) {
        (
            self.view.accessibility_label().map(str::to_owned),
            self.view.accessibility_value().map(str::to_owned),
            self.view.wants_input_events(),
        )
    }

    #[cfg(hydrolysis_hwui_gpu)]
    fn placement(
        &self,
    ) -> (
        std::rc::Rc<std::cell::RefCell<crate::platform_view::PlatformViewTable>>,
        crate::platform_view::PlatformViewPlacement,
    ) {
        (
            std::rc::Rc::clone(self.hosted.table()),
            self.hosted.placement(),
        )
    }
}

impl GpuContentRuntime {
    pub(crate) const fn new(
        view: GpuContentView,
        #[cfg(hydrolysis_hwui_gpu)] hosted: crate::runner::android::hosted_gpu::HostedLayer,
    ) -> Self {
        Self {
            view,
            resize: crate::renderer::ResizeTracker::new(),
            producer: None,
            binding: None,
            #[cfg(hydrolysis_hwui_gpu)]
            hosted,
        }
    }
}

/// The `ExternalFrameView` a [`crate::renderer::tree::ExternalFrameNode`]
/// owns, and the stream's frame receiver once a mount has started it.
///
/// Unlike `GpuContent`, an external-frame source is restartable: the view
/// hands out a fresh [`ExternalFrameStream`] handle every call, and a lost
/// device or a reborn mount starts the source again with the new output.
/// `receiver` is `Some` once the node's install layer has started the source
/// on the window's device (`LayerTarget::mount_external_frame`).
pub struct ExternalFrameRuntime {
    pub(crate) view: ExternalFrameView,
    /// The size and scale the view's input handler was last told about.
    pub(crate) resize: crate::renderer::ResizeTracker,
    /// The mailbox drain end, installed by the node's commit.
    pub(crate) receiver: Option<FrameReceiver>,
    /// The submitted-frame producer/sink pair the compositor created with
    /// the receiver: `sink` submits each new frame, `producer` re-issues the
    /// binding when the plane size changes.
    #[cfg_attr(hydrolysis_hwui, expect(dead_code))]
    pub(crate) producer: Option<cherenkov::GpuProducer<cherenkov_gpu::Gpu>>,
    /// The submission end of `producer`'s pair.
    #[cfg_attr(hydrolysis_hwui, expect(dead_code))]
    pub(crate) sink: Option<cherenkov::FrameSink<cherenkov_gpu::Gpu>>,
    /// The plane size of the last presented frame, for the stretch transform.
    #[cfg_attr(hydrolysis_hwui, expect(dead_code))]
    pub(crate) frame_pixels: Option<(u32, u32)>,
    /// The engine layer and plane size the producer is currently bound at.
    pub(crate) binding: Option<(cherenkov::LayerId, (u32, u32))>,
    /// The HWUI painter's realization: the view's own platform surface and
    /// the pump that renders into it.
    #[cfg(hydrolysis_hwui_gpu)]
    pub(crate) hosted: crate::runner::android::hosted_gpu::HostedLayer,
}

impl std::fmt::Debug for ExternalFrameRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalFrameRuntime")
            .finish_non_exhaustive()
    }
}

impl ExternalFrameRuntime {
    pub(crate) const fn new(
        view: ExternalFrameView,
        #[cfg(hydrolysis_hwui_gpu)] hosted: crate::runner::android::hosted_gpu::HostedLayer,
    ) -> Self {
        Self {
            view,
            resize: crate::renderer::ResizeTracker::new(),
            receiver: None,
            producer: None,
            sink: None,
            frame_pixels: None,
            binding: None,
            #[cfg(hydrolysis_hwui_gpu)]
            hosted,
        }
    }
}
