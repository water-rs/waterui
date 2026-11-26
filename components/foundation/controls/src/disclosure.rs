//! Disclosure component with a focusable header, indicator, and lazy expandable content.

use core::fmt;

use nami::Binding;
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::handler::ViewBuilder;
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View};

use crate::label::{ControlLabel, IntoLabel, Label, LabelKind};
use crate::slot::SlotBuilder;

/// Resolved configuration passed to `NativeBackend` for a [`Disclosure`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DisclosureConfig<S> {
    header: ControlLabel<S>,
    content: SlotBuilder<S>,
    expanded: Binding<bool>,
}

impl<S> DisclosureConfig<S> {
    /// Returns the disclosure header label.
    #[must_use]
    pub const fn header(&self) -> &ControlLabel<S> {
        &self.header
    }

    /// Returns the lazy content builder.
    #[must_use]
    pub const fn content(&self) -> &SlotBuilder<S> {
        &self.content
    }

    /// Returns the binding controlling whether the disclosure content is expanded.
    #[must_use]
    pub const fn expanded(&self) -> &Binding<bool> {
        &self.expanded
    }

    /// Deconstructs the configuration into its header label, content builder, and expanded binding.
    #[must_use]
    pub fn into_parts(self) -> (ControlLabel<S>, SlotBuilder<S>, Binding<bool>) {
        (self.header, self.content, self.expanded)
    }
}

impl<S: 'static> MapSlots<S> for DisclosureConfig<S> {
    type With<T: 'static> = DisclosureConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> DisclosureConfig<T> {
        let map_content = f.clone();
        let content = self.content;
        DisclosureConfig {
            header: self.header.map_slots(f),
            content: SlotBuilder::new(move || map_content(content.build())),
            expanded: self.expanded,
        }
    }
}

impl<S: 'static> NativeView for DisclosureConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

/// A disclosure component with a header label, platform indicator, and expandable content.
///
/// Content is built lazily when expanded; its state may not be preserved when collapsed.
#[derive(Debug, Clone)]
#[must_use]
pub struct Disclosure<L, F> {
    header: Label<L>,
    content: F,
    expanded: Binding<bool>,
}

impl<L, F> Disclosure<L, F>
where
    F: ViewBuilder,
    F::Output: View,
{
    /// Creates a new disclosure with the specified header label and lazy content builder.
    pub fn new<I: IntoLabel<Kind = L>>(header: I, content: F) -> Self {
        Self {
            header: header.into_label(),
            content,
            expanded: Binding::new(false),
        }
    }

    /// Binds whether the disclosure is expanded to an external [`Binding<bool>`].
    pub fn expanded(mut self, expanded: &Binding<bool>) -> Self {
        self.expanded = expanded.clone();
        self
    }
}

impl<L: LabelKind, F: ViewBuilder + 'static> Disclosure<L, F>
where
    F::Output: View + 'static,
{
    fn into_config<S: ChildSlot>(self, env: &Environment) -> DisclosureConfig<S> {
        let content = self.content;
        DisclosureConfig {
            header: self.header.into_control_label::<S>(env),
            content: SlotBuilder::new(move || S::from_view(content.build())),
            expanded: self.expanded,
        }
    }
}

/// Creates a disclosure component with a header label and lazy expandable content.
pub fn disclosure<I: IntoLabel, F: ViewBuilder>(header: I, content: F) -> Disclosure<I::Kind, F>
where
    F::Output: View,
{
    Disclosure::new(header, content)
}

impl<L: LabelKind, F: ViewBuilder + 'static> View for Disclosure<L, F>
where
    F::Output: View + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl<S: 'static> fmt::Display for DisclosureConfig<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Disclosure")
    }
}
