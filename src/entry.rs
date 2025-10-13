use waterui_core::View;

/// Wraps the root view for an application, allowing custom entry pipelines to
/// compose around it in the future.
pub const fn entry<V: View>(content: V) -> V {
    content
}
