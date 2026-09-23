//! Accordion component with a header and expandable content.

use crate::{ViewExt, component::Dynamic};
use nami::Binding;
use waterui_core::{View, handler::ViewBuilder};
use waterui_layout::stack::vstack;

/// An accordion component with a header and expandable content.
/// Content will be rendered lazily when the accordion is expanded. Its state may not be preserved when collapsed.
/// # Examples
/// ```rust
/// use waterui::prelude::*;
/// use waterui::widget::accordion;
/// accordion(
///     "Tap to Expand",
///     || "This is the expanded content"
/// );
/// ```
#[derive(Debug, Clone)]
pub struct Accordion<H, V> {
    toggle: Binding<bool>,
    header: H,
    content: V,
}

impl<H, F> Accordion<H, F>
where
    H: View,
    F: ViewBuilder,
{
    /// Creates a new accordion with the specified header and content.
    ///
    /// # Arguments
    /// * `header` - The view to display as the accordion header.
    /// * `content` - A function that generates the content view when the accordion is expanded.
    pub fn new(header: H, content: F) -> Self {
        Self::with_toggle(&Binding::bool(false), header, content)
    }

    /// Creates a new accordion with a custom toggle binding.
    /// This allows external control of the accordion's expanded/collapsed state.
    ///
    /// # Arguments
    /// * `toggle` - A binding that controls whether the accordion is expanded (true) or collapsed (false).
    /// * `header` - The view to display as the accordion header.
    /// * `content` - A function that generates the content view when the accordion
    pub fn with_toggle(toggle: &Binding<bool>, header: H, content: F) -> Self {
        Self {
            toggle: toggle.clone(),
            header,
            content,
        }
    }
}

/// Creates an accordion component with a header and expandable content.
pub fn accordion<H, F>(header: H, content: F) -> Accordion<H, F>
where
    H: View,
    F: ViewBuilder,
{
    Accordion::new(header, content)
}

impl<H, V> View for Accordion<H, V>
where
    H: View,
    V::Output: 'static + View,
    V: ViewBuilder,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let (handler, dynamic) = Dynamic::new();
        let toggle = self.toggle;
        vstack((
            self.header.on_tap(move || {
                toggle.toggle();
                if toggle.get() {
                    handler.set(self.content.build());
                } else {
                    handler.set(());
                }
            }),
            dynamic,
        ))
    }
}
