//! A text input component wired to a reactive string binding.
//!
//! ![`TextField`](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/text_field.svg)
pub use crate::input::{Capitalization, ContentType, KeyboardType, SubmitLabel};

use core::num::NonZeroUsize;

use alloc::vec::Vec;
use nami::signal::IntoComputed;
use nami::{Binding, Computed, SignalExt as _};
use waterui_core::Str;
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::handler::{Handler, SharedAction};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View, contract_violation};
use waterui_text::{IntoText, TextConfig, styled::StyledStr};

use crate::input::{FieldInput, ResolvedFieldInput};

use crate::label::{ControlLabel, IntoLabel, Label, LabelKind, impl_label_style_methods};
use crate::menu::{MenuItem, MenuView, ResolvedMenuItem, resolve_menu_items};
use crate::size::ControlSize;

mod private {
    pub trait Sealed {}
}

/// Value types accepted by [`TextField::new`] and [`field`].
///
/// Implemented for [`Str`] (plain text) and [`StyledStr`] (styled rich text),
/// so `&Binding<Str>` and `&Binding<StyledStr>` flow through the same
/// constructor.
pub trait TextFieldValue: private::Sealed + Clone + 'static {
    #[doc(hidden)]
    fn into_text_field_binding(binding: &Binding<Self>) -> Binding<StyledStr>;
}

impl private::Sealed for Str {}
impl private::Sealed for StyledStr {}

impl TextFieldValue for Str {
    fn into_text_field_binding(binding: &Binding<Self>) -> Binding<StyledStr> {
        map_plain_binding(binding)
    }
}

impl TextFieldValue for StyledStr {
    fn into_text_field_binding(binding: &Binding<Self>) -> Binding<StyledStr> {
        binding.clone()
    }
}

/// Configuration for the [`TextField`] component.
#[derive(Debug)]
#[non_exhaustive]
pub struct TextFieldConfig<S> {
    /// The label displayed for the text field.
    label: ControlLabel<S>,
    /// The binding to the text value.
    value: Binding<StyledStr>,
    /// Shared resolved input configuration (prompt, content type, submit label, submit action).
    input: ResolvedFieldInput,
    /// The type of keyboard to use for input.
    keyboard: KeyboardType,
    /// Resolved autocapitalization behavior for this field.
    autocapitalization: Capitalization,
    /// Resolved autocorrection behavior for this field.
    autocorrect: bool,
    /// Optional selected-text menu items resolved for the effective locale.
    selection_menu: Computed<Vec<ResolvedMenuItem<S>>>,
    /// Maximum number of lines the field renders. `None` removes the limit.
    line_limit: Computed<Option<NonZeroUsize>>,
    /// How large the text field is drawn.
    size: ControlSize,
}

impl<S> TextFieldConfig<S> {
    /// The label displayed for the text field.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// Sets the `label`.
    pub fn set_label(&mut self, label: ControlLabel<S>) {
        self.label = label;
    }

    /// The label displayed for the control, for editing it in place.
    pub const fn label_mut(&mut self) -> &mut ControlLabel<S> {
        &mut self.label
    }

    /// The binding to the text value.
    #[must_use]
    pub const fn value(&self) -> &Binding<StyledStr> {
        &self.value
    }

    /// The resolved placeholder text shown when the field is empty.
    #[must_use]
    pub const fn prompt(&self) -> &TextConfig {
        self.input.prompt()
    }

    /// The resolved type of keyboard to use for input.
    #[must_use]
    pub const fn keyboard(&self) -> KeyboardType {
        self.keyboard
    }

    /// The semantic content type the field declares for autofill.
    #[must_use]
    pub const fn content_type(&self) -> Option<ContentType> {
        self.input.content_type()
    }

    /// The resolved autocapitalization behavior for this field.
    #[must_use]
    pub const fn autocapitalization(&self) -> Capitalization {
        self.autocapitalization
    }

