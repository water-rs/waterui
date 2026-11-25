//! Confirmation / alert dialog (`Dialog`).
//!
//! A [`Dialog`] is a window-modal alert presented by a binding:
//! `content.dialog(Dialog::new(&presented, "Delete draft?"))` registers the
//! dialog on the subtree; while `presented` is `true` the content beneath is
//! inert to pointer, keyboard and accessibility, focus moves into the dialog,
//! and the dialog answers with the handler of the action the user chose.
//!
//! Backends realize the same semantic object differently: Apple platforms
//! present a native alert (`NSAlert` sheet on `AppKit`, `UIAlertController`
//! on `UIKit`); Hydrolysis — Android included — draws the composed card on its
//! modal layer.
//!
//! # Examples
//!
//! ```rust,no_run
//! use waterui::prelude::*;
//! use waterui::dialog::{Dialog, DialogAction};
//!
//! fn view() -> impl View {
//!     let confirm = binding(false);
//!     vstack((
//!         button("Paste").action({
//!             let confirm = confirm.clone();
//!             move || confirm.set(true)
//!         }),
//!         text("editor").dialog(
//!             Dialog::new(&confirm, "Paste multiple lines?")
//!                 .message("The clipboard holds more than one line.")
//!                 .action(DialogAction::new("Paste", || {}))
//!                 .action(DialogAction::cancel("Cancel", || {})),
//!         ),
//!     ))
//! }
//! ```

use nami::Binding;
use waterui_accessibility::AccessibilityRole;
use waterui_controls::{Button, ButtonRole, ButtonStyle, LabelKind, button};
use waterui_core::handler::{
    ExtractionError, Handler, ReportExtraction as _, SharedAction, shared_action,
};
use waterui_core::metadata::MetadataKey;
use waterui_core::{Environment, OneOf2, View};
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::{HorizontalAlignment, hstack, vstack, zstack};
use waterui_text::text::{IntoText, Text, text};

use crate::shape::{RoundedRectangle, ShapeExt};
use crate::theme::color::Scrim;
pub use crate::widget::DialogTheme;
use crate::{Computed, SignalExt as _, Str, ViewExt};
use waterui_graphics::color::Color;

/// Translation key for the acknowledgement button a dialog declared without
/// actions presents.
///
/// A self-drawn backend renders this entry; `AppKit` draws its own localized
/// `NSAlert` button instead, so the platform's wording wins where one exists.
pub const DIALOG_ACKNOWLEDGE_LABEL_KEY: &str = "waterui.dialog.acknowledge";

/// One action button of a [`Dialog`]: a title, an optional [`ButtonRole`],
/// and the handler that runs when the user chooses it.
///
/// Ordering inside the action row follows the platform convention: `Cancel`
/// first, `Destructive` next, then ordinary (`None`) confirmation actions in
/// declaration order. At most one `Cancel` action is allowed — more than one
/// fails at render. The first ordinary (`None`) action is the primary one and
/// is bound to `Return` on platforms with a hardware Return key.
///
/// Titles are plain text — native alert buttons take a title only, so this
/// accepts [`IntoText`], not `IntoLabel` (an icon would be silently dropped on
/// Apple).
#[derive(Clone)]
pub struct DialogAction {
    title: Text,
    role: Option<ButtonRole>,
    action: SharedAction,
}

impl core::fmt::Debug for DialogAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DialogAction")
            .field("title", &self.title)
            .field("role", &self.role)
            .field("action", &self.action)
            .finish()
    }
}

impl DialogAction {
    /// Creates an ordinary confirmation action (`role = None`).
    pub fn new<H, Args>(title: impl IntoText, action: H) -> Self
    where
        H: Handler<Args, ()> + 'static,
    {
        Self::with_role(title, None, action)
    }

    /// Creates an action with [`ButtonRole::Destructive`].
    pub fn destructive<H, Args>(title: impl IntoText, action: H) -> Self
    where
        H: Handler<Args, ()> + 'static,
    {
        Self::with_role(title, Some(ButtonRole::Destructive), action)
    }

    /// Creates an action with [`ButtonRole::Cancel`].
    pub fn cancel<H, Args>(title: impl IntoText, action: H) -> Self
    where
        H: Handler<Args, ()> + 'static,
    {
        Self::with_role(title, Some(ButtonRole::Cancel), action)
    }

    /// Creates an action with an explicit optional [`ButtonRole`].
    pub fn with_role<H, Args>(title: impl IntoText, role: Option<ButtonRole>, action: H) -> Self
    where
        H: Handler<Args, ()> + 'static,
    {
        Self {
            title: title.into_text(),
            role,
            action: shared_action(action),
        }
    }

    /// The button title.
    #[must_use]
    pub const fn title(&self) -> &Text {
        &self.title
    }

