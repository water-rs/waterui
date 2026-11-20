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

use alloc::rc::Rc;
use core::{any::Any, fmt};
use nami::{Binding, Computed, SignalExt};
use waterui_core::{
    AnyView, Environment, IntoSignalF32, View,
    backend::{Backend, ChildSlot},
    contract_violation,
    handler::{AnyViewBuilder, ViewBuilder},
    plugin::Plugin,
};
use waterui_icon::SystemIcon;
use waterui_layout::stack::hstack;
use waterui_text::{IntoText, Text, font::Font, styled::StyledStr};

/// Position of the icon relative to the text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: an icon sits before or after its text"
)]
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
/// ```rust
/// # use waterui::prelude::*;
/// # use waterui_controls::{LabelDisplayMode, button, label};
/// # use waterui_icon::system_icon;
/// # fn toolbar() -> impl View {
/// hstack((
///     button(label("Search").system_icon(system_icon::search())).action(|| {}),
///     button(label("Settings").system_icon(system_icon::settings())).action(|| {}),
/// ))
/// .install(LabelDisplayMode::IconOnly)
/// # }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
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

fn resolve_display_mode(
    preferred: LabelDisplayMode,
    has_icon: bool,
    env: &Environment,
) -> LabelDisplayMode {
    let requested = if matches!(preferred, LabelDisplayMode::Automatic) {
        env.get::<LabelDisplayMode>()
            .copied()
            .unwrap_or(LabelDisplayMode::Automatic)
    } else {
        preferred
    };

    match requested {
        LabelDisplayMode::Hidden => LabelDisplayMode::Hidden,
        LabelDisplayMode::Automatic | LabelDisplayMode::TitleAndIcon if has_icon => {
            LabelDisplayMode::TitleAndIcon
        }
        LabelDisplayMode::IconOnly if has_icon => LabelDisplayMode::IconOnly,
        _ => LabelDisplayMode::TitleOnly,
    }
}

/// View produced by rebuilding a [`ViewBuilder`] each time a [`Label`] is realized.
pub struct Built<B>(Rc<B>);

impl<B> Built<B> {
    /// Wraps `builder` for repeated realization.
    #[must_use]
    pub fn new(builder: B) -> Self {
        Self(Rc::new(builder))
    }
}

impl<B> Clone for Built<B> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<B> fmt::Debug for Built<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Built")
    }
}

impl<B: ViewBuilder> View for Built<B> {
    fn body(self, _env: &Environment) -> impl View {
        self.0.build()
    }

