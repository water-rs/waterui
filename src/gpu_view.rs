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
//! moves it into an engine `GpuContentHandle`, which one layer consumes at
//! install. A mount that drops its layer cannot be repopulated — the content
//! is gone — so a keyed mount presenting `GpuContent` is allowed to live for
//! the frame's whole key set, and transient (capture) windows never install
//! it at all: a capture cannot consume the one install the producer gets.

use waterui_graphics::gpu::GpuContentView;

/// The `GpuContentView` a [`crate::renderer::tree::GpuContentNode`] owns, and
/// whether its producer has been installed on an engine layer yet.
///
/// `installed` flips when the first `GpuContentLayer` carrying this runtime
/// reaches a persistent window's install pass — never on a transient target,
/// which would spend the view's single install on a surface that dies with
/// the call.
pub(crate) struct GpuContentRuntime {
    pub(crate) view: GpuContentView,
    /// `true` once `take_engine_content` has run; the producer is on the
    /// engine from then on and only `gpu_content_size`/transform edits apply.
    pub(crate) installed: bool,
}

impl GpuContentRuntime {
    pub(crate) fn new(view: GpuContentView) -> Self {
        Self {
            view,
            installed: false,
        }
    }
}
