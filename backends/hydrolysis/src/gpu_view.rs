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
/// `producer` is set when the first `GpuContentLayer` carrying this runtime
/// reaches a persistent window's install pass — never on a transient target,
/// which would spend the view's single install on a surface that dies with
/// the call.
pub struct GpuContentRuntime {
    pub(crate) view: GpuContentView,
    /// The engine producer the content was registered as: `Some` once
    /// `take_engine_content` has run, retained so it can be bound again and
    /// retired on drop.
    pub(crate) producer: Option<cherenkov::GpuProducer<cherenkov_gpu::Gpu>>,
    /// The engine layer and pixel size the producer is currently bound at.
    pub(crate) binding: Option<(cherenkov::LayerId, (u32, u32))>,
}

impl GpuContentRuntime {
    pub(crate) const fn new(view: GpuContentView) -> Self {
        Self {
            view,
            producer: None,
            binding: None,
        }
    }
}

/// The `ExternalFrameView` a [`crate::renderer::tree::ExternalFrameNode`]
/// owns, and the stream's frame receiver once a mount has started it.
///
/// Unlike `GpuContent`, an external-frame source is restartable: the view
/// hands out a fresh [`ExternalFrameStream`] handle every call, and a lost
/// device or a reborn mount starts the source again with the new output.
/// `receiver` is `Some` once the first `ExternalFrameLayer` carrying this
/// runtime has started the source on the window's device.
pub struct ExternalFrameRuntime {
    pub(crate) view: ExternalFrameView,
    /// The mailbox drain end, installed by the compositor's install pass.
    pub(crate) receiver: Option<FrameReceiver>,
    /// The submitted-frame producer/sink pair the compositor created with
    /// the receiver: `sink` submits each new frame, `producer` re-issues the
    /// binding when the plane size changes.
    pub(crate) producer: Option<cherenkov::GpuProducer<cherenkov_gpu::Gpu>>,
    /// The submission end of `producer`'s pair.
    pub(crate) sink: Option<cherenkov::FrameSink<cherenkov_gpu::Gpu>>,
    /// The plane size of the last presented frame, for the stretch transform.
    pub(crate) frame_pixels: Option<(u32, u32)>,
    /// The engine layer and plane size the producer is currently bound at.
    pub(crate) binding: Option<(cherenkov::LayerId, (u32, u32))>,
}

impl ExternalFrameRuntime {
    pub(crate) const fn new(view: ExternalFrameView) -> Self {
        Self {
            view,
            receiver: None,
            producer: None,
            sink: None,
            frame_pixels: None,
            binding: None,
        }
    }
}