    fn dispatch<Bk: Backend>(self, backend: &mut Bk, env: &Environment, cx: Bk::Ctx) -> Bk::Output {
        self.0.build().dispatch(backend, env, cx)
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Content representation of a [`Label`], parameterized so unused label forms
/// (icons, stacks, custom builders, erased slots) are unreachable when not constructed.
pub trait LabelKind: sealed::Sealed + Clone + 'static {
    /// Resulting kind when an icon of type `I` is attached.
    type WithIcon<I: View + Clone>: LabelKind;

    /// Borrows the semantic text.
    fn semantic_text(&self) -> &Text;
    /// Replaces the semantic text.
    fn set_semantic_text(&mut self, text: Text);
    /// Whether this label carries caller-supplied arbitrary visual content.
    fn has_custom_content(&self) -> bool;
    /// Whether this label carries an icon.
    fn has_icon(&self) -> bool;
    /// Semantic [`SystemIcon`], if one was attached.
    fn semantic_icon(&self) -> Option<SystemIcon>;
    /// Lifts the icon into an erased builder for chrome that places icons separately.
    fn icon_view(&self) -> Option<AnyViewBuilder<AnyView>>;
    /// Attaches `icon` (`system_icon` is `Some` when `I` is [`SystemIcon`]).
    fn with_icon<I: View + Clone>(
        self,
        icon: I,
        system_icon: Option<SystemIcon>,
    ) -> Self::WithIcon<I>;
    /// Sets icon placement (`Leading` or `Trailing`), or reports a contract violation on custom content.
    fn set_icon_position(&mut self, position: IconPosition, method: &'static str);
    /// Sets spacing between icon and text, or reports a contract violation on custom content.
    fn set_spacing(&mut self, spacing: Computed<f32>);
    /// Sets the text font, or reports a contract violation on custom content.
    fn set_font(&mut self, font: Font);
    /// Caps semantic text line count unless already set.
    fn set_default_text_line_limit(&mut self, limit: core::num::NonZeroUsize);
    /// Strips any visual icon/custom view, keeping semantic text and [`SystemIcon`].
    fn into_semantic(self) -> SemanticLabel;
    /// Erases any visual icon/custom view for runtime-open heterogeneous collections.
    fn into_erased(self) -> ErasedLabel;
    /// Builds the slot factory for [`ControlLabel`].
    fn into_slot_builder<S: ChildSlot>(
        self,
        mode: LabelDisplayMode,
    ) -> Rc<dyn Fn(Option<Font>) -> S>;
    /// Dispatches the resolved visual presentation through `backend`.
    fn dispatch<B: Backend>(
        self,
        mode: LabelDisplayMode,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output;
    /// Expands to an erased view for callers invoking `View::body` directly.
    fn body_erased(self, mode: LabelDisplayMode) -> AnyView;
}

/// Plain semantic text label content (no icon, no stack, no custom view).
#[derive(Debug, Clone)]
pub struct TitleLabel {
    text: Text,
    icon_position: IconPosition,
    spacing: Computed<f32>,
    font: Option<Font>,
}

impl sealed::Sealed for TitleLabel {}

impl LabelKind for TitleLabel {
    type WithIcon<I: View + Clone> = IconLabel<I>;

    fn semantic_text(&self) -> &Text {
        &self.text
    }

    fn set_semantic_text(&mut self, text: Text) {
        self.text = text;
    }

    fn has_custom_content(&self) -> bool {
        false
    }

    fn has_icon(&self) -> bool {
        false
    }

    fn semantic_icon(&self) -> Option<SystemIcon> {
        None
    }

    fn icon_view(&self) -> Option<AnyViewBuilder<AnyView>> {
        None
    }

    fn with_icon<I: View + Clone>(self, icon: I, system_icon: Option<SystemIcon>) -> IconLabel<I> {
        IconLabel {
            text: self.text,
            icon,
            system_icon,
            icon_position: self.icon_position,
            spacing: self.spacing,
            font: self.font,
        }
    }

    fn set_icon_position(&mut self, position: IconPosition, _method: &'static str) {
        self.icon_position = position;
    }

    fn set_spacing(&mut self, spacing: Computed<f32>) {
        self.spacing = spacing;
    }

    fn set_font(&mut self, font: Font) {
        self.font = Some(font);
    }

    fn set_default_text_line_limit(&mut self, limit: core::num::NonZeroUsize) {
        self.text = self.text.clone().default_line_limit(limit);
    }

    fn into_semantic(self) -> SemanticLabel {
        SemanticLabel {
            text: self.text,
            system_icon: None,
            icon_position: self.icon_position,
            spacing: self.spacing,
            font: self.font,
        }
    }

    fn into_erased(self) -> ErasedLabel {
        ErasedLabel::Semantic {
            text: self.text,
            icon: None,
            system_icon: None,
            icon_position: self.icon_position,
            spacing: self.spacing,
            font: self.font,
        }
    }

    fn into_slot_builder<S: ChildSlot>(
        self,
        mode: LabelDisplayMode,
    ) -> Rc<dyn Fn(Option<Font>) -> S> {
        let Self { text, font, .. } = self;
        Rc::new(move |default_font| {
            if mode == LabelDisplayMode::Hidden {
                S::from_view(())
            } else {
                let text = font
                    .clone()
                    .or(default_font)
                    .map_or_else(|| text.clone(), |f| text.clone().font(f));
                S::from_view(text)
            }
        })
    }

    fn dispatch<B: Backend>(
        self,
        mode: LabelDisplayMode,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        if mode == LabelDisplayMode::Hidden {
            ().dispatch(backend, env, cx)
        } else {
            let text = match self.font {
                Some(font) => self.text.font(font),
                None => self.text,
            };
            text.dispatch(backend, env, cx)
        }
    }

    fn body_erased(self, mode: LabelDisplayMode) -> AnyView {
        if mode == LabelDisplayMode::Hidden {
            AnyView::new(())
        } else {
            let text = match self.font {
                Some(font) => self.text.font(font),
                None => self.text,
            };
            AnyView::new(text)
        }
    }
}

/// Semantic text label paired with a concrete icon view `I`.
#[derive(Debug, Clone)]
pub struct IconLabel<I> {
    text: Text,
    icon: I,
    system_icon: Option<SystemIcon>,
    icon_position: IconPosition,
    spacing: Computed<f32>,
    font: Option<Font>,
}

impl<I> sealed::Sealed for IconLabel<I> {}

impl<I: View + Clone> LabelKind for IconLabel<I> {
    type WithIcon<NewI: View + Clone> = IconLabel<NewI>;

    fn semantic_text(&self) -> &Text {
        &self.text
    }

    fn set_semantic_text(&mut self, text: Text) {
        self.text = text;
    }

    fn has_custom_content(&self) -> bool {
        false
    }

    fn has_icon(&self) -> bool {
        true
    }

    fn semantic_icon(&self) -> Option<SystemIcon> {
        self.system_icon.clone()
    }

    fn icon_view(&self) -> Option<AnyViewBuilder<AnyView>> {
        let icon = self.icon.clone();
        Some(AnyViewBuilder::new(move || AnyView::new(icon.clone())))
    }

    fn with_icon<NewI: View + Clone>(
        self,
        icon: NewI,
        system_icon: Option<SystemIcon>,
    ) -> IconLabel<NewI> {
        IconLabel {
            text: self.text,
            icon,
            system_icon,
            icon_position: self.icon_position,
            spacing: self.spacing,
            font: self.font,
        }
    }

    fn set_icon_position(&mut self, position: IconPosition, _method: &'static str) {
        self.icon_position = position;
    }

    fn set_spacing(&mut self, spacing: Computed<f32>) {
        self.spacing = spacing;
    }

    fn set_font(&mut self, font: Font) {
        self.font = Some(font);
    }

    fn set_default_text_line_limit(&mut self, limit: core::num::NonZeroUsize) {
        self.text = self.text.clone().default_line_limit(limit);
    }

    fn into_semantic(self) -> SemanticLabel {
        SemanticLabel {
            text: self.text,
            system_icon: self.system_icon,
            icon_position: self.icon_position,
            spacing: self.spacing,
            font: self.font,
        }
    }

    fn into_erased(self) -> ErasedLabel {
        let icon = self.icon;
        ErasedLabel::Semantic {
            text: self.text,
            icon: Some(AnyViewBuilder::new(move || AnyView::new(icon.clone()))),
            system_icon: self.system_icon,
            icon_position: self.icon_position,
            spacing: self.spacing,
            font: self.font,
        }
    }

    fn into_slot_builder<S: ChildSlot>(
        self,
        mode: LabelDisplayMode,
    ) -> Rc<dyn Fn(Option<Font>) -> S> {
        let Self {
            text,
            icon,
            icon_position,
            spacing,
            font,
            ..
        } = self;
        Rc::new(move |default_font| {
            let styled_text = || {
                font.clone()
                    .or(default_font)
                    .map_or_else(|| text.clone(), |f| text.clone().font(f))
            };
            match mode {
                LabelDisplayMode::Hidden => S::from_view(()),
                LabelDisplayMode::TitleOnly => S::from_view(styled_text()),
                LabelDisplayMode::IconOnly => S::from_view(icon.clone()),
                LabelDisplayMode::TitleAndIcon | LabelDisplayMode::Automatic => match icon_position
                {
                    IconPosition::Leading => {
                        S::from_view(hstack((icon.clone(), styled_text())).spacing(spacing.clone()))
                    }
                    IconPosition::Trailing => {
                        S::from_view(hstack((styled_text(), icon.clone())).spacing(spacing.clone()))
                    }
                },
            }
        })
    }

    fn dispatch<B: Backend>(
        self,
        mode: LabelDisplayMode,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        let text = match self.font {
            Some(font) => self.text.font(font),
            None => self.text,
        };
        match mode {
            LabelDisplayMode::Hidden => ().dispatch(backend, env, cx),
            LabelDisplayMode::TitleOnly => text.dispatch(backend, env, cx),
            LabelDisplayMode::IconOnly => self.icon.dispatch(backend, env, cx),
            LabelDisplayMode::TitleAndIcon | LabelDisplayMode::Automatic => {
                match self.icon_position {
                    IconPosition::Leading => hstack((self.icon, text))
                        .spacing(self.spacing)
                        .dispatch(backend, env, cx),
                    IconPosition::Trailing => hstack((text, self.icon))
                        .spacing(self.spacing)
                        .dispatch(backend, env, cx),
                }
            }
        }
    }

    fn body_erased(self, mode: LabelDisplayMode) -> AnyView {
        let text = match self.font {
            Some(font) => self.text.font(font),
            None => self.text,
        };
        match mode {
            LabelDisplayMode::Hidden => AnyView::new(()),
            LabelDisplayMode::TitleOnly => AnyView::new(text),
            LabelDisplayMode::IconOnly => AnyView::new(self.icon),
            LabelDisplayMode::TitleAndIcon | LabelDisplayMode::Automatic => {
                match self.icon_position {
                    IconPosition::Leading => {
                        AnyView::new(hstack((self.icon, text)).spacing(self.spacing))
                    }
                    IconPosition::Trailing => {
                        AnyView::new(hstack((text, self.icon)).spacing(self.spacing))
                    }
                }
            }
        }
    }
}

/// Arbitrary visual content paired with separate semantic text ([`Label::new`]).
#[derive(Debug, Clone)]
pub struct CustomLabel<V> {
    semantic_text: Text,
    view: V,
}

impl<V> sealed::Sealed for CustomLabel<V> {}

impl<V: View + Clone> LabelKind for CustomLabel<V> {
    type WithIcon<I: View + Clone> = Self;

    fn semantic_text(&self) -> &Text {
        &self.semantic_text
    }

    fn set_semantic_text(&mut self, text: Text) {
        self.semantic_text = text;
    }

    fn has_custom_content(&self) -> bool {
        true
    }

    fn has_icon(&self) -> bool {
        false
    }

    fn semantic_icon(&self) -> Option<SystemIcon> {
        None
    }

    fn icon_view(&self) -> Option<AnyViewBuilder<AnyView>> {
        None
    }

    fn with_icon<I: View + Clone>(self, _icon: I, system_icon: Option<SystemIcon>) -> Self {
        if system_icon.is_some() {
            contract_violation!(
                "Label::system_icon requires a semantic text label; compose icons inside Label::new content; the call is ignored"
            );
        } else {
            contract_violation!(
                "Label::icon requires a semantic text label; compose icons inside Label::new content; the call is ignored"
            );
        }
        self
    }

    fn set_icon_position(&mut self, _position: IconPosition, method: &'static str) {
        contract_violation!(
            "Label::{method} requires a semantic text label; arrange Label::new content directly; the call is ignored"
        );
    }

    fn set_spacing(&mut self, _spacing: Computed<f32>) {
        contract_violation!(
            "Label::spacing requires a semantic text label; space Label::new content directly; the call is ignored"
        );
    }

    fn set_font(&mut self, _font: Font) {
        contract_violation!(
            "Label::font requires a semantic text label; style Label::new content directly; the call is ignored"
        );
    }

    fn set_default_text_line_limit(&mut self, _limit: core::num::NonZeroUsize) {}

    fn into_semantic(self) -> SemanticLabel {
        SemanticLabel {
            text: self.semantic_text,
            system_icon: None,
            icon_position: IconPosition::Leading,
            spacing: Computed::constant(8.0),
            font: None,
        }
    }

    fn into_erased(self) -> ErasedLabel {
        let view = self.view;
        ErasedLabel::Custom {
            semantic_text: self.semantic_text,
            view: AnyViewBuilder::new(move || AnyView::new(view.clone())),
        }
    }

    fn into_slot_builder<S: ChildSlot>(
        self,
        mode: LabelDisplayMode,
    ) -> Rc<dyn Fn(Option<Font>) -> S> {
        let view = self.view;
        Rc::new(move |_default_font| match mode {
            LabelDisplayMode::Hidden | LabelDisplayMode::IconOnly => S::from_view(()),
            _ => S::from_view(view.clone()),
        })
    }

    fn dispatch<B: Backend>(
        self,
        mode: LabelDisplayMode,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        match mode {
            LabelDisplayMode::Hidden | LabelDisplayMode::IconOnly => ().dispatch(backend, env, cx),
            _ => self.view.dispatch(backend, env, cx),
        }
    }

    fn body_erased(self, mode: LabelDisplayMode) -> AnyView {
        match mode {
            LabelDisplayMode::Hidden | LabelDisplayMode::IconOnly => AnyView::new(()),
            _ => AnyView::new(self.view),
        }
    }
}

/// Semantic-only label metadata retained by menu commands and submenus (never
/// dispatches [`SystemIcon`] as a view unless explicitly attached again).
#[derive(Debug, Clone)]
pub struct SemanticLabel {
    text: Text,
    system_icon: Option<SystemIcon>,
    icon_position: IconPosition,
    spacing: Computed<f32>,
    font: Option<Font>,
}

impl sealed::Sealed for SemanticLabel {}

impl LabelKind for SemanticLabel {
    type WithIcon<I: View + Clone> = IconLabel<I>;

    fn semantic_text(&self) -> &Text {
        &self.text
    }

    fn set_semantic_text(&mut self, text: Text) {
        self.text = text;
    }

    fn has_custom_content(&self) -> bool {
        false
    }

    fn has_icon(&self) -> bool {
        self.system_icon.is_some()
    }

    fn semantic_icon(&self) -> Option<SystemIcon> {
        self.system_icon.clone()
    }

    fn icon_view(&self) -> Option<AnyViewBuilder<AnyView>> {
        self.system_icon.as_ref().map(|sys| {
            let sys = sys.clone();
            AnyViewBuilder::new(move || AnyView::new(sys.clone()))
        })
    }

    fn with_icon<I: View + Clone>(self, icon: I, system_icon: Option<SystemIcon>) -> IconLabel<I> {
        IconLabel {
            text: self.text,
            icon,
            system_icon,
            icon_position: self.icon_position,
            spacing: self.spacing,
            font: self.font,
        }
    }

    fn set_icon_position(&mut self, position: IconPosition, _method: &'static str) {
        self.icon_position = position;
    }

    fn set_spacing(&mut self, spacing: Computed<f32>) {
        self.spacing = spacing;
    }

    fn set_font(&mut self, font: Font) {
        self.font = Some(font);
    }

    fn set_default_text_line_limit(&mut self, limit: core::num::NonZeroUsize) {
        self.text = self.text.clone().default_line_limit(limit);
    }

    fn into_semantic(self) -> Self {
        self
    }

    fn into_erased(self) -> ErasedLabel {
        let icon = self
            .system_icon
            .clone()
            .map(|sys| AnyViewBuilder::new(move || AnyView::new(sys.clone())));
        ErasedLabel::Semantic {
            text: self.text,
            icon,
            system_icon: self.system_icon,
            icon_position: self.icon_position,
            spacing: self.spacing,
            font: self.font,
        }
    }

    fn into_slot_builder<S: ChildSlot>(
        self,
        mode: LabelDisplayMode,
    ) -> Rc<dyn Fn(Option<Font>) -> S> {
        let Self { text, font, .. } = self;
        Rc::new(move |default_font| {
            if mode == LabelDisplayMode::Hidden {
                S::from_view(())
            } else {
                let text = font
                    .clone()
                    .or(default_font)
                    .map_or_else(|| text.clone(), |f| text.clone().font(f));
                S::from_view(text)
            }
        })
    }

    fn dispatch<B: Backend>(
        self,
        mode: LabelDisplayMode,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        if mode == LabelDisplayMode::Hidden {
            ().dispatch(backend, env, cx)
        } else {
            let text = match self.font {
                Some(font) => self.text.font(font),
                None => self.text,
            };
            text.dispatch(backend, env, cx)
        }
    }

    fn body_erased(self, mode: LabelDisplayMode) -> AnyView {
        if mode == LabelDisplayMode::Hidden {
            AnyView::new(())
        } else {
            let text = match self.font {
                Some(font) => self.text.font(font),
                None => self.text,
            };
            AnyView::new(text)
        }
    }
}

/// Explicitly erased label content for open runtime boundaries (such as JSX or
/// heterogeneous item collections). Unreachable unless [`Label::erase`] is called.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ErasedLabel {
    /// Erased semantic label.
    Semantic {
        /// Title text.
        text: Text,
        /// Optional erased icon builder.
        icon: Option<AnyViewBuilder<AnyView>>,
        /// Optional semantic system icon.
        system_icon: Option<SystemIcon>,
        /// Icon position.
        icon_position: IconPosition,
        /// Spacing between icon and text.
        spacing: Computed<f32>,
        /// Optional text font.
        font: Option<Font>,
    },
    /// Erased custom-content label.
    Custom {
        /// Spoken semantic text.
        semantic_text: Text,
        /// Erased custom view builder.
        view: AnyViewBuilder<AnyView>,
    },
}

impl sealed::Sealed for ErasedLabel {}

impl LabelKind for ErasedLabel {
    type WithIcon<I: View + Clone> = Self;

    fn semantic_text(&self) -> &Text {
        match self {
            Self::Semantic { text, .. } => text,
            Self::Custom { semantic_text, .. } => semantic_text,
        }
    }

    fn set_semantic_text(&mut self, new_text: Text) {
        match self {
            Self::Semantic { text, .. } => *text = new_text,
            Self::Custom { semantic_text, .. } => *semantic_text = new_text,
        }
    }

    fn has_custom_content(&self) -> bool {
        matches!(self, Self::Custom { .. })
    }

    fn has_icon(&self) -> bool {
        matches!(self, Self::Semantic { icon: Some(_), .. })
    }

    fn semantic_icon(&self) -> Option<SystemIcon> {
        match self {
            Self::Semantic { system_icon, .. } => system_icon.clone(),
            Self::Custom { .. } => None,
        }
    }

    fn icon_view(&self) -> Option<AnyViewBuilder<AnyView>> {
        match self {
            Self::Semantic { icon, .. } => icon.clone(),
            Self::Custom { .. } => None,
        }
    }

    fn with_icon<I: View + Clone>(mut self, icon: I, new_system_icon: Option<SystemIcon>) -> Self {
        match &mut self {
            Self::Semantic {
                icon: slot,
                system_icon,
                ..
            } => {
                *slot = Some(AnyViewBuilder::new(move || AnyView::new(icon.clone())));
                *system_icon = new_system_icon;
            }
            Self::Custom { .. } => {
                if new_system_icon.is_some() {
                    contract_violation!(
                        "Label::system_icon requires a semantic text label; compose icons inside Label::new content; the call is ignored"
                    );
                } else {
                    contract_violation!(
                        "Label::icon requires a semantic text label; compose icons inside Label::new content; the call is ignored"
                    );
                }
            }
        }
        self
    }

    fn set_icon_position(&mut self, position: IconPosition, method: &'static str) {
        match self {
            Self::Semantic { icon_position, .. } => *icon_position = position,
            Self::Custom { .. } => {
                contract_violation!(
                    "Label::{method} requires a semantic text label; arrange Label::new content directly; the call is ignored"
                );
            }
        }
    }

    fn set_spacing(&mut self, new_spacing: Computed<f32>) {
        match self {
            Self::Semantic { spacing, .. } => *spacing = new_spacing,
            Self::Custom { .. } => {
                contract_violation!(
                    "Label::spacing requires a semantic text label; space Label::new content directly; the call is ignored"
                );
            }
        }
    }

    fn set_font(&mut self, new_font: Font) {
        match self {
            Self::Semantic { font, .. } => *font = Some(new_font),
            Self::Custom { .. } => {
                contract_violation!(
                    "Label::font requires a semantic text label; style Label::new content directly; the call is ignored"
                );
            }
        }
    }

    fn set_default_text_line_limit(&mut self, limit: core::num::NonZeroUsize) {
        if let Self::Semantic { text, .. } = self {
            *text = text.clone().default_line_limit(limit);
        }
    }

    fn into_semantic(self) -> SemanticLabel {
        match self {
            Self::Semantic {
                text,
                system_icon,
                icon_position,
                spacing,
                font,
                ..
            } => SemanticLabel {
                text,
                system_icon,
                icon_position,
                spacing,
                font,
            },
            Self::Custom { semantic_text, .. } => SemanticLabel {
                text: semantic_text,
                system_icon: None,
                icon_position: IconPosition::Leading,
                spacing: Computed::constant(8.0),
                font: None,
            },
        }
    }

    fn into_erased(self) -> Self {
        self
    }

    fn into_slot_builder<S: ChildSlot>(
        self,
        mode: LabelDisplayMode,
    ) -> Rc<dyn Fn(Option<Font>) -> S> {
        Rc::new(move |default_font| {
            let mut label = self.clone();
            if !label.has_custom_content()
                && let Self::Semantic { font, .. } = &mut label
                && font.is_none()
            {
                *font = default_font;
            }
            S::from_view(label.body_erased(mode))
        })
    }

    fn dispatch<B: Backend>(
        self,
        mode: LabelDisplayMode,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        self.body_erased(mode).dispatch(backend, env, cx)
    }

    fn body_erased(self, mode: LabelDisplayMode) -> AnyView {
        match self {
            Self::Semantic {
                text,
                icon,
                icon_position,
                spacing,
                font,
                ..
            } => {
                let text = match font {
                    Some(font) => text.font(font),
                    None => text,
                };
                match mode {
                    LabelDisplayMode::Hidden => AnyView::new(()),
                    LabelDisplayMode::TitleOnly => AnyView::new(text),
                    LabelDisplayMode::IconOnly => AnyView::new(
                        icon.expect(
                            "effective_display_mode answers IconOnly only for a label with an icon",
                        )
                        .build(),
                    ),
                    LabelDisplayMode::TitleAndIcon | LabelDisplayMode::Automatic => {
                        let icon = icon
                            .expect(
                                "effective_display_mode answers TitleAndIcon only for a label with an icon",
                            )
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
                }
            }
            Self::Custom { view, .. } => match mode {
                LabelDisplayMode::Hidden | LabelDisplayMode::IconOnly => AnyView::new(()),
                _ => view.build(),
            },
        }
    }
}

/// Resolved control label carried by native control configurations in a
/// backend's child slot `S`.
pub struct ControlLabel<S> {
    build: Rc<dyn Fn(Option<Font>) -> S>,
    semantic_text: Text,
    semantic_icon: Option<SystemIcon>,
    accessibility_text: Option<Text>,
    accessibility_label: Option<Computed<StyledStr>>,
    display_mode: LabelDisplayMode,
    has_custom_content: bool,
    has_icon: bool,
}

impl<S> Clone for ControlLabel<S> {
    fn clone(&self) -> Self {
        Self {
            build: Rc::clone(&self.build),
            semantic_text: self.semantic_text.clone(),
            semantic_icon: self.semantic_icon.clone(),
            accessibility_text: self.accessibility_text.clone(),
            accessibility_label: self.accessibility_label.clone(),
            display_mode: self.display_mode,
            has_custom_content: self.has_custom_content,
            has_icon: self.has_icon,
        }
    }
}

impl<S> fmt::Debug for ControlLabel<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlLabel")
            .field("display_mode", &self.display_mode)
            .field("has_custom_content", &self.has_custom_content)
            .finish_non_exhaustive()
    }
}

impl<S> ControlLabel<S> {
    /// Materializes the label's visual child in slot `S`.
    #[must_use]
    pub fn build(&self) -> S {
        (self.build)(None)
    }

