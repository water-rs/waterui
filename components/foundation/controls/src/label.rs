//! Semantic labels shared by controls, menus, and chrome.
//!
//! # Why every control demands a label
//!
//! `WaterUI` controls (Slider, Stepper, Toggle, Button, `ColorPicker`, Calendar,
//! `DatePicker`, `FilePicker`, `MultiDatePicker`, `TextField`, `SecureField`) take a
//! semantic [`Label`] at construction time. The label is required even if you
//! intend to **hide it visually** with [`Label::hide_label`] or
//! [`LabelDisplayMode::Hidden`].
//!
//! ## Why force it?
//!
//! - **Accessibility is non-negotiable.** Screen readers
//!   (`VoiceOver` / `TalkBack` / Narrator), voice control, switch control, and
//!   command palettes all read the semantic label to announce or activate a
//!   control. A control with no label appears as an anonymous, unreachable
//!   widget — users of assistive technology cannot interact with it.
//! - **Visual hide is a presentation concern, not a semantic one.** Hiding the
//!   visible chrome with `.hide_label()` keeps the label in the accessibility
//!   tree. The control still has a name; it just isn't drawn. Compact toolbars,
//!   icon-only buttons, paired-icon-and-control rows, and tightly packed
//!   forms can hide the label without losing accessibility.
//! - **Type-level enforcement beats runtime guidance.** Putting the label in
//!   the constructor signature is the cheapest way to make sure every
//!   `WaterUI` app ships with an accessible UI by default. There is no
//!   `Slider::anonymous(&binding)` escape hatch — author intent is recorded
//!   structurally, not in a comment.
//!
//! ## When a "decorative" control feels label-less
//!
//! Sometimes the label seems redundant — a slider next to a brightness icon,
//! say. The right pattern is `slider("Brightness", &value).hide_label()`. The
//! visual chrome adapts (icon only); the accessibility tree announces
//! "Brightness". The icon is presentation; the label is meaning.
//!
//! ## See also
//!
//! - [`LabelDisplayMode`] for the visual modes a label can take
//! - [`Label::hide_label`] / [`Label::display_mode`] for per-label control

use core::any::Any;
use nami::{Binding, Computed, SignalExt};
use waterui_core::{
    AnyView, Environment, IntoSignalF32, View,
    handler::{AnyViewBuilder, ViewBuilder},
    plugin::Plugin,
};
use waterui_icon::SystemIcon;
use waterui_layout::stack::hstack;
use waterui_text::{IntoText, Text, font::Font, styled::StyledStr};

/// Position of the icon relative to the text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IconPosition {
    /// Icon appears before the text (left in LTR).
    #[default]
    Leading,
    /// Icon appears after the text (right in LTR).
    Trailing,
}

nami::impl_constant!(IconPosition);

/// Controls how a semantic label should present its text and icon.
///
/// `Label` keeps its semantic text even when the visual presentation switches
/// to icon-only. Native backends and hydrolysis accessibility use that semantic
/// text so screen readers still announce the title in compact chrome.
///
/// Install this as a plugin on a subtree to adapt labels for chrome such as
/// toolbars or compact layouts:
///
/// ```rust,ignore
/// hstack((
///     button(label("Search").system_icon(system_icon::search())).action(|| {}),
///     button(label("Settings").system_icon(system_icon::settings())).action(|| {}),
/// ))
/// .install(LabelDisplayMode::IconOnly)
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LabelDisplayMode {
    /// Use the label's own preferred presentation.
    #[default]
    Automatic,
    /// Show both title and icon when an icon is available.
    TitleAndIcon,
    /// Show only the title text.
    TitleOnly,
    /// Show only the icon when an icon is available.
    IconOnly,
    /// Visually omit both title and icon.
    ///
    /// The label collapses to a zero-size view, but the semantic text is
    /// still carried by [`Label::semantic_text`] for parent components and
    /// assistive technology. Use this when the label's purpose is fully
    /// described by adjacent context (for example, a slider beside an icon
    /// whose meaning is obvious from layout) but the accessibility tree must
    /// still announce it.
    Hidden,
}

nami::impl_constant!(LabelDisplayMode);

impl Plugin for LabelDisplayMode {}

impl Default for Label {
    fn default() -> Self {
        Self::semantic(Text::default())
    }
}

