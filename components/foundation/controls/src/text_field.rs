//! A text input component wired to a reactive string binding.
//!
//! ![`TextField`](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/text_field.svg)
pub use crate::input::{ContentType, KeyboardType};

use core::num::NonZeroUsize;

use alloc::vec::Vec;
use nami::{Binding, Computed, SignalExt as _};
use waterui_core::Str;
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::handler::{Handler, SharedAction};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View, contract_violation};
use waterui_text::{IntoText, Text, TextConfig, styled::StyledStr};

use crate::label::{ControlLabel, IntoLabel, Label, LabelKind, impl_label_style_methods};
use crate::menu::{MenuItem, MenuView, ResolvedMenuItem, resolve_menu_items};

/// Configuration options for a `TextField`.
#[non_exhaustive]
#[derive(Debug)]
pub struct TextFieldConfig<L> {
    /// The label displayed for the text field.
    ///
    /// Always present: it is required at construction so assistive technology
    /// has a name to announce, even when
    /// [`LabelDisplayMode::Hidden`](crate::label::LabelDisplayMode::Hidden)
    /// removes the visible chrome.
    label: Label<L>,
    /// The binding to the text value.
    value: Binding<StyledStr>,
    /// The placeholder text shown when the field is empty.
    prompt: Text,
    /// The type of keyboard to use for input.
    keyboard: KeyboardType,
    /// The semantic content type the field declares for autofill.
    ///
    /// `None` declares none — the field is not offered to autofill services.
    content_type: Option<ContentType>,
    /// Optional selected-text menu items.
    ///
    /// These actions are shown by native text selection UI.
    selection_menu: Computed<Vec<MenuItem>>,
    /// Maximum number of lines the field renders. `None` removes the limit.
    line_limit: Option<NonZeroUsize>,
    /// The action run when the user submits the field with Return or Enter.
    ///
    /// A field without one leaves Return unconsumed, so it bubbles to the
    /// field's ancestors like any other key it does not use.
    on_submit: Option<SharedAction>,
}

impl<L> TextFieldConfig<L> {
    /// The label displayed for the text field.
    ///
    /// Always present: it is required at construction so assistive technology
    /// has a name to announce, even when
    /// [`LabelDisplayMode::Hidden`](crate::label::LabelDisplayMode::Hidden)
    /// removes the visible chrome.
    #[must_use]
    pub const fn label(&self) -> &Label<L> {
        &self.label
    }

    /// Sets the `label`.
    pub fn set_label(&mut self, label: Label<L>) {
        self.label = label;
    }

    /// The label displayed for the control, for editing it in place.
    pub const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// The binding to the text value.
    #[must_use]
    pub const fn value(&self) -> &Binding<StyledStr> {
        &self.value
    }

    /// The placeholder text shown when the field is empty.
    #[must_use]
    pub const fn prompt(&self) -> &Text {
        &self.prompt
    }

    /// The type of keyboard to use for input.
    #[must_use]
    pub const fn keyboard(&self) -> &KeyboardType {
        &self.keyboard
    }

    /// The semantic content type the field declares for autofill.
    ///
    /// `None` declares none — the field is not offered to autofill services.
    #[must_use]
    pub const fn content_type(&self) -> Option<ContentType> {
        self.content_type
    }

    /// Optional selected-text menu items.
    ///
    /// These actions are shown by native text selection UI.
    #[must_use]
    pub const fn selection_menu(&self) -> &Computed<Vec<MenuItem>> {
        &self.selection_menu
    }

    /// Maximum number of lines the field renders. `None` removes the limit.
    #[must_use]
    pub const fn line_limit(&self) -> Option<NonZeroUsize> {
        self.line_limit
    }

    /// The action run when the user submits the field with Return or Enter.
    ///
    /// A field without one leaves Return unconsumed, so it bubbles to the
    /// field's ancestors like any other key it does not use.
    #[must_use]
    pub const fn on_submit(&self) -> Option<&SharedAction> {
        self.on_submit.as_ref()
    }
}

/// Native-resolved text field payload.
#[derive(Debug)]
#[non_exhaustive]
pub struct ResolvedTextFieldConfig<S> {
    /// The label displayed for the text field.
    label: ControlLabel<S>,
    /// The binding to the text value.
    value: Binding<StyledStr>,
    /// The resolved placeholder text shown when the field is empty.
    prompt: TextConfig,
    /// The type of keyboard to use for input.
    keyboard: KeyboardType,
    /// The semantic content type the field declares for autofill.
    content_type: Option<ContentType>,
    /// Optional selected-text menu items resolved for the effective locale.
    selection_menu: Computed<Vec<ResolvedMenuItem<S>>>,
    /// Maximum number of lines the field renders. `None` removes the limit.
    line_limit: Option<NonZeroUsize>,
    /// The action run when the user submits the field with Return or Enter.
    ///
    /// A field without one leaves Return unconsumed, so it bubbles to the
    /// field's ancestors like any other key it does not use.
    on_submit: Option<SharedAction>,
}