    /// Materializes the label's visual child in slot `S`, using `default_font`
    /// when the label is semantic and has no explicit font of its own.
    #[must_use]
    pub fn build_with_font(&self, default_font: Option<Font>) -> S {
        (self.build)(default_font)
    }

    /// Borrows the semantic text carried by this label.
    #[must_use]
    pub const fn semantic_text(&self) -> &Text {
        &self.semantic_text
    }

    /// Returns the resolved accessibility label signal.
    #[must_use]
    pub fn accessibility_label(&self) -> Computed<StyledStr> {
        self.accessibility_label.clone().unwrap_or_else(|| {
            self.accessibility_text
                .as_ref()
                .unwrap_or(&self.semantic_text)
                .content()
        })
    }

    /// Returns the resolved display mode.
    #[must_use]
    pub const fn display_mode_preference(&self) -> LabelDisplayMode {
        self.display_mode
    }

    /// Mutates the display mode in place.
    pub const fn set_display_mode(&mut self, mode: LabelDisplayMode) {
        self.display_mode = mode;
    }

    /// Resolves which presentation this label takes under `env`.
    #[must_use]
    pub fn effective_display_mode(&self, env: &Environment) -> LabelDisplayMode {
        resolve_display_mode(self.display_mode, self.has_icon, env)
    }