    /// Whether autocorrection is enabled for this field.
    #[must_use]
    pub const fn autocorrect(&self) -> bool {
        self.autocorrect
    }

    /// The label and action shown on the software keyboard's Return key.
    #[must_use]
    pub const fn submit_label(&self) -> SubmitLabel {
        self.input.submit_label()
    }

    /// Optional selected-text menu items resolved for the effective locale.
    #[must_use]
    pub const fn selection_menu(&self) -> &Computed<Vec<ResolvedMenuItem<S>>> {
        &self.selection_menu
    }

    /// Maximum number of lines the field renders. `None` removes the limit.
    #[must_use]
    pub const fn line_limit(&self) -> &Computed<Option<NonZeroUsize>> {
        &self.line_limit
    }

    /// The action run when the user submits the field with Return or Enter.
    ///
    /// A field without one leaves Return unconsumed, so it bubbles to the
    /// field's ancestors like any other key it does not use.
    #[must_use]
    pub const fn on_submit(&self) -> Option<&SharedAction> {
        self.input.on_submit()
    }

    /// How large the text field is drawn.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
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
        Capitalization,
        bool,
        SubmitLabel,
        Computed<Vec<ResolvedMenuItem<S>>>,
        Computed<Option<NonZeroUsize>>,
        Option<SharedAction>,
    ) {
        let (prompt, content_type, submit_label, on_submit) = self.input.into_parts();
        (
            self.label,
            self.value,
            prompt,
            self.keyboard,
            content_type,
            self.autocapitalization,
            self.autocorrect,
            submit_label,
            self.selection_menu,
            self.line_limit,
            on_submit,
        )
    }
}

impl<S: 'static> NativeView for TextFieldConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

impl<S: 'static> MapSlots<S> for TextFieldConfig<S> {
    type With<T: 'static> = TextFieldConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> TextFieldConfig<T> {
        let menu_f = f.clone();
        TextFieldConfig {
            label: self.label.map_slots(f),
            value: self.value,
            input: self.input,
            keyboard: self.keyboard,
            autocapitalization: self.autocapitalization,
            autocorrect: self.autocorrect,
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
            size: self.size,
        }
    }
}

/// A single-line text input field.
#[derive(Debug)]
#[must_use]
pub struct TextField<L> {
    /// The label displayed for the text field.
    ///
    /// Always present: it is required at construction so assistive technology
    /// has a name to announce, even when
    /// [`LabelDisplayMode::Hidden`](crate::label::LabelDisplayMode::Hidden)
    /// removes the visible chrome.
    label: Label<L>,
    /// The binding to the text value.
    value: Binding<StyledStr>,
    /// Shared input configuration (prompt, content type, submit label, submit action).
    input: FieldInput,
    /// Explicit keyboard override; when `None`, derived from `content_type`.
    keyboard: Option<KeyboardType>,
    /// Explicit autocapitalization override; when `None`, derived from `content_type` or `keyboard`.
    autocapitalization: Option<Capitalization>,
    /// Explicit autocorrection override; when `None`, derived from `content_type` or `keyboard`.
    autocorrect: Option<bool>,
    /// Optional selected-text menu items.
    ///
    /// These actions are shown by native text selection UI.
    selection_menu: Computed<Vec<MenuItem>>,
    /// Maximum number of lines the field renders. `None` removes the limit.
    line_limit: Computed<Option<NonZeroUsize>>,
    /// Explicit control size override; when `None`, inherits the nearest ambient [`ControlSize`].
    size: Option<ControlSize>,
}