/// Sets the display mode in place. Companion to the consuming
/// [`Label::display_mode`] builder, used by the [`impl_label_style_methods`]
/// macro to mutate a control's `label: Label` field without taking ownership.
impl Label {
    /// Mutates the display mode in place. See [`Label::display_mode`] for the
    /// owning builder variant.
    pub const fn set_display_mode(&mut self, mode: LabelDisplayMode) {
        self.display_mode = mode;
    }

    /// Returns the configured display mode prior to environment resolution.
    #[must_use]
    pub const fn display_mode_preference(&self) -> LabelDisplayMode {
        self.display_mode
    }
}

/// Emits `label_style(LabelDisplayMode)` and `hide_label()` builders on a
/// `configurable!`-style wrapper struct (`Foo(FooConfig)`) whose inner
/// config carries a [`Label`] field named `label`.
///
/// Reused by every `WaterUI` control crate so the same `.label_style(...)` /
/// `.hide_label()` surface appears on every `configurable!`-style control
/// without forcing a trait into the user's import path.
#[macro_export]
macro_rules! impl_label_style_methods {
    ($ty:ident) => {
        impl $ty {
            /// Sets the visual presentation mode for this control's label.
            ///
            /// The semantic text of the label is always retained for assistive
            /// technology regardless of the chosen visual mode.
            #[must_use]
            pub const fn label_style(mut self, mode: $crate::label::LabelDisplayMode) -> Self {
                self.0.label.set_display_mode(mode);
                self
            }

            /// Visually hides the label while preserving its semantic text
            /// for assistive technology.
            ///
            /// Shortcut for [`Self::label_style`] with
            /// [`LabelDisplayMode::Hidden`](crate::label::LabelDisplayMode::Hidden).
            #[must_use]
            pub const fn hide_label(self) -> Self {
                self.label_style($crate::label::LabelDisplayMode::Hidden)
            }
        }
    };
}

pub use impl_label_style_methods;

#[derive(Debug, Clone)]
struct LabelIcon {
    view: AnyViewBuilder<AnyView>,
    system_icon: Option<SystemIcon>,
}

impl LabelIcon {
    fn custom(icon: impl View + Clone) -> Self {
        let view = AnyViewBuilder::new(move || AnyView::new(icon.clone()));
        Self {
            view,
            system_icon: None,
        }
    }

    fn system(icon: SystemIcon) -> Self {
        let system_icon = icon.clone();
        let view = AnyViewBuilder::new(move || AnyView::new(icon.clone()));
        Self {
            view,
            system_icon: Some(system_icon),
        }
    }
}

#[derive(Debug, Clone)]
enum LabelContent {
    Semantic {
        text: Text,
        icon: Option<LabelIcon>,
        icon_position: IconPosition,
        spacing: Computed<f32>,
        font: Option<Font>,
    },
    Custom {
        semantic_text: Text,
        view: AnyViewBuilder<AnyView>,
    },
}

/// Semantic label for controls, commands, and chrome.
#[derive(Debug, Clone)]
#[expect(
    clippy::struct_field_names,
    reason = "the `accessibility_` prefix groups the accessibility-override fields"
)]
pub struct Label {
    content: LabelContent,
    display_mode: LabelDisplayMode,
    accessibility_text: Option<Text>,
    accessibility_label: Option<Computed<StyledStr>>,
}

/// Converts semantic text inputs into a [`Label`].
///
/// String, [`Text`], [`StyledStr`], [`Binding`], and [`Computed`] inputs use
/// `WaterUI`'s i18n-aware semantic text pipeline. Use [`Label::new`] only when
/// the visible label is an arbitrary view that differs from its spoken text.
pub trait IntoLabel {
    /// Converts a value into a semantic label.
    fn into_label(self) -> Label;
}

impl IntoLabel for Label {
    fn into_label(self) -> Label {
        self
    }
}

impl IntoLabel for Text {
    fn into_label(self) -> Label {
        Label::semantic(self)
    }
}

impl IntoLabel for &'static str {
    fn into_label(self) -> Label {
        Label::semantic(self)
    }
}

impl IntoLabel for alloc::string::String {
    fn into_label(self) -> Label {
        Label::semantic(self)
    }
}

impl IntoLabel for waterui_core::Str {
    fn into_label(self) -> Label {
        Label::semantic(self)
    }
}

impl IntoLabel for StyledStr {
    fn into_label(self) -> Label {
        Label::semantic(self)
    }
}