    /// Whether this label renders caller-supplied views rather than its own
    /// text and icon.
    #[must_use]
    pub const fn has_custom_content(&self) -> bool {
        self.has_custom_content
    }
}

impl<S: 'static> ControlLabel<S> {
    /// Moves the produced child slot through `f`.
    #[must_use]
    pub fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> ControlLabel<T> {
        let build = self.build;
        ControlLabel {
            build: Rc::new(move |font| f(build(font))),
            semantic_text: self.semantic_text,
            semantic_icon: self.semantic_icon,
            accessibility_text: self.accessibility_text,
            accessibility_label: self.accessibility_label,
            display_mode: self.display_mode,
            has_custom_content: self.has_custom_content,
            has_icon: self.has_icon,
        }
    }
}

impl<V: View> View for ControlLabel<V> {
    fn body(self, _env: &Environment) -> impl View {
        self.build()
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        self.build().dispatch(backend, env, cx)
    }
}

impl<V: View> sealed::Sealed for ControlLabel<V> {}

impl<V: View> LabelKind for ControlLabel<V> {
    type WithIcon<I: View + Clone> = IconLabel<I>;

    fn semantic_text(&self) -> &Text {
        &self.semantic_text
    }

    fn set_semantic_text(&mut self, text: Text) {
        self.semantic_text = text;
    }

