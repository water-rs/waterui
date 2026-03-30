//! Semantic labels shared by controls, menus, and chrome.

use core::any::Any;
use nami::Computed;
use waterui_core::{AnyView, Environment, View, handler::AnyViewBuilder, plugin::Plugin};
use waterui_icon::SystemIcon;
use waterui_layout::stack::hstack;
use waterui_text::{IntoText, Text, styled::StyledStr};

/// Position of the icon relative to the text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IconPosition {
    /// Icon appears before the text (left in LTR).
    #[default]
    Leading,
    /// Icon appears after the text (right in LTR).
    Trailing,
}

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
///     button(label("Search").system_icon(SystemIcon::SEARCH)).action(|| {}),
///     button(label("Settings").system_icon(SystemIcon::SETTINGS)).action(|| {}),
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
}

impl Plugin for LabelDisplayMode {}

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

/// Semantic label for controls, commands, and chrome.
#[derive(Debug, Clone)]
pub struct Label {
    text: Text,
    icon: Option<LabelIcon>,
    icon_position: IconPosition,
    spacing: f32,
    display_mode: LabelDisplayMode,
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

impl Label {
    /// Creates a new label with the specified text.
    #[must_use]
    pub fn new(text: impl IntoText) -> Self {
        Self {
            text: text.into_text(),
            icon: None,
            icon_position: IconPosition::Leading,
            spacing: 6.0,
            display_mode: LabelDisplayMode::Automatic,
        }
    }

    /// Replaces the text payload.
    #[must_use]
    pub fn text(mut self, text: impl IntoText) -> Self {
        self.text = text.into_text();
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
        let icon = if let Some(system_icon) = (&icon as &dyn Any).downcast_ref::<SystemIcon>() {
            LabelIcon::system(system_icon.clone())
        } else {
            LabelIcon::custom(icon)
        };
        self.icon = Some(icon);
        self
    }

    /// Adds a semantic system icon to the label.
    #[must_use]
    pub fn system_icon(mut self, icon: SystemIcon) -> Self {
        self.icon = Some(LabelIcon::system(icon));
        self
    }

    /// Places the icon on the trailing side.
    #[must_use]
    pub const fn trailing(mut self) -> Self {
        self.icon_position = IconPosition::Trailing;
        self
    }

    /// Places the icon on the leading side.
    #[must_use]
    pub const fn leading(mut self) -> Self {
        self.icon_position = IconPosition::Leading;
        self
    }

    /// Sets the spacing between icon and text.
    #[must_use]
    pub const fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
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

    fn effective_display_mode(&self, env: &Environment) -> LabelDisplayMode {
        let requested = if matches!(self.display_mode, LabelDisplayMode::Automatic) {
            env.get::<LabelDisplayMode>()
                .copied()
                .unwrap_or(LabelDisplayMode::Automatic)
        } else {
            self.display_mode
        };

        match requested {
            LabelDisplayMode::Automatic | LabelDisplayMode::TitleAndIcon if self.icon.is_some() => {
                LabelDisplayMode::TitleAndIcon
            }
            LabelDisplayMode::IconOnly if self.icon.is_some() => LabelDisplayMode::IconOnly,
            _ => LabelDisplayMode::TitleOnly,
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __text(&self) -> &Text {
        &self.text
    }

    #[doc(hidden)]
    #[must_use]
    pub fn __icon(&self) -> Option<SystemIcon> {
        self.icon.as_ref().and_then(|icon| icon.system_icon.clone())
    }
}

impl View for Label {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let mode = self.effective_display_mode(env);
        match mode {
            LabelDisplayMode::TitleOnly => AnyView::new(self.text),
            LabelDisplayMode::IconOnly => AnyView::new(
                self.icon
                    .expect("LabelDisplayMode::IconOnly requires an icon when rendered")
                    .view
                    .build(),
            ),
            LabelDisplayMode::Automatic => unreachable!(
                "Label::effective_display_mode must resolve Automatic before rendering"
            ),
            LabelDisplayMode::TitleAndIcon => {
                let icon = self
                    .icon
                    .expect("LabelDisplayMode::TitleAndIcon requires an icon when rendered")
                    .view
                    .build();
                match self.icon_position {
                    IconPosition::Leading => {
                        AnyView::new(hstack((icon, self.text)).spacing(self.spacing))
                    }
                    IconPosition::Trailing => {
                        AnyView::new(hstack((self.text, icon)).spacing(self.spacing))
                    }
                }
            }
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
        let content = label.__text().resolve(&env).content.get();

        assert_eq!(content.to_plain(), "Ready");
    }

    #[test]
    fn computed_static_str_resolves_through_i18n_catalog() {
        let env = test_env();
        let label = Computed::constant("greeting").into_label();

        assert_eq!(
            label.__text().resolve(&env).content.get().to_plain(),
            "Hello"
        );
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
}

/// Convenience function to create a label.
#[must_use]
pub fn label(text: impl IntoText) -> Label {
    Label::new(text)
}