impl<T> IntoLabel for Computed<T>
where
    T: IntoText + Clone + 'static,
{
    fn into_label(self) -> Label {
        Label::semantic(self)
    }
}

impl<T> IntoLabel for Binding<T>
where
    T: IntoText + Clone + 'static,
{
    fn into_label(self) -> Label {
        Label::semantic(self)
    }
}

impl Label {
    /// Creates a label with arbitrary visual content and separate semantic text.
    ///
    /// `semantic_text` is exposed to controls and assistive technology, while
    /// `content` is the view rendered on screen. For ordinary text labels, use
    /// [`label`] or pass an [`IntoLabel`] value directly to the control.
    ///
    /// `content` is a builder rather than a value, because the control may
    /// realize the label more than once — a list row rebuilds it whenever it
    /// scrolls back into view. Any closure returning a view is a builder, which
    /// is also what lets modifier chains (`.padding()`, `.clip(…)`, …) be used
    /// here at all: those wrappers are not `Clone`.
    ///
    /// ```rust,ignore
    /// let verified = Label::new(
    ///     "Verified account",
    ///     || hstack((text("Account"), verification_badge())),
    /// );
    /// ```
    #[must_use]
    pub fn new(semantic_text: impl IntoText, content: impl ViewBuilder) -> Self {
        Self {
            content: LabelContent::Custom {
                semantic_text: semantic_text.into_text(),
                view: AnyViewBuilder::new(move || AnyView::new(content.build())),
            },
            display_mode: LabelDisplayMode::Automatic,
            accessibility_text: None,
            accessibility_label: None,
        }
    }

    fn semantic(text: impl IntoText) -> Self {
        Self {
            content: LabelContent::Semantic {
                text: text.into_text(),
                icon: None,
                icon_position: IconPosition::Leading,
                spacing: Computed::constant(6.0),
                font: None,
            },
            display_mode: LabelDisplayMode::Automatic,
            accessibility_text: None,
            accessibility_label: None,
        }
    }

    /// Replaces the text payload.
    #[must_use]
    pub fn text(mut self, text: impl IntoText) -> Self {
        let text = text.into_text();
        match &mut self.content {
            LabelContent::Semantic {
                text: semantic_text,
                ..
            }
            | LabelContent::Custom { semantic_text, .. } => *semantic_text = text,
        }
        self.accessibility_label = None;
        self
    }

    /// Overrides the spoken accessibility text without changing the visual text.
    pub(crate) fn accessibility_text(mut self, text: impl IntoText) -> Self {
        self.accessibility_text = Some(text.into_text());
        self.accessibility_label = None;
        self
    }