    fn has_custom_content(&self) -> bool {
        self.has_custom_content
    }

    fn has_icon(&self) -> bool {
        self.has_icon
    }

    fn semantic_icon(&self) -> Option<SystemIcon> {
        self.semantic_icon.clone()
    }

    fn icon_view(&self) -> Option<AnyViewBuilder<AnyView>> {
        None
    }

    fn with_icon<I: View + Clone>(self, icon: I, system_icon: Option<SystemIcon>) -> IconLabel<I> {
        IconLabel {
            text: self.semantic_text,
            icon,
            system_icon,
            icon_position: IconPosition::Leading,
            spacing: Computed::constant(8.0),
            font: None,
        }
    }

    fn set_icon_position(&mut self, _position: IconPosition, _method: &'static str) {}

    fn set_spacing(&mut self, _spacing: Computed<f32>) {}

    fn set_font(&mut self, font: Font) {
        let prev = Rc::clone(&self.build);
        self.build = Rc::new(move |default| prev(Some(font.clone()).or(default)));
    }

    fn set_default_text_line_limit(&mut self, _limit: core::num::NonZeroUsize) {}

    fn into_semantic(self) -> SemanticLabel {
        SemanticLabel {
            text: self.semantic_text,
            system_icon: self.semantic_icon,
            icon_position: IconPosition::Leading,
            spacing: Computed::constant(8.0),
            font: None,
        }
    }

    fn into_erased(self) -> ErasedLabel {
        let build = self.build;
        ErasedLabel::Custom {
            semantic_text: self.semantic_text,
            view: AnyViewBuilder::new(move || AnyView::new(build(None))),
        }
    }

    fn into_slot_builder<S: ChildSlot>(
        self,
        mode: LabelDisplayMode,
    ) -> Rc<dyn Fn(Option<Font>) -> S> {
        let build = self.build;
        Rc::new(move |default_font| match mode {
            LabelDisplayMode::Hidden => S::from_view(()),
            _ => S::from_view(build(default_font)),
        })
    }

    fn dispatch<B: Backend>(
        self,
        mode: LabelDisplayMode,
        backend: &mut B,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        match mode {
            LabelDisplayMode::Hidden => ().dispatch(backend, env, cx),
            _ => self.build().dispatch(backend, env, cx),
        }
    }

    fn body_erased(self, mode: LabelDisplayMode) -> AnyView {
        match mode {
            LabelDisplayMode::Hidden => AnyView::new(()),
            _ => AnyView::new(self.build()),
        }
    }
}

impl Default for Label<TitleLabel> {
    fn default() -> Self {
        Self::semantic(Text::default())
    }
}

/// Sets the display mode in place. Companion to the consuming
/// [`Label::display_mode`] builder, used by the [`impl_label_style_methods`]
/// macro to mutate a control's `label: Label<K>` field without taking ownership.
impl<K> Label<K> {
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
/// control struct whose [`Label`] is accessible via `label_mut()`.
///
/// Reused by every `WaterUI` control crate so the same `.label_style(...)` /
/// `.hide_label()` surface appears on every control without forcing a trait
/// into the user's import path.
#[macro_export]
macro_rules! impl_label_style_methods {
    ($ty:ident < $($gen:ident),+ >) => {
        impl<$($gen),+> $ty<$($gen),+> {
            /// Sets the visual presentation mode for this control's label.
            ///
            /// The semantic text of the label is always retained for assistive
            /// technology regardless of the chosen visual mode.
            pub const fn label_style(mut self, mode: $crate::label::LabelDisplayMode) -> Self {
                self.label_mut().set_display_mode(mode);
                self
            }

            /// Visually hides the label while preserving its semantic text
            /// for assistive technology.
            ///
            /// Shortcut for [`Self::label_style`] with
            /// [`LabelDisplayMode::Hidden`](crate::label::LabelDisplayMode::Hidden).
            pub const fn hide_label(self) -> Self {
                self.label_style($crate::label::LabelDisplayMode::Hidden)
            }
        }
    };
    ($ty:ident) => {
        impl $ty {
            /// Sets the visual presentation mode for this control's label.
            ///
            /// The semantic text of the label is always retained for assistive
            /// technology regardless of the chosen visual mode.
            pub const fn label_style(mut self, mode: $crate::label::LabelDisplayMode) -> Self {
                self.label_mut().set_display_mode(mode);
                self
            }

            /// Visually hides the label while preserving its semantic text
            /// for assistive technology.
            ///
            /// Shortcut for [`Self::label_style`] with
            /// [`LabelDisplayMode::Hidden`](crate::label::LabelDisplayMode::Hidden).
            pub const fn hide_label(self) -> Self {
                self.label_style($crate::label::LabelDisplayMode::Hidden)
            }
        }
    };
}

pub use impl_label_style_methods;

/// Semantic label for controls, commands, and chrome.
#[derive(Debug, Clone)]
#[expect(
    clippy::struct_field_names,
    reason = "the `accessibility_` prefix groups the accessibility-override fields"
)]
pub struct Label<K> {
    content: K,
    display_mode: LabelDisplayMode,
    accessibility_text: Option<Text>,
    accessibility_label: Option<Computed<StyledStr>>,
}