    /// The semantic role this action carries, if any.
    #[must_use]
    pub const fn role(&self) -> Option<ButtonRole> {
        self.role
    }

    /// The handler chosen by pressing this action.
    #[must_use]
    pub const fn action(&self) -> &SharedAction {
        &self.action
    }
}

impl<L, Action, Style> From<Button<L, Action, Style>> for DialogAction
where
    L: LabelKind,
    Action: FnMut(&Environment) -> Result<(), ExtractionError> + 'static,
{
    fn from(value: Button<L, Action, Style>) -> Self {
        let (label, mut action, role) = value.into_action_parts();
        Self {
            title: label.semantic_text().clone(),
            role,
            action: shared_action(move |env: Environment| {
                action(&env).report_extraction();
            }),
        }
    }
}

/// A confirmation / alert dialog, presented window-modal by a binding.
///
/// Attach it to the view that owns the alert with
/// [`ViewExt::dialog`](crate::ViewExt::dialog):
///
/// ```rust,no_run
/// use waterui::prelude::*;
/// use waterui::dialog::{Dialog, DialogAction};
///
/// let confirm = binding(false);
/// text("Editor").dialog(
///     Dialog::new(&confirm, "Discard changes?")
///         .action(DialogAction::destructive("Discard", || {}))
///         .action(DialogAction::cancel("Keep", || {})),
/// );
/// ```
///
/// While `is_presented` is `true` the dialog is window-modal: the content
/// beneath it is inert, focus is contained inside the dialog, and the content
/// returns focus on dismissal. Choosing an action runs its handler and the
/// backend writes `is_presented = false`; the cancel path — Escape, the
/// Android back gesture, the scrim tap on Hydrolysis — runs the `Cancel`
/// action's handler the same way. Setting `is_presented = false` from the app
/// dismisses the dialog without running any handler.
///
/// One dialog is presented per window at a time; a second one presented while
/// one is up waits in presentation order.
#[derive(Clone)]
pub struct Dialog {
    is_presented: Binding<bool>,
    title: Text,
    message: Option<Text>,
    actions: Vec<DialogAction>,
}

impl MetadataKey for Dialog {}

impl core::fmt::Debug for Dialog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Dialog")
            .field("title", &self.title)
            .field("message", &self.message)
            .field("actions", &self.actions)
            .finish_non_exhaustive()
    }
}

impl Dialog {
    /// Creates a dialog bound to `is_presented` with the given title.
    ///
    /// The title is reactive text — `text!("Paste {} lines?", lines)` keeps it
    /// live while the dialog is up.
    #[must_use]
    pub fn new(is_presented: &Binding<bool>, title: impl IntoText) -> Self {
        Self {
            is_presented: is_presented.clone(),
            title: title.into_text(),
            message: None,
            actions: Vec::new(),
        }
    }

    /// Adds a supporting message under the title.
    #[must_use]
    pub fn message(mut self, message: impl IntoText) -> Self {
        self.message = Some(message.into_text());
        self
    }

    /// Appends an action to the dialog. Declaration order is kept within each
    /// role; roles are laid out in platform convention order.
    #[must_use]
    pub fn action(mut self, action: impl Into<DialogAction>) -> Self {
        self.actions.push(action.into());
        self
    }

    /// The presentation binding the dialog was created with.
    #[must_use]
    pub const fn is_presented(&self) -> &Binding<bool> {
        &self.is_presented
    }

    /// The dialog's headline.
    #[must_use]
    pub const fn title(&self) -> &Text {
        &self.title
    }

    /// The supporting message, when set.
    #[must_use]
    pub const fn message_text(&self) -> Option<&Text> {
        self.message.as_ref()
    }

    /// The declared actions, before validation and platform ordering.
    #[must_use]
    pub fn actions(&self) -> &[DialogAction] {
        &self.actions
    }

    /// The `Cancel` action, when one was declared.
    ///
    /// Shared by every realization's cancel path — Escape, the Android back
    /// gesture, the Hydrolysis scrim tap — so they all run the same handler.
    #[must_use]
    pub fn cancel_action(&self) -> Option<&DialogAction> {
        self.actions
            .iter()
            .find(|action| action.role == Some(ButtonRole::Cancel))
    }