    /// Adds an icon view to the label.
    ///
    /// The icon is treated as visual chrome while the label text remains the
    /// accessibility name announced by assistive technology.
    ///
    /// If the icon is a direct [`SystemIcon`], native menus can reuse its
    /// semantic representation. Other icon views remain fully supported for
    /// ordinary UI rendering and hydrolysis popup menus, but native semantic
    /// menus currently only project [`SystemIcon`].
    ///
    /// # Panics
    ///
    /// Panics for [`Label::new`] labels with arbitrary visual content.
    /// `icon` is a value rather than a builder so a [`SystemIcon`] stays
    /// recognizable: the semantic identity of an SF Symbol is what lets Apple
    /// chrome render it natively, and a closure would erase it. Size an icon by
    /// choosing the icon's own size, or wrap the whole label with
    /// [`Label::new`], which does take a builder.
    #[must_use]
    pub fn icon(mut self, icon: impl View + Clone) -> Self {
        let system_icon = (&icon as &dyn Any).downcast_ref::<SystemIcon>().cloned();
        let icon = system_icon.map_or_else(|| LabelIcon::custom(icon), LabelIcon::system);
        match &mut self.content {
            LabelContent::Semantic {
                icon: semantic_icon,
                ..
            } => *semantic_icon = Some(icon),
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::icon requires a semantic text label; compose icons inside Label::new content"
                )
            }
        }
        self
    }

    /// Adds a semantic system icon to the label.
    ///
    /// `SystemIcon` is platform-asymmetric: it renders SF Symbols on Apple
    /// platforms (iOS / iPadOS / macOS / tvOS / visionOS) and is
    /// **intentionally unsupported on Android, Linux, Web**. For portable,
    /// cross-platform icons, prefer [`Label::icon`] with an icon-pack crate
    /// such as `waterui-icons-lucide`, `waterui-icons-material-icon`, or
    /// `waterui-icons-fontawesome7`. See [`SystemIcon`] for the full
    /// rationale.
    ///
    /// # Panics
    ///
    /// Panics for [`Label::new`] labels with arbitrary visual content.
    #[must_use]
    pub fn system_icon(mut self, icon: SystemIcon) -> Self {
        match &mut self.content {
            LabelContent::Semantic {
                icon: semantic_icon,
                ..
            } => *semantic_icon = Some(LabelIcon::system(icon)),
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::system_icon requires a semantic text label; compose icons inside Label::new content"
                )
            }
        }
        self
    }

    /// Places the icon on the trailing side.
    ///
    /// # Panics
    ///
    /// Panics for [`Label::new`] labels with arbitrary visual content.
    #[must_use]
    pub fn trailing(mut self) -> Self {
        match &mut self.content {
            LabelContent::Semantic { icon_position, .. } => *icon_position = IconPosition::Trailing,
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::trailing requires a semantic text label; arrange Label::new content directly"
                )
            }
        }
        self
    }

    /// Places the icon on the leading side.
    ///
    /// # Panics
    ///
    /// Panics for [`Label::new`] labels with arbitrary visual content.
    #[must_use]
    pub fn leading(mut self) -> Self {
        match &mut self.content {
            LabelContent::Semantic { icon_position, .. } => *icon_position = IconPosition::Leading,
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::leading requires a semantic text label; arrange Label::new content directly"
                )
            }
        }
        self
    }

    /// Sets the spacing between icon and text.
    ///
    /// Signal changes invalidate only the label stack's layout.
    ///
    /// # Panics
    ///
    /// Panics for [`Label::new`] labels with arbitrary visual content.
    #[must_use]
    pub fn spacing(mut self, spacing: impl IntoSignalF32 + 'static) -> Self {
        match &mut self.content {
            LabelContent::Semantic {
                spacing: label_spacing,
                ..
            } => *label_spacing = spacing.into_signal_f32().computed(),
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::spacing requires a semantic text label; space Label::new content directly"
                )
            }
        }
        self
    }

    /// Sets the visual font used by the label text.
    ///
    /// This affects only the on-screen label text. The semantic text exposed to
    /// controls and assistive technology remains unchanged.
    ///
    /// # Panics
    ///
    /// Panics for [`Label::new`] labels because arbitrary content owns its own
    /// visual styling.
    #[must_use]
    pub fn font(mut self, font: impl Into<Font>) -> Self {
        match &mut self.content {
            LabelContent::Semantic {
                font: label_font, ..
            } => *label_font = Some(font.into()),
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::font requires a semantic text label; style Label::new content directly"
                )
            }
        }
        self
    }

    /// Sets the preferred display mode for this label.
    #[must_use]
    pub const fn display_mode(mut self, mode: LabelDisplayMode) -> Self {
        self.display_mode = mode;
        self
    }

    /// Prefers showing both title and icon.
    #[must_use]
    pub const fn title_and_icon(self) -> Self {
        self.display_mode(LabelDisplayMode::TitleAndIcon)
    }

    /// Prefers showing only the title text.
    #[must_use]
    pub const fn title_only(self) -> Self {
        self.display_mode(LabelDisplayMode::TitleOnly)
    }

    /// Prefers showing only the icon when one is available.
    ///
    /// The semantic text is still preserved for accessibility and compact
    /// chrome such as toolbars.
    #[must_use]
    pub const fn icon_only(self) -> Self {
        self.display_mode(LabelDisplayMode::IconOnly)
    }

    /// Visually hides the label.
    ///
    /// The rendered view collapses to zero size, but the semantic text is
    /// preserved for assistive technology. Equivalent to
    /// `.display_mode(LabelDisplayMode::Hidden)`.
    #[must_use]
    pub const fn hide_label(self) -> Self {
        self.display_mode(LabelDisplayMode::Hidden)
    }

    fn effective_display_mode(&self, env: &Environment) -> LabelDisplayMode {
        let requested = if matches!(self.display_mode, LabelDisplayMode::Automatic) {
            env.get::<LabelDisplayMode>()
                .copied()
                .unwrap_or(LabelDisplayMode::Automatic)
        } else {
            self.display_mode
        };

        match requested {
            LabelDisplayMode::Hidden => LabelDisplayMode::Hidden,
            LabelDisplayMode::Automatic | LabelDisplayMode::TitleAndIcon if self.has_icon() => {
                LabelDisplayMode::TitleAndIcon
            }
            LabelDisplayMode::IconOnly if self.has_icon() => LabelDisplayMode::IconOnly,
            _ => LabelDisplayMode::TitleOnly,
        }
    }

    /// Whether this label renders caller-supplied views rather than its own
    /// text and icon.
    ///
    /// A backend that has a fast path for text-only labels must check this
    /// rather than reading [`Self::display_mode_preference`]: resolving a
    /// custom-content label against the environment reports
    /// [`LabelDisplayMode::TitleOnly`] too, since it has no icon, and taking
    /// the text path for one drops the content it was built to show.
    #[must_use]
    pub const fn has_custom_content(&self) -> bool {
        matches!(self.content, LabelContent::Custom { .. })
    }

    /// Returns the semantic text carried by this label.
    #[must_use = "this borrows the label's text without consuming the label"]
    pub const fn semantic_text(&self) -> &Text {
        match &self.content {
            LabelContent::Semantic { text, .. } => text,
            LabelContent::Custom { semantic_text, .. } => semantic_text,
        }
    }

    /// Returns the resolved accessibility label carried by this label.
    ///
    /// # Panics
    ///
    /// Panics when called before an environment-dependent label has been
    /// resolved in a component `body(env)` path.
    #[must_use]
    pub fn accessibility_label(&self) -> Computed<StyledStr> {
        self.accessibility_label
            .clone()
            .unwrap_or_else(|| self.semantic_text().content())
    }

    /// Returns the semantic system icon carried by this label.
    ///
    /// Custom icon views are visual-only and therefore return `None` here.
    #[must_use]
    pub fn semantic_icon(&self) -> Option<SystemIcon> {
        match &self.content {
            LabelContent::Semantic { icon, .. } => {
                icon.as_ref().and_then(|icon| icon.system_icon.clone())
            }
            LabelContent::Custom { .. } => None,
        }
    }

    /// Returns the label's icon as a view builder, whatever kind it is.
    ///
    /// Chrome that hosts an icon apart from its title — a tab bar item is an
    /// image plus a title, not one composed view — needs the icon on its own.
    /// Ask [`Self::semantic_icon`] first: a platform that recognizes the symbol
    /// draws it natively at any size, while this is a view the host has to
    /// render for itself.
    #[must_use]
    pub fn icon_view(&self) -> Option<AnyViewBuilder<AnyView>> {
        match &self.content {
            LabelContent::Semantic { icon, .. } => icon.as_ref().map(|icon| icon.view.clone()),
            LabelContent::Custom { .. } => None,
        }
    }

    const fn has_icon(&self) -> bool {
        match &self.content {
            LabelContent::Semantic { icon, .. } => icon.is_some(),
            LabelContent::Custom { .. } => false,
        }
    }

    /// Caps the visible semantic text at `limit` lines unless the author
    /// already set an explicit limit.
    ///
    /// Controls whose platform baseline keeps labels on one line — a `SwiftUI`
    /// or Material button truncates rather than folding its label into a
    /// paragraph — apply their default through this, while an explicit
    /// [`Text::line_limit`] from the author stays authoritative. Custom-view
    /// labels are untouched: an arbitrary composition owns its own wrapping.
    #[must_use]
    pub fn default_text_line_limit(mut self, limit: core::num::NonZeroUsize) -> Self {
        if let LabelContent::Semantic { text, .. } = &mut self.content {
            *text = text.clone().default_line_limit(limit);
        }
        self
    }

    /// Resolves environment-dependent label presentation before crossing into
    /// a native backend payload.
    #[must_use]
    pub fn resolve(mut self, env: &Environment) -> Self {
        self.display_mode = self.effective_display_mode(env);
        let accessibility_text = self
            .accessibility_text
            .as_ref()
            .unwrap_or_else(|| self.semantic_text());
        self.accessibility_label = Some(accessibility_text.resolve(env).content);
        self
    }
}