impl<L> TextField<L> {
    /// Creates a new `TextField` with the given label and value binding.
    ///
    /// The label is mandatory — see [the label module](mod@crate::label). It names
    /// the field for assistive technology and is distinct from
    /// [`Self::prompt`], which is the placeholder shown inside an empty field.
    /// Chain [`Self::hide_label`] to drop the visible label while keeping the
    /// accessibility name.
    pub fn new<I: IntoLabel<Kind = L>, V: TextFieldValue>(label: I, value: &Binding<V>) -> Self {
        Self {
            label: label.into_label(),
            value: V::into_text_field_binding(value),
            input: FieldInput::new(),
            keyboard: None,
            autocapitalization: None,
            autocorrect: None,
            selection_menu: Computed::constant(Vec::new()),
            line_limit: Computed::constant(NonZeroUsize::new(1)),
            size: None,
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Sets the maximum number of lines to show.
    ///
    /// By default, the line limit is `Some(1)`. Pass `None` to remove the
    /// limit, `NonZeroUsize::new(n)` or a `NonZeroUsize` for a fixed cap, or a
    /// reactive signal producing `Option<NonZeroUsize>` to change the limit in
    /// place.
    pub fn line_limit(mut self, line_limit: impl IntoComputed<Option<NonZeroUsize>>) -> Self {
        self.line_limit = line_limit.into_computed();
        self
    }

    /// Runs `action` when the user submits the field with Return or Enter.
    ///
    /// In a field without a line limit Return inserts a line break and never
    /// submits.
    pub fn on_submit<Args>(mut self, action: impl Handler<Args, ()>) -> Self {
        self.input.set_on_submit(action);
        self
    }

    /// Sets the prompt for the text field.
    pub fn prompt(mut self, prompt: impl IntoText) -> Self {
        self.input.set_prompt(prompt);
        self
    }

    /// Sets custom selected-text menu items for this text field.
    pub fn selection_menu(mut self, items: impl MenuView) -> Self {
        self.selection_menu = items.into_menu_items();
        self
    }

    /// The binding backing this field's text.
    ///
    /// This is the escape hatch that lets wrappers observe or replace the
    /// field's text source — `waterui_form::valid::ValidatableView` uses it to
    /// splice a validating binding in front of the field.
    pub const fn value_binding(&mut self) -> &mut Binding<StyledStr> {
        &mut self.value
    }

    /// Sets the on-screen keyboard the field requests while editing.
    ///
    /// Overrides any default keyboard implied by [`Self::content_type`].
    /// Platforms without a software keyboard ignore this hint.
    pub const fn keyboard(mut self, keyboard: KeyboardType) -> Self {
        self.keyboard = Some(keyboard);
        self
    }

    /// Sets the semantic content type the field declares for autofill —
    /// what the field's content means, not how it looks. On Apple platforms
    /// this drives the autofill suggestions above the keyboard, including the
    /// one-time code the system offers for a received SMS; on Android the
    /// autofill service sees the matching hint.
    ///
    /// Also sets sensible defaults for keyboard, autocapitalization, and
    /// autocorrection unless explicitly overridden.
    pub const fn content_type(mut self, content_type: ContentType) -> Self {
        self.input.set_content_type(content_type);
        self
    }

    /// Sets how the input system capitalizes typed text.
    ///
    /// Overrides the default derived from [`Self::content_type`] or [`Self::keyboard`].
    pub const fn autocapitalization(mut self, capitalization: Capitalization) -> Self {
        self.autocapitalization = Some(capitalization);
        self
    }

    /// Sets whether the input system autocorrects typed text.
    ///
    /// Overrides the default derived from [`Self::content_type`] or [`Self::keyboard`].
    pub const fn autocorrect(mut self, enabled: bool) -> Self {
        self.autocorrect = Some(enabled);
        self
    }

    /// Sets the label and action shown on the software keyboard's Return key.
    pub const fn submit_label(mut self, label: SubmitLabel) -> Self {
        self.input.set_submit_label(label);
        self
    }

    /// Sets how large the text field is drawn. When omitted, the field inherits
    /// the nearest ambient [`ControlSize`] or defaults to
    /// [`ControlSize::Small`].
    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = Some(size);
        self
    }
}

impl<L: LabelKind> TextField<L> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> TextFieldConfig<S> {
        let selection_menu = resolve_menu_items(&self.selection_menu, env);
        let content_type = self.input.content_type();
        let keyboard = self
            .keyboard
            .or_else(|| content_type.map(ContentType::default_keyboard))
            .unwrap_or_default();
        let autocapitalization = self
            .autocapitalization
            .or_else(|| content_type.map(ContentType::default_capitalization))
            .unwrap_or_else(|| keyboard.default_capitalization());
        let autocorrect = self
            .autocorrect
            .or_else(|| content_type.map(ContentType::default_autocorrect))
            .unwrap_or_else(|| keyboard.default_autocorrect());
        TextFieldConfig {
            label: self.label.into_control_label::<S>(env),
            value: self.value,
            input: self.input.resolve(env),
            keyboard,
            autocapitalization,
            autocorrect,
            selection_menu,
            line_limit: self.line_limit,
            size: ControlSize::resolve(self.size, env, ControlSize::Small),
        }
    }
}