    /// The action list as renderers must present it: roles in platform
    /// convention order — dismissive first, affirmative trailing — with
    /// declaration order kept within each role, and a single acknowledgement
    /// button with role `Cancel` when the declaration was empty (the same
    /// default `NSAlert` draws). The acknowledgement's title is the
    /// [`DIALOG_ACKNOWLEDGE_LABEL_KEY`] catalog entry.
    ///
    /// # Contract
    ///
    /// Declaring more than one `Cancel` action breaks the contract —
    /// validation fails fast at render, not at first presentation: a debug
    /// build panics, and a release build logs the error and keeps every
    /// declared action, in the order above.
    #[must_use]
    pub fn resolved_actions(&self) -> Vec<DialogAction> {
        if self
            .actions
            .iter()
            .filter(|action| action.role == Some(ButtonRole::Cancel))
            .count()
            > 1
        {
            crate::contract_violation!(
                "Dialog declares more than one Cancel action; every declared action is kept"
            );
        }
        if self.actions.is_empty() {
            return vec![DialogAction::cancel(
                Text::localized_or(DIALOG_ACKNOWLEDGE_LABEL_KEY, "OK"),
                || {},
            )];
        }
        let mut ordered = Vec::with_capacity(self.actions.len());
        ordered.extend(
            self.actions
                .iter()
                .filter(|action| action.role == Some(ButtonRole::Cancel))
                .cloned(),
        );
        ordered.extend(
            self.actions
                .iter()
                .filter(|action| action.role == Some(ButtonRole::Destructive))
                .cloned(),
        );
        ordered.extend(
            self.actions
                .iter()
                .filter(|action| {
                    !matches!(
                        action.role,
                        Some(ButtonRole::Cancel | ButtonRole::Destructive)
                    )
                })
                .cloned(),
        );
        ordered
    }

    /// The primary action: the first ordinary (`None`) one, bound to Return.
    ///
    /// `None` when no ordinary action was declared — Return then has no
    /// dialog-level meaning and activates the focused button.
    #[must_use]
    pub fn primary_action(&self) -> Option<&DialogAction> {
        self.actions.iter().find(|action| action.role.is_none())
    }

    /// The shared "chose this action" step: run the handler, then write the
    /// presentation binding back — the backend's half of the action contract.
    ///
    /// This is the backend-facing entry point: `components` call it instead of
    /// touching the handler directly so every realization answers identically.
    #[doc(hidden)]
    pub fn run_action(&self, action: &DialogAction, env: &Environment) {
        // A handler whose parameters cannot be extracted does not run; the
        // failure is reported and the dialog still dismisses, as the user
        // answered it.
        action.action.run(env);
        self.is_presented.set(false);
    }

    /// The shared cancel path: runs the `Cancel` handler and dismisses, or
    /// does nothing when the dialog declared none — a dialog without `Cancel`
    /// must be answered.
    #[doc(hidden)]
    pub fn run_cancel(&self, env: &Environment) {
        if let Some(cancel) = self.cancel_action() {
            self.run_action(cancel, env);
        }
    }
}

/// One dialog action as a card button styled with `theme`.
fn action_button(
    action: DialogAction,
    dialog: &Dialog,
    primary: bool,
    theme: &DialogTheme,
) -> impl View + use<> {
    let title = text(action.title().clone());
    let role = action.role();
    let color = match role {
        Some(ButtonRole::Destructive) => theme.destructive_action_color.clone(),
        _ if primary => theme.primary_action_color.clone(),
        _ => theme.default_action_color.clone(),
    };
    let label = title.color(color);
    let dialog = dialog.clone();
    let btn = button(label)
        .style(ButtonStyle::Plain)
        .action(move |env: Environment| dialog.run_action(&action, &env));
    match role {
        Some(role) => btn.role(role),
        None => btn,
    }
}

/// The alert card the Hydrolysis modal layer draws using the active
/// [`DialogTheme`] from the environment, carrying the `Dialog` accessibility
/// role and labelled by the title.
struct DialogCard(Dialog);

impl View for DialogCard {
    fn body(self, env: &Environment) -> impl View {
        let dialog = &self.0;
        let theme = env.get::<DialogTheme>().cloned().unwrap_or_default();
        // Resolving first is what lets the label read a localized title: an
        // unresolved environment-dependent `Text` has no content signal.
        let spoken = dialog
            .title
            .resolve(env)
            .content()
            .map(|title| title.to_plain())
            .computed();
        dialog_card(dialog, spoken, &theme)
    }
}