impl View for Label {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let mode = self.effective_display_mode(env);
        if matches!(mode, LabelDisplayMode::Automatic) {
            panic!("Label::effective_display_mode must resolve Automatic before rendering");
        }

        let Self { content, .. } = self;
        match content {
            LabelContent::Semantic {
                text,
                icon,
                icon_position,
                spacing,
                font,
            } => {
                let text = if let Some(font) = font {
                    text.font(font)
                } else {
                    text
                };

                match mode {
                    LabelDisplayMode::TitleOnly => AnyView::new(text),
                    LabelDisplayMode::IconOnly => AnyView::new(
                        icon.expect("LabelDisplayMode::IconOnly requires an icon when rendered")
                            .view
                            .build(),
                    ),
                    LabelDisplayMode::TitleAndIcon => {
                        let icon = icon
                            .expect("LabelDisplayMode::TitleAndIcon requires an icon when rendered")
                            .view
                            .build();
                        match icon_position {
                            IconPosition::Leading => {
                                AnyView::new(hstack((icon, text)).spacing(spacing))
                            }
                            IconPosition::Trailing => {
                                AnyView::new(hstack((text, icon)).spacing(spacing))
                            }
                        }
                    }
                    LabelDisplayMode::Hidden => AnyView::new(()),
                    LabelDisplayMode::Automatic => {
                        panic!(
                            "Label::effective_display_mode must resolve Automatic before rendering"
                        );
                    }
                }
            }
            LabelContent::Custom { view, .. } => match mode {
                LabelDisplayMode::TitleOnly | LabelDisplayMode::TitleAndIcon => view.build(),
                LabelDisplayMode::Hidden | LabelDisplayMode::IconOnly => AnyView::new(()),
                LabelDisplayMode::Automatic => {
                    panic!("Label::effective_display_mode must resolve Automatic before rendering");
                }
            },
        }
    }
}

