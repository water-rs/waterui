//! Accordion component with a header and expandable content.

use crate::ViewExt;
use nami::{Binding, SignalExt as _};
use waterui_core::accessibility::{AccessibilityChildren, AccessibilityRole, AccessibilityState};
use waterui_core::{View, handler::ViewBuilder};
use waterui_layout::stack::vstack;

use super::condition::when;

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
        let toggle = self.toggle;
        let expanded = toggle.clone();
        // The header is a control: assistive technology and a test drive it
        // as one button, named by what the header shows, that reports whether
        // the content it reveals is showing — rather than as text that happens
        // to react to a tap. Its descendants are folded into that node, as a
        // button's label is, so the header is not announced twice.
        let state = expanded.map(|expanded| AccessibilityState::new().expanded(Some(expanded)));
        vstack((
            self.header
                .on_tap(move || {
                    toggle.toggle();
                })
                .a11y_role(AccessibilityRole::Button)
                .a11y_children(AccessibilityChildren::ExcludeDescendants)
                .a11y_state_signal(state),
            when(expanded, move || self.content.build()),
        ))
    }
}