fn dialog_card(dialog: &Dialog, label: Computed<Str>, theme: &DialogTheme) -> impl View + use<> {
    let actions = dialog.resolved_actions();
    let mut seen_default = false;
    let buttons: Vec<_> = actions
        .into_iter()
        .map(|action| {
            let primary = action.role().is_none() && !seen_default;
            seen_default |= action.role().is_none();
            action_button(action, dialog, primary, theme)
        })
        .collect();
    let headline = text(dialog.title.clone())
        .headline()
        .color(theme.title_color.clone());
    let actions_row = Frame::new(
        hstack(buttons)
            .spacing(theme.actions_spacing)
            .padding_with(EdgeInsets::new(theme.actions_top_padding, 0.0, 0.0, 0.0)),
    )
    .max_width(f32::INFINITY)
    .alignment(theme.actions_alignment);
    let contents = match &dialog.message {
        Some(message) => OneOf2::A(
            vstack((
                headline,
                text(message.clone()).color(theme.message_color.clone()),
                actions_row,
            ))
            .alignment(HorizontalAlignment::Leading)
            .spacing(theme.content_spacing),
        ),
        None => OneOf2::B(
            vstack((headline, actions_row))
                .alignment(HorizontalAlignment::Leading)
                .spacing(theme.content_spacing),
        ),
    };

    Frame::new(
        contents
            .padding_with(theme.content_padding.clone())
            .background(
                RoundedRectangle::new(theme.corner_radius).fill(theme.container_color.clone()),
            )
            .a11y_label(label)
            .a11y_role(AccessibilityRole::Dialog),
    )
    .min_width(theme.min_width)
    .max_width(theme.max_width)
}

impl Dialog {
    /// The self-drawn realization a backend's modal layer presents: a
    /// full-window `Scrim` backdrop — tapping it runs the cancel path — with
    /// the alert card centered above it.
    ///
    /// Backend-facing, like [`Self::run_action`]: an application presents a
    /// dialog through [`ViewExt::dialog`](crate::ViewExt::dialog) only.
    #[doc(hidden)]
    pub fn modal_layer(&self) -> impl View + use<> {
        let scrim_dialog = self.clone();
        zstack((
            // The scrim is a pointer target only: the card's own cancel
            // action and Escape carry dismissal for assistive technology, so
            // the backdrop must not become a focusable element ahead of them.
            Color::new(Scrim)
                .on_tap(move |env: Environment| scrim_dialog.run_cancel(&env))
                .a11y_hidden(true),
            DialogCard(self.clone()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use nami::Binding;
    use waterui_controls::{ButtonRole, button};

    use super::{Dialog, DialogAction};

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "more than one Cancel action")
    )]
    fn two_cancel_actions_are_all_kept() {
        let dialog = Dialog::new(&Binding::bool(true), "Title")
            .action(DialogAction::cancel("First", || {}))
            .action(DialogAction::cancel("Second", || {}));
        let roles = dialog
            .resolved_actions()
            .iter()
            .map(DialogAction::role)
            .collect::<Vec<_>>();
        assert_eq!(roles, [Some(ButtonRole::Cancel), Some(ButtonRole::Cancel)]);
    }

    #[test]
    fn button_conversion_preserves_role_in_dialog_actions() {
        let dialog = Dialog::new(&Binding::bool(true), "Title")
            .action(button("Save").action(|| {}))
            .action(button("Delete").role(ButtonRole::Destructive).action(|| {}))
            .action(button("Cancel").role(ButtonRole::Cancel).action(|| {}));
        let roles = dialog
            .resolved_actions()
            .iter()
            .map(DialogAction::role)
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                Some(ButtonRole::Cancel),
                Some(ButtonRole::Destructive),
                None,
            ]
        );
    }

    #[test]
    fn dialog_theme_plugin_overrides_card_widths() {
        use waterui_core::plugin::Plugin as _;
        use waterui_core::{AnyView, Environment, Native, View};
        use waterui_layout::{ProposalSize, container::FixedContainer};

        use super::{DialogCard, DialogTheme};

        let dialog = Dialog::new(&Binding::bool(true), "Confirm");
        let default_env = Environment::new();
        let default_view = DialogCard(dialog.clone())
            .body(&default_env)
            .body(&default_env)
            .body(&default_env);
        let default_native = AnyView::new(default_view)
            .downcast::<Native<FixedContainer<Vec<AnyView>>>>()
            .expect("DialogCard expands to Native<FixedContainer>");
        let default_container = default_native.into_inner();
        let (default_layout, _) = default_container.as_parts();
        let default_min = default_layout
            .size_that_fits(ProposalSize::new(100.0, 200.0), &[])
            .width;
        assert!((default_min - DialogTheme::default().min_width).abs() < f32::EPSILON);

        let mut custom_env = Environment::new();
        DialogTheme {
            min_width: 340.0,
            max_width: 620.0,
            ..DialogTheme::default()
        }
        .install(&mut custom_env);

        let custom_view = DialogCard(dialog)
            .body(&custom_env)
            .body(&custom_env)
            .body(&custom_env);
        let custom_native = AnyView::new(custom_view)
            .downcast::<Native<FixedContainer<Vec<AnyView>>>>()
            .expect("DialogCard expands to Native<FixedContainer>");
        let custom_container = custom_native.into_inner();
        let (custom_layout, _) = custom_container.as_parts();
        let custom_min = custom_layout
            .size_that_fits(ProposalSize::new(100.0, 200.0), &[])
            .width;
        assert!((custom_min - 340.0).abs() < f32::EPSILON);
    }
}