impl<S> ResolvedTextFieldConfig<S> {
    /// The label displayed for the text field.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// The binding to the text value.
    #[must_use]
    pub const fn value(&self) -> &Binding<StyledStr> {
        &self.value
    }

    /// The resolved placeholder text shown when the field is empty.
    #[must_use]
    pub const fn prompt(&self) -> &TextConfig {
        &self.prompt
    }

    /// The type of keyboard to use for input.
    #[must_use]
    pub const fn keyboard(&self) -> &KeyboardType {
        &self.keyboard
    }

    /// The semantic content type the field declares for autofill.
    #[must_use]
    pub const fn content_type(&self) -> Option<ContentType> {
        self.content_type
    }

    /// Optional selected-text menu items resolved for the effective locale.
    #[must_use]
    pub const fn selection_menu(&self) -> &Computed<Vec<ResolvedMenuItem<S>>> {
        &self.selection_menu
    }

    /// Maximum number of lines the field renders. `None` removes the limit.
    #[must_use]
    pub const fn line_limit(&self) -> Option<NonZeroUsize> {
        self.line_limit
    }

    /// The action run when the user submits the field with Return or Enter.
    ///
    /// A field without one leaves Return unconsumed, so it bubbles to the
    /// field's ancestors like any other key it does not use.
    #[must_use]
    pub const fn on_submit(&self) -> Option<&SharedAction> {
        self.on_submit.as_ref()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    #[expect(
        clippy::type_complexity,
        reason = "the tuple is the value's complete field list"
    )]
    pub fn into_parts(
        self,
    ) -> (
        ControlLabel<S>,
        Binding<StyledStr>,
        TextConfig,
        KeyboardType,
        Option<ContentType>,
        Computed<Vec<ResolvedMenuItem<S>>>,
        Option<NonZeroUsize>,
        Option<SharedAction>,
    ) {
        (
            self.label,
            self.value,
            self.prompt,
            self.keyboard,
            self.content_type,
            self.selection_menu,
            self.line_limit,
            self.on_submit,
        )
    }
}

impl<S: 'static> NativeView for ResolvedTextFieldConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

impl<S: 'static> MapSlots<S> for ResolvedTextFieldConfig<S> {
    type With<T: 'static> = ResolvedTextFieldConfig<T>;

    fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> ResolvedTextFieldConfig<T> {
        let menu_f = f.clone();
        ResolvedTextFieldConfig {
            label: self.label.map_slots(f),
            value: self.value,
            prompt: self.prompt,
            keyboard: self.keyboard,
            content_type: self.content_type,
            selection_menu: self
                .selection_menu
                .map(move |items| {
                    items
                        .into_iter()
                        .map(|item| item.map_slots(menu_f.clone()))
                        .collect()
                })
                .computed(),
            line_limit: self.line_limit,
            on_submit: self.on_submit,
        }
    }
}

/// Resolved text field view passed to native backends.
#[derive(Debug)]
#[must_use]
pub struct ResolvedTextField<S>(pub ResolvedTextFieldConfig<S>);

impl<S: View> From<ResolvedTextFieldConfig<S>> for ResolvedTextField<S> {
    fn from(value: ResolvedTextFieldConfig<S>) -> Self {
        Self(value)
    }
}

impl<S: View> View for ResolvedTextField<S> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.0.map_slots(AnyView::new))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.0.map_slots(B::Slot::from_view), env, cx)
    }
}

/// A single-line text input field.
#[derive(Debug)]
#[must_use]
pub struct TextField<L>(TextFieldConfig<L>);

impl<L> TextField<L> {
    /// Creates a new `TextField` with the given label and value binding.
    ///
    /// The label is mandatory — see [the label module](mod@crate::label). It names
    /// the field for assistive technology and is distinct from
    /// [`Self::prompt`], which is the placeholder shown inside an empty field.
    /// Chain [`Self::hide_label`] to drop the visible label while keeping the
    /// accessibility name.
    pub fn new<I: IntoLabel<Kind = L>>(label: I, value: &Binding<Str>) -> Self {
        let styled_value = map_plain_binding(value);
        Self::styled(label, &styled_value)
    }

    /// Creates a new `TextField` with the given label, backed by a styled text
    /// binding.
    pub fn styled<I: IntoLabel<Kind = L>>(label: I, value: &Binding<StyledStr>) -> Self {
        Self(TextFieldConfig {
            label: label.into_label(),
            value: value.clone(),
            prompt: Text::default(),
            keyboard: KeyboardType::default(),
            content_type: None,
            selection_menu: Computed::constant(Vec::new()),
            line_limit: NonZeroUsize::new(1),
            on_submit: None,
        })
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        self.0.label_mut()
    }

    /// Sets the maximum number of lines to show.
    ///
    /// By default, the line limit is 1. The argument is a [`NonZeroUsize`] so
    /// that "zero lines" is rejected at the type level rather than at runtime.
    /// Use [`Self::disable_line_limit`] to remove the limit entirely.
    pub const fn line_limit(mut self, line_limit: NonZeroUsize) -> Self {
        self.0.line_limit = Some(line_limit);
        self
    }