impl<L: LabelKind> View for TextField<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(TextField<L>);

/// Creates a new [`TextField`] with the specified label and value binding.
pub fn field<L: IntoLabel, V: TextFieldValue>(label: L, value: &Binding<V>) -> TextField<L::Kind> {
    TextField::new(label, value)
}

fn map_plain_binding(value: &Binding<Str>) -> Binding<StyledStr> {
    Binding::mapping(value, StyledStr::plain, |plain_binding, styled| {
        if !styled.is_plain() {
            contract_violation!(
                "TextField::new(label, &Binding<Str>) cannot accept styled text updates; pass a &Binding<StyledStr> for styled text; the styling is dropped and the plain text is stored"
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
        let value = Binding::new(Str::from(""));
        let _ = TextField::new("Note", &value).line_limit(NonZeroUsize::new(2));
    }

    #[test]
    fn line_limit_none_removes_limit() {
        let value = Binding::new(Str::from(""));
        let _ = TextField::new("Note", &value).line_limit(None::<NonZeroUsize>);
    }

    #[test]
    fn line_limit_accepts_reactive_signal() {
        let value = Binding::new(Str::from(""));
        let limit = Binding::new(NonZeroUsize::new(2));
        let field = TextField::new("Note", &value).line_limit(limit.clone());
        assert_eq!(field.line_limit.snapshot(), NonZeroUsize::new(2));
        limit.set(None);
        assert_eq!(field.line_limit.snapshot(), None);
    }

    #[test]
    fn new_constructor_accepts_styled_binding() {
        let styled = Binding::new(StyledStr::plain("a"));
        let _ = TextField::new("Body", &styled);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "cannot accept styled text updates")
    )]
    fn plain_mapping_reports_styled_write_back() {
        let plain = Binding::new(Str::from("a"));
        let mapped = super::map_plain_binding(&plain);
        mapped.set(StyledStr::plain("b").italic(true));

        // Release outcome: the styling is dropped and the plain text stored.
        assert_eq!(plain.snapshot(), "b");
    }

    #[test]
    fn plain_mapping_updates_nested_mapped_source() {
        let source = Binding::new(String::from("a"));
        let plain = Binding::mapping(&source, Str::from, |binding, value: Str| {
            binding.set(value.into());
        });
        let mapped = super::map_plain_binding(&plain);
        mapped.set(StyledStr::plain("updated"));

        assert_eq!(source.snapshot(), "updated");
    }

    #[test]
    fn plain_field_uses_sentence_capitalization_and_autocorrect() {
        use super::{Capitalization, KeyboardType, SubmitLabel};
        use waterui_core::{AnyView, Environment};

        let value = Binding::new(Str::from(""));
        let config = TextField::new("Note", &value).into_config::<AnyView>(&Environment::new());
        assert_eq!(config.keyboard(), KeyboardType::Text);
        assert_eq!(config.content_type(), None);
        assert_eq!(config.autocapitalization(), Capitalization::Sentences);
        assert!(config.autocorrect());
        assert_eq!(config.submit_label(), SubmitLabel::Return);
    }

    #[test]
    fn content_type_implies_input_defaults_and_explicit_modifiers_win() {
        use super::{Capitalization, ContentType, KeyboardType, SubmitLabel};
        use waterui_core::{AnyView, Environment};

        let env = Environment::new();
        let value = Binding::new(Str::from(""));

        let email = TextField::new("Email", &value)
            .content_type(ContentType::EmailAddress)
            .into_config::<AnyView>(&env);
        assert_eq!(email.keyboard(), KeyboardType::Email);
        assert_eq!(email.autocapitalization(), Capitalization::None);
        assert!(!email.autocorrect());

        let name = TextField::new("Name", &value)
            .content_type(ContentType::PersonName)
            .into_config::<AnyView>(&env);
        assert_eq!(name.keyboard(), KeyboardType::Text);
        assert_eq!(name.autocapitalization(), Capitalization::Words);
        assert!(!name.autocorrect());

        let postal = TextField::new("Postal code", &value)
            .content_type(ContentType::PostalCode)
            .into_config::<AnyView>(&env);
        assert_eq!(postal.autocapitalization(), Capitalization::Characters);
        assert!(!postal.autocorrect());

        let otp = TextField::new("Code", &value)
            .content_type(ContentType::OneTimeCode)
            .into_config::<AnyView>(&env);
        assert_eq!(otp.autocapitalization(), Capitalization::None);
        assert!(!otp.autocorrect());

        // Explicit modifiers override derived content-type defaults regardless of call order.
        let overridden = TextField::new("Email", &value)
            .autocapitalization(Capitalization::Words)
            .autocorrect(true)
            .keyboard(KeyboardType::Text)
            .submit_label(SubmitLabel::Next)
            .content_type(ContentType::EmailAddress)
            .into_config::<AnyView>(&env);
        assert_eq!(overridden.keyboard(), KeyboardType::Text);
        assert_eq!(overridden.autocapitalization(), Capitalization::Words);
        assert!(overridden.autocorrect());
        assert_eq!(overridden.submit_label(), SubmitLabel::Next);
    }

    #[test]
    fn keyboard_type_implies_defaults_and_supports_decimal() {
        use super::{Capitalization, KeyboardType};
        use waterui_core::{AnyView, Environment};

        let env = Environment::new();
        let value = Binding::new(Str::from(""));

        let url = TextField::new("Website", &value)
            .keyboard(KeyboardType::URL)
            .into_config::<AnyView>(&env);
        assert_eq!(url.keyboard(), KeyboardType::URL);
        assert_eq!(url.autocapitalization(), Capitalization::None);
        assert!(!url.autocorrect());

        let price = TextField::new("Price", &value)
            .keyboard(KeyboardType::Decimal)
            .into_config::<AnyView>(&env);
        assert_eq!(price.keyboard(), KeyboardType::Decimal);
        assert_eq!(price.autocapitalization(), Capitalization::None);
        assert!(!price.autocorrect());
    }

    #[test]
    fn text_field_control_size_inherits_from_environment_and_explicit_size_wins() {
        use crate::size::ControlSize;
        use waterui_core::plugin::Plugin as _;
        use waterui_core::{AnyView, Environment};

        let value = Binding::new(Str::from(""));
        let base_env = Environment::new();
        let mut ambient_env = Environment::new();
        ControlSize::ExtraSmall.install(&mut ambient_env);

        assert_eq!(
            TextField::new("Name", &value)
                .into_config::<AnyView>(&base_env)
                .size(),
            ControlSize::Small
        );
        assert_eq!(
            TextField::new("Name", &value)
                .into_config::<AnyView>(&ambient_env)
                .size(),
            ControlSize::ExtraSmall
        );
        assert_eq!(
            TextField::new("Name", &value)
                .size(ControlSize::Large)
                .into_config::<AnyView>(&ambient_env)
                .size(),
            ControlSize::Large
        );
    }
}