/// Creates an i18n-aware semantic text label.
///
/// This is the ergonomic counterpart to [`Label::new`], which accepts
/// arbitrary visual content plus separate semantic text.
#[must_use]
pub fn label(text: impl IntoText) -> Label {
    Label::semantic(text)
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use nami::{Computed, Signal};
    use waterui_core::Environment;
    use waterui_locale::{TranslationCatalog, locales};

    use super::IntoLabel;

    #[test]
    fn computed_string_value_converts_into_label_text() {
        let env = test_env();
        let label = Computed::constant(String::from("Ready")).into_label();
        assert_eq!(semantic_plain(&label, &env), "Ready");
    }

    #[test]
    fn computed_static_str_resolves_through_i18n_catalog() {
        let env = test_env();
        let label = Computed::constant("greeting").into_label();

        assert_eq!(semantic_plain(&label, &env), "Hello");
    }

    #[test]
    fn hide_label_keeps_semantic_text_for_accessibility() {
        let env = test_env();
        let label = super::label("greeting").hide_label();

        // The semantic text remains intact for assistive technology even when
        // the label is configured to render no visible chrome.
        assert_eq!(semantic_plain(&label, &env), "Hello");
    }

    #[test]
    fn arbitrary_content_label_keeps_separate_semantic_text() {
        let env = test_env();
        let label = super::Label::new("greeting", || waterui_text::text("Visual"));

        assert_eq!(semantic_plain(&label, &env), "Hello");
    }

    #[test]
    fn hidden_mode_overrides_implicit_title_only_fallback() {
        // Without an icon, Automatic / TitleAndIcon would fall back to TitleOnly.
        // Hidden must take precedence over those defaults.
        let env = test_env();
        let label = super::label("greeting").hide_label();

        let mode = label.effective_display_mode(&env);
        assert!(matches!(mode, super::LabelDisplayMode::Hidden));
    }

    fn test_env() -> Environment {
        let mut env = Environment::new();
        env.insert(locales::EN);
        env.insert(
            TranslationCatalog::new()
                .add_toml("en", "greeting = \"Hello\"")
                .expect("test catalog must parse"),
        );
        env
    }

    fn semantic_plain(label: &super::Label, env: &Environment) -> String {
        label
            .semantic_text()
            .resolve(env)
            .content
            .get()
            .to_plain()
            .into_string()
    }

    #[test]
    fn font_keeps_semantic_text_unchanged() {
        let env = test_env();
        let label = super::label("greeting").font(waterui_text::font::Body);

        assert_eq!(semantic_plain(&label, &env), "Hello");
    }
}