/// Converts semantic text inputs into a [`Label`].
///
/// String, [`Text`], [`StyledStr`], [`Binding`], and [`Computed`] inputs use
/// `WaterUI`'s i18n-aware semantic text pipeline. Use [`Label::new`] only when
/// the visible label is an arbitrary view that differs from its spoken text.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as a control label",
    label = "expected a label",
    note = "`IntoLabel` is implemented by `&'static str`, `String`, `Str`, `StyledStr`, `Text`, `Label`, and `Binding<T>` / `Computed<T>` whose `T` is `IntoText`. For a label rendered as an arbitrary view, use `Label::new(semantic_text, || view)`."
)]
pub trait IntoLabel {
    /// Concrete label kind produced by this value.
    type Kind: LabelKind;

    /// Converts a value into a semantic label.
    fn into_label(self) -> Label<Self::Kind>;
}

impl<K: LabelKind> IntoLabel for Label<K> {
    type Kind = K;

    fn into_label(self) -> Self {
        self
    }
}

impl IntoLabel for Text {
    type Kind = TitleLabel;

    fn into_label(self) -> Label<TitleLabel> {
        Label::semantic(self)
    }
}

impl IntoLabel for &'static str {
    type Kind = TitleLabel;

    fn into_label(self) -> Label<TitleLabel> {
        Label::semantic(self)
    }
}

impl IntoLabel for alloc::string::String {
    type Kind = TitleLabel;

    fn into_label(self) -> Label<TitleLabel> {
        Label::semantic(self)
    }
}

impl IntoLabel for waterui_core::Str {
    type Kind = TitleLabel;

    fn into_label(self) -> Label<TitleLabel> {
        Label::semantic(self)
    }
}

impl IntoLabel for StyledStr {
    type Kind = TitleLabel;

    fn into_label(self) -> Label<TitleLabel> {
        Label::semantic(self)
    }
}