    /// Disables the line limit.
    pub const fn disable_line_limit(mut self) -> Self {
        self.0.line_limit = None;
        self
    }

    /// Runs `action` when the user submits the field with Return or Enter.
    ///
    /// In a field without a line limit Return inserts a line break and never
    /// submits.
    pub fn on_submit<Args>(mut self, action: impl Handler<Args, ()>) -> Self {
        self.0.on_submit = Some(SharedAction::new(action));
        self
    }

    /// Sets the prompt for the text field.
    pub fn prompt(mut self, prompt: impl IntoText) -> Self {
        self.0.prompt = prompt.into_text();
        self
    }

    /// Sets custom selected-text menu items for this text field.
    pub fn selection_menu(mut self, items: impl MenuView) -> Self {
        self.0.selection_menu = items.into_menu_items();
        self
    }

    /// The binding backing this field's text.
    ///
    /// This is the escape hatch that lets wrappers observe or replace the
    /// field's text source — `waterui_form::valid::ValidatableView` uses it to
    /// splice a validating binding in front of the field.
    pub const fn value_binding(&mut self) -> &mut Binding<StyledStr> {
        &mut self.0.value
    }

    /// Sets the on-screen keyboard the field requests while editing.
    ///
    /// Platforms without a software keyboard ignore this hint.
    pub const fn keyboard(mut self, keyboard: KeyboardType) -> Self {
        self.0.keyboard = keyboard;
        self
    }

    /// Sets the semantic content type the field declares for autofill —
    /// what the field's content means, not how it looks. On Apple platforms
    /// this drives the autofill suggestions above the keyboard, including the
    /// one-time code the system offers for a received SMS; on Android the
    /// autofill service sees the matching hint.
    pub const fn content_type(mut self, content_type: ContentType) -> Self {
        self.0.content_type = Some(content_type);
        self
    }
}

impl<L: LabelKind> TextField<L> {
    fn into_resolved<S: ChildSlot>(self, env: &Environment) -> ResolvedTextFieldConfig<S> {
        let selection_menu = resolve_menu_items(&self.0.selection_menu, env);
        ResolvedTextFieldConfig {
            label: self.0.label.into_control_label::<S>(env),
            value: self.0.value,
            prompt: self.0.prompt.resolve(env),
            keyboard: self.0.keyboard,
            content_type: self.0.content_type,
            selection_menu,
            line_limit: self.0.line_limit,
            on_submit: self.0.on_submit,
        }
    }
}

impl<L: LabelKind> View for TextField<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_resolved::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_resolved::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(TextField<L>);

/// Creates a new [`TextField`] with the specified label and value binding.
pub fn field<L: IntoLabel>(label: L, value: &Binding<Str>) -> TextField<L::Kind> {
    TextField::new(label, value)
}

fn map_plain_binding(value: &Binding<Str>) -> Binding<StyledStr> {
    Binding::mapping(value, StyledStr::plain, |plain_binding, styled| {
        if !styled.is_plain() {
            contract_violation!(
                "TextField::new(label, &Binding<Str>) cannot accept styled text updates; use TextField::styled(label, &Binding<StyledStr>); the styling is dropped and the plain text is stored"
            );
        }
        plain_binding.set(styled.to_plain());
    })
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use nami::{Binding, Signal};
    use waterui_core::Str;
    use waterui_text::styled::StyledStr;

    use core::num::NonZeroUsize;

    use super::TextField;

    #[test]
    fn line_limit_non_one_is_supported() {
        let value = Binding::container(Str::from(""));
        let _ = TextField::new("Note", &value).line_limit(NonZeroUsize::new(2).unwrap());
    }

    #[test]
    fn disable_line_limit_does_not_panic() {
        let value = Binding::container(Str::from(""));
        let _ = TextField::new("Note", &value).disable_line_limit();
    }

    #[test]
    fn styled_constructor_accepts_styled_binding() {
        let styled = Binding::container(StyledStr::plain("a"));
        let _ = TextField::styled("Body", &styled);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "cannot accept styled text updates")
    )]
    fn plain_mapping_reports_styled_write_back() {
        let plain = Binding::container(Str::from("a"));
        let mapped = super::map_plain_binding(&plain);
        mapped.set(StyledStr::plain("b").italic(true));

        // Release outcome: the styling is dropped and the plain text stored.
        assert_eq!(plain.snapshot(), "b");
    }

    #[test]
    fn plain_mapping_updates_nested_mapped_source() {
        let source = Binding::container(String::from("a"));
        let plain = Binding::mapping(&source, Str::from, |binding, value: Str| {
            binding.set(value.into());
        });
        let mapped = super::map_plain_binding(&plain);
        mapped.set(StyledStr::plain("updated"));

        assert_eq!(source.snapshot(), "updated");
    }
}
