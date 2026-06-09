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
use nami::{Binding, Computed};
use waterui_core::{AnyView, Environment, View, handler::AnyViewBuilder, plugin::Plugin};
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
        Self::new(Text::default())
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
        spacing: f32,
    },
    Custom {
        semantic_text: Text,
        view: AnyViewBuilder<AnyView>,
    },
}

/// Semantic label for controls, commands, and chrome.
#[derive(Debug, Clone)]
pub struct Label {
    content: LabelContent,
    display_mode: LabelDisplayMode,
    font: Option<Font>,
    accessibility_text: Option<Text>,
    accessibility_label: Option<Computed<StyledStr>>,
}

/// Conversion trait for semantic labels.
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
        Label::new(self)
    }
}

impl IntoLabel for &'static str {
    fn into_label(self) -> Label {
        Label::new(self)
    }
}

impl IntoLabel for alloc::string::String {
    fn into_label(self) -> Label {
        Label::new(self)
    }
}

impl IntoLabel for waterui_core::Str {
    fn into_label(self) -> Label {
        Label::new(self)
    }
}

impl IntoLabel for StyledStr {
    fn into_label(self) -> Label {
        Label::new(self)
    }
}

impl<T> IntoLabel for Computed<T>
where
    T: IntoText + Clone + 'static,
{
    fn into_label(self) -> Label {
        Label::new(self)
    }
}

impl<T> IntoLabel for Binding<T>
where
    T: IntoText + Clone + 'static,
{
    fn into_label(self) -> Label {
        Label::new(self)
    }
}

impl Label {
    /// Creates a new label with the specified text.
    #[must_use]
    pub fn new(text: impl IntoText) -> Self {
        Self {
            content: LabelContent::Semantic {
                text: text.into_text(),
                icon: None,
                icon_position: IconPosition::Leading,
                spacing: 6.0,
            },
            display_mode: LabelDisplayMode::Automatic,
            font: None,
            accessibility_text: None,
            accessibility_label: None,
        }
    }

    /// Creates a label with arbitrary visual content and separate semantic text.
    #[must_use]
    pub fn custom(semantic_text: impl IntoText, content: impl View + Clone) -> Self {
        Self {
            content: LabelContent::Custom {
                semantic_text: semantic_text.into_text(),
                view: AnyViewBuilder::new(move || AnyView::new(content.clone())),
            },
            display_mode: LabelDisplayMode::Automatic,
            font: None,
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
                    "Label::icon requires a semantic text label; attach icons inside Label::custom content"
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
    #[must_use]
    pub fn system_icon(mut self, icon: SystemIcon) -> Self {
        match &mut self.content {
            LabelContent::Semantic {
                icon: semantic_icon,
                ..
            } => *semantic_icon = Some(LabelIcon::system(icon)),
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::system_icon requires a semantic text label; attach icons inside Label::custom content"
                )
            }
        }
        self
    }

    /// Places the icon on the trailing side.
    #[must_use]
    pub fn trailing(mut self) -> Self {
        match &mut self.content {
            LabelContent::Semantic { icon_position, .. } => *icon_position = IconPosition::Trailing,
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::trailing requires a semantic text label; arrange custom label content directly"
                )
            }
        }
        self
    }

    /// Places the icon on the leading side.
    #[must_use]
    pub fn leading(mut self) -> Self {
        match &mut self.content {
            LabelContent::Semantic { icon_position, .. } => *icon_position = IconPosition::Leading,
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::leading requires a semantic text label; arrange custom label content directly"
                )
            }
        }
        self
    }

    /// Sets the spacing between icon and text.
    #[must_use]
    pub fn spacing(mut self, spacing: f32) -> Self {
        match &mut self.content {
            LabelContent::Semantic {
                spacing: label_spacing,
                ..
            } => *label_spacing = spacing,
            LabelContent::Custom { .. } => {
                panic!(
                    "Label::spacing requires a semantic text label; space custom label content directly"
                )
            }
        }
        self
    }

    /// Sets the visual font used by the label text.
    ///
    /// This affects only the on-screen label text. The semantic text exposed to
    /// controls and assistive technology remains unchanged.
    #[must_use]
    pub fn font(mut self, font: impl Into<Font>) -> Self {
        self.font = Some(font.into());
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

    /// Returns the semantic text carried by this label.
    pub fn semantic_text(&self) -> &Text {
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

    fn has_icon(&self) -> bool {
        match &self.content {
            LabelContent::Semantic { icon, .. } => icon.is_some(),
            LabelContent::Custom { .. } => false,
        }
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
        self.accessibility_label = Some(accessibility_text.resolve_reactive(env).content);
        self
    }
}

impl View for Label {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let mode = self.effective_display_mode(env);
        if matches!(mode, LabelDisplayMode::Automatic) {
            panic!("Label::effective_display_mode must resolve Automatic before rendering");
        }

        let Self {
            content,
            font,
            accessibility_text: _,
            accessibility_label: _,
            ..
        } = self;
        match content {
            LabelContent::Semantic {
                text,
                icon,
                icon_position,
                spacing,
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
        let content = label.semantic_text().resolve(&env).content.get();

        assert_eq!(content.to_plain(), "Ready");
    }

    #[test]
    fn computed_static_str_resolves_through_i18n_catalog() {
        let env = test_env();
        let label = Computed::constant("greeting").into_label();

        assert_eq!(
            label.semantic_text().resolve(&env).content.get().to_plain(),
            "Hello"
        );
    }

    #[test]
    fn hide_label_keeps_semantic_text_for_accessibility() {
        let env = test_env();
        let label = super::Label::new("greeting").hide_label();

        // The semantic text remains intact for assistive technology even when
        // the label is configured to render no visible chrome.
        assert_eq!(
            label.semantic_text().resolve(&env).content.get().to_plain(),
            "Hello"
        );
    }

    #[test]
    fn custom_label_keeps_separate_semantic_text() {
        let env = test_env();
        let label = super::Label::custom("greeting", waterui_text::text("Visual"));

        assert_eq!(
            label.semantic_text().resolve(&env).content.get().to_plain(),
            "Hello"
        );
    }

    #[test]
    fn hidden_mode_overrides_implicit_title_only_fallback() {
        // Without an icon, Automatic / TitleAndIcon would fall back to TitleOnly.
        // Hidden must take precedence over those defaults.
        let env = test_env();
        let label = super::Label::new("greeting").hide_label();

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

    #[test]
    fn font_keeps_semantic_text_unchanged() {
        let env = test_env();
        let label = super::Label::new("greeting").font(waterui_text::font::Body);

        assert_eq!(
            label.semantic_text().resolve(&env).content.get().to_plain(),
            "Hello"
        );
    }
}

/// Convenience function to create a label.
#[must_use]
pub fn label(text: impl IntoText) -> Label {
    Label::new(text)
}