impl<T> IntoLabel for Computed<T>
where
    T: IntoText + Clone + 'static,
{
    type Kind = TitleLabel;

    fn into_label(self) -> Label<TitleLabel> {
        Label::semantic(self)
    }
}

impl<T> IntoLabel for Binding<T>
where
    T: IntoText + Clone + 'static,
{
    type Kind = TitleLabel;

    fn into_label(self) -> Label<TitleLabel> {
        Label::semantic(self)
    }
}

impl<B: ViewBuilder> Label<CustomLabel<Built<B>>> {
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
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_controls::Label;
    /// # fn verification_badge() -> impl View {
    /// #     text("✓")
    /// # }
    /// # fn build() -> impl IntoLabel {
    /// let verified = Label::new(
    ///     "Verified account",
    ///     || hstack((text("Account"), verification_badge())),
    /// );
    /// # verified
    /// # }
    /// ```
    #[must_use]
    pub fn new(semantic_text: impl IntoText, content: B) -> Self {
        Self {
            content: CustomLabel {
                semantic_text: semantic_text.into_text(),
                view: Built::new(content),
            },
            display_mode: LabelDisplayMode::Automatic,
            accessibility_text: None,
            accessibility_label: None,
        }
    }
}

impl Label<TitleLabel> {
    fn semantic(text: impl IntoText) -> Self {
        Self {
            content: TitleLabel {
                text: text.into_text(),
                icon_position: IconPosition::Leading,
                spacing: Computed::constant(8.0),
                font: None,
            },
            display_mode: LabelDisplayMode::Automatic,
            accessibility_text: None,
            accessibility_label: None,
        }
    }
}

impl<K> Label<K> {
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

    /// Overrides the spoken accessibility text without changing the visual text.
    pub(crate) fn accessibility_text(mut self, text: impl IntoText) -> Self {
        self.accessibility_text = Some(text.into_text());
        self.accessibility_label = None;
        self
    }
}

impl<S> Label<ControlLabel<S>> {
    /// Rebuilds a [`Label`] from a [`ControlLabel`] and display mode.
    #[doc(hidden)]
    #[must_use]
    pub fn from_control_label(label: ControlLabel<S>, display_mode: LabelDisplayMode) -> Self {
        let accessibility_text = label.accessibility_text.clone();
        let accessibility_label = label.accessibility_label.clone();
        Self {
            content: label,
            display_mode,
            accessibility_text,
            accessibility_label,
        }
    }
}

impl<K: LabelKind> Label<K> {
    /// Replaces the text payload.
    #[must_use]
    pub fn text(mut self, text: impl IntoText) -> Self {
        self.content.set_semantic_text(text.into_text());
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
    /// # Contract violation
    ///
    /// Calling it on a [`Label::new`] label with arbitrary visual content is
    /// a contract violation: a debug build panics, a release build logs it
    /// and ignores the call.
    /// `icon` is a value rather than a builder so a [`SystemIcon`] stays
    /// recognizable: the semantic identity of an SF Symbol is what lets Apple
    /// chrome render it natively, and a closure would erase it. Size an icon by
    /// choosing the icon's own size, or wrap the whole label with
    /// [`Label::new`], which does take a builder.
    #[must_use]
    pub fn icon<I: View + Clone>(self, icon: I) -> Label<K::WithIcon<I>> {
        let system_icon = (&icon as &dyn Any).downcast_ref::<SystemIcon>().cloned();
        Label {
            content: self.content.with_icon(icon, system_icon),
            display_mode: self.display_mode,
            accessibility_text: self.accessibility_text,
            accessibility_label: self.accessibility_label,
        }
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
    /// # Contract violation
    ///
    /// Calling it on a [`Label::new`] label with arbitrary visual content is
    /// a contract violation: a debug build panics, a release build logs it
    /// and ignores the call.
    #[must_use]
    pub fn system_icon(self, icon: SystemIcon) -> Label<K::WithIcon<SystemIcon>> {
        let semantic = Some(icon.clone());
        Label {
            content: self.content.with_icon(icon, semantic),
            display_mode: self.display_mode,
            accessibility_text: self.accessibility_text,
            accessibility_label: self.accessibility_label,
        }
    }

    /// Places the icon on the trailing side.
    ///
    /// # Contract violation
    ///
    /// Calling it on a [`Label::new`] label with arbitrary visual content is
    /// a contract violation: a debug build panics, a release build logs it
    /// and ignores the call.
    #[must_use]
    pub fn trailing(mut self) -> Self {
        self.content
            .set_icon_position(IconPosition::Trailing, "trailing");
        self
    }

    /// Places the icon on the leading side.
    ///
    /// # Contract violation
    ///
    /// Calling it on a [`Label::new`] label with arbitrary visual content is
    /// a contract violation: a debug build panics, a release build logs it
    /// and ignores the call.
    #[must_use]
    pub fn leading(mut self) -> Self {
        self.content
            .set_icon_position(IconPosition::Leading, "leading");
        self
    }

    /// Sets the spacing between icon and text.
    ///
    /// Signal changes invalidate only the label stack's layout.
    ///
    /// # Contract violation
    ///
    /// Calling it on a [`Label::new`] label with arbitrary visual content is
    /// a contract violation: a debug build panics, a release build logs it
    /// and ignores the call.
    #[must_use]
    pub fn spacing(mut self, spacing: impl IntoSignalF32 + 'static) -> Self {
        self.content
            .set_spacing(spacing.into_signal_f32().computed());
        self
    }

    /// Sets the visual font used by the label text.
    ///
    /// This affects only the on-screen label text. The semantic text exposed to
    /// controls and assistive technology remains unchanged.
    ///
    /// # Contract violation
    ///
    /// Calling it on a [`Label::new`] label is a contract violation, because
    /// arbitrary content owns its own visual styling: a debug build panics, a
    /// release build logs it and ignores the call.
    #[must_use]
    pub fn font(mut self, font: impl Into<Font>) -> Self {
        self.content.set_font(font.into());
        self
    }

    /// Resolves which presentation this label takes under `env`.
    ///
    /// An [`LabelDisplayMode::Automatic`] preference first consults the
    /// environment's `LabelDisplayMode`, then falls back through what the
    /// label can actually draw: a hidden label stays hidden, a label with an
    /// icon renders its icon (`TitleAndIcon`, or `IconOnly` when requested),
    /// and anything else renders its title. The resolved mode is never
    /// `Automatic` once [`Self::resolve`] has run.
    ///
    /// A backend choosing chrome for a label-driven control reads this rather
    /// than re-deriving the same resolution, so its presentation always
    /// agrees with what the label draws.
    #[must_use]
    pub fn effective_display_mode(&self, env: &Environment) -> LabelDisplayMode {
        resolve_display_mode(self.display_mode, self.content.has_icon(), env)
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
    pub fn has_custom_content(&self) -> bool {
        self.content.has_custom_content()
    }

    /// Returns the semantic text carried by this label.
    #[must_use = "this borrows the label's text without consuming the label"]
    pub fn semantic_text(&self) -> &Text {
        self.content.semantic_text()
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
        self.content.semantic_icon()
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
        self.content.icon_view()
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
        self.content.set_default_text_line_limit(limit);
        self
    }

    /// Resolves environment-dependent label presentation before crossing into
    /// a native backend payload.
    #[must_use]
    pub fn resolve(mut self, env: &Environment) -> Self {
        self.resolve_in_place(env);
        self
    }

    /// Resolves environment-dependent label presentation in place. See
    /// [`Label::resolve`] for the owning variant.
    pub fn resolve_in_place(&mut self, env: &Environment) {
        self.display_mode = self.effective_display_mode(env);
        let accessibility_text = self
            .accessibility_text
            .as_ref()
            .unwrap_or_else(|| self.content.semantic_text());
        self.accessibility_label = Some(accessibility_text.resolve(env).into_content());
    }

    /// Strips any caller-supplied custom visual view or custom icon, keeping
    /// the semantic text, any [`SystemIcon`], display mode, and accessibility
    /// overrides.
    #[must_use]
    pub fn into_semantic(self) -> Label<SemanticLabel> {
        Label {
            content: self.content.into_semantic(),
            display_mode: self.display_mode,
            accessibility_text: self.accessibility_text,
            accessibility_label: self.accessibility_label,
        }
    }

    /// Erases any visual icon or custom view into `Label<ErasedLabel>`, for
    /// runtime-open collections whose items carry heterogeneous label types.
    #[must_use]
    pub fn erase(self) -> Label<ErasedLabel> {
        Label {
            content: self.content.into_erased(),
            display_mode: self.display_mode,
            accessibility_text: self.accessibility_text,
            accessibility_label: self.accessibility_label,
        }
    }

    /// Resolves this label into a [`ControlLabel<S>`] producing its visual
    /// subtree in the backend's child slot `S`.
    #[must_use]
    pub fn into_control_label<S: ChildSlot>(self, env: &Environment) -> ControlLabel<S> {
        let resolved = self.resolve(env);
        let mode = resolved.display_mode;
        let semantic_text = resolved.content.semantic_text().clone();
        let semantic_icon = resolved.content.semantic_icon();
        let accessibility_text = resolved.accessibility_text.clone();
        let accessibility_label = resolved.accessibility_label();
        let has_custom_content = resolved.content.has_custom_content();
        let has_icon = resolved.content.has_icon();
        let build = resolved.content.into_slot_builder::<S>(mode);
        ControlLabel {
            build,
            semantic_text,
            semantic_icon,
            accessibility_text,
            accessibility_label: Some(accessibility_label),
            display_mode: mode,
            has_custom_content,
            has_icon,
        }
    }

    /// Converts an un-resolved label into a [`ControlLabel<Label<K>>`] for
    /// component configuration extraction.
    #[must_use]
    pub fn into_unscoped_control_label(self) -> ControlLabel<Self> {
        let mode = self.display_mode;
        let semantic_text = self.content.semantic_text().clone();
        let semantic_icon = self.content.semantic_icon();
        let accessibility_text = self.accessibility_text.clone();
        let accessibility_label = self.accessibility_label.clone();
        let has_custom_content = self.content.has_custom_content();
        let has_icon = self.content.has_icon();
        let build = Rc::new(move |default_font: Option<Font>| match default_font {
            Some(font) if !self.has_custom_content() => self.clone().font(font),
            _ => self.clone(),
        });
        ControlLabel {
            build,
            semantic_text,
            semantic_icon,
            accessibility_text,
            accessibility_label,
            display_mode: mode,
            has_custom_content,
            has_icon,
        }
    }
}

impl<K: LabelKind> View for Label<K> {
    fn body(self, env: &Environment) -> impl View {
        let mode = self.effective_display_mode(env);
        self.content.body_erased(mode)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let mode = self.effective_display_mode(env);
        self.content.dispatch(mode, backend, env, cx)
    }
}

/// Creates an i18n-aware semantic text label.
///
/// This is the ergonomic counterpart to [`Label::new`], which accepts
/// arbitrary visual content plus separate semantic text.
#[must_use]
pub fn label(text: impl IntoText) -> Label<TitleLabel> {
    Label::semantic(text)
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use nami::{Computed, Signal};
    use waterui_core::Environment;
    use waterui_locale::{TranslationCatalog, locales};

    use super::{IntoLabel, LabelKind};

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

    fn semantic_plain<K: LabelKind>(label: &super::Label<K>, env: &Environment) -> String {
        label
            .semantic_text()
            .resolve(env)
            .content()
            .snapshot()
            .to_plain()
            .into_string()
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "Label::trailing requires a semantic text label")
    )]
    fn icon_placement_on_arbitrary_content_is_ignored() {
        let env = test_env();
        let label = super::Label::new("greeting", || waterui_text::text("Visual")).trailing();

        // Release outcome: the call is ignored and the label is unchanged.
        assert!(label.has_custom_content());
        assert_eq!(semantic_plain(&label, &env), "Hello");
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "Label::font requires a semantic text label")
    )]
    fn font_on_arbitrary_content_is_ignored() {
        let env = test_env();
        let label = super::Label::new("greeting", || waterui_text::text("Visual"))
            .font(waterui_text::font::Body);

        assert!(label.has_custom_content());
        assert_eq!(semantic_plain(&label, &env), "Hello");
    }

    #[test]
    fn font_keeps_semantic_text_unchanged() {
        let env = test_env();
        let label = super::label("greeting").font(waterui_text::font::Body);

        assert_eq!(semantic_plain(&label, &env), "Hello");
    }
}
