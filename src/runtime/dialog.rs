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
//! let confirm = binding(false);
//! vstack((
//!     button("Paste").action({
//!         let confirm = confirm.clone();
//!         move || confirm.set(true)
//!     }),
//!     text("editor").dialog(
//!         Dialog::new(&confirm, "Paste multiple lines?")
//!             .message("The clipboard holds more than one line.")
//!             .action(DialogAction::new("Paste", || {}))
//!             .action(DialogAction::cancel("Cancel", || {})),
//!     ),
//! ))
//! ```

use nami::Binding;
use waterui_controls::{ButtonStyle, button};
use waterui_core::accessibility::AccessibilityRole;
use waterui_core::extract::Use;
use waterui_core::handler::{Handler, SharedAction, shared_action};
use waterui_core::key::{Key, KeyHandling, KeyPress, NamedKey};
use waterui_core::metadata::MetadataKey;
use waterui_core::{Environment, View};
use waterui_layout::alignment;
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::{hstack, vstack, zstack};
use waterui_text::text::{IntoText, Text, text};

use crate::shape::{FixedRoundedRectangle, ShapeExt};
use crate::theme::color::{Accent, Error, Foreground, MutedForeground, Scrim, Surface};
use crate::{AnyView, ViewExt};
use waterui_graphics::color::Color;

/// M3 `dialog.container.min-width`: the narrowest a card gets, in dp.
const CARD_MIN_WIDTH: f32 = 280.0;
/// M3 `dialog.container.max-width`: the widest a card gets, in dp.
const CARD_MAX_WIDTH: f32 = 560.0;
/// M3 `dialog.container.corner-radius` (extra-large shape scale), in dp.
const CARD_CORNER_RADIUS: f32 = 28.0;
/// M3 `dialog.container.padding`, in dp.
const CARD_PADDING: f32 = 24.0;
/// Gap between the headline, the supporting text and the action row, in dp.
const CARD_CONTENT_SPACING: f32 = 16.0;
/// Extra gap the M3 spec asks between supporting text and the action row
/// (`body` → `actions` is 24 dp, on top of the 16 dp row spacing), in dp.
const ACTIONS_TOP_PADDING: f32 = 8.0;
/// Gap between adjacent action buttons, in dp.
const ACTIONS_SPACING: f32 = 8.0;

/// The role a [`DialogAction`] carries: how it reads and where it sits.
///
/// Ordering inside the action row follows the platform convention; declaration
/// order is kept within each role. At most one `Cancel` action is allowed —
/// more than one fails at render. The first `Default` action is the primary
/// one and is bound to `Return` on platforms with a hardware Return key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogRole {
    /// An ordinary confirmation action (`Save`, `Paste`). The first declared
    /// one is the primary action.
    Default,
    /// The dismissive answer. Escape, the Android back gesture and the
    /// Hydrolysis scrim tap all run the `Cancel` action's handler. A dialog
    /// with no `Cancel` action must be answered — the cancel path does
    /// nothing.
    Cancel,
    /// A destructive confirmation (`Delete`, `Discard`), drawn with the
    /// `Error` emphasis on Hydrolysis and `hasDestructiveAction` / `.destructive`
    /// on Apple.
    Destructive,
}

/// One action button of a [`Dialog`]: a title, a role and the handler that
/// runs when the user chooses it.
///
/// Titles are plain text — native alert buttons take a title only, so this
/// accepts [`IntoText`], not `IntoLabel` (an icon would be silently dropped on
/// Apple).
#[derive(Clone)]
pub struct DialogAction {
    title: Text,
    role: DialogRole,
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
    /// Creates an action with [`DialogRole::Default`].
    pub fn new<H, Args>(title: impl IntoText, action: H) -> Self
    where
        H: Handler<Args, ()> + 'static,
    {
        Self {
            title: title.into_text(),
            role: DialogRole::Default,
            action: shared_action(action),
        }
    }

    /// Creates an action with [`DialogRole::Destructive`].
    pub fn destructive<H, Args>(title: impl IntoText, action: H) -> Self
    where
        H: Handler<Args, ()> + 'static,
    {
        Self {
            title: title.into_text(),
            role: DialogRole::Destructive,
            action: shared_action(action),
        }
    }

    /// Creates an action with [`DialogRole::Cancel`].
    pub fn cancel<H, Args>(title: impl IntoText, action: H) -> Self
    where
        H: Handler<Args, ()> + 'static,
    {
        Self {
            title: title.into_text(),
            role: DialogRole::Cancel,
            action: shared_action(action),
        }
    }

    /// The button title.
    pub const fn title(&self) -> &Text {
        &self.title
    }

    /// The role this action carries.
    #[must_use]
    pub const fn role(&self) -> DialogRole {
        self.role
    }

    /// The handler chosen by pressing this action.
    #[must_use]
    pub const fn action(&self) -> &SharedAction {
        &self.action
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
///
/// `Dialog` also implements [`View`]: rendered directly it produces the
/// composed card — the `Scrim` backdrop plus the alert card — which is what
/// the Hydrolysis modal layer draws.
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
    pub fn action(mut self, action: DialogAction) -> Self {
        self.actions.push(action);
        self
    }

    /// The presentation binding the dialog was created with.
    #[must_use]
    pub const fn is_presented(&self) -> &Binding<bool> {
        &self.is_presented
    }

    /// The dialog's headline.
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
            .find(|action| action.role == DialogRole::Cancel)
    }

    /// The action list as renderers must present it: roles in platform
    /// convention order — dismissive first, affirmative trailing — with
    /// declaration order kept within each role, and a single platform-worded
    /// acknowledgement button with role `Cancel` when the declaration was
    /// empty (the same default `NSAlert` draws).
    ///
    /// # Panics
    ///
    /// Panics when more than one `Cancel` action was declared — validation
    /// fails fast at render, not at first presentation.
    #[must_use]
    pub fn resolved_actions(&self) -> Vec<DialogAction> {
        assert!(
            self.actions
                .iter()
                .filter(|action| action.role == DialogRole::Cancel)
                .count()
                <= 1,
            "Dialog declares more than one Cancel action"
        );
        if self.actions.is_empty() {
            return vec![DialogAction::cancel("OK", || {})];
        }
        let mut ordered = Vec::with_capacity(self.actions.len());
        ordered.extend(
            self.actions
                .iter()
                .filter(|action| action.role == DialogRole::Cancel)
                .cloned(),
        );
        ordered.extend(
            self.actions
                .iter()
                .filter(|action| action.role == DialogRole::Destructive)
                .cloned(),
        );
        ordered.extend(
            self.actions
                .iter()
                .filter(|action| action.role == DialogRole::Default)
                .cloned(),
        );
        ordered
    }

    /// The shared "chose this action" step: run the handler, then write the
    /// presentation binding back — the backend's half of the action contract.
    ///
    /// This is the backend-facing entry point: `components` call it instead of
    /// touching the handler directly so every realization answers identically.
    #[doc(hidden)]
    pub fn run_action(&self, action: &DialogAction, env: &Environment) {
        let () = action.action.call(env);
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

/// One dialog action as a card button.
///
/// The primary action (the first `Default`) draws with the `Accent` emphasis;
/// `Destructive` draws with `Error`; everything else keeps the theme's default
/// text colour.
fn action_button(action: DialogAction, dialog: &Dialog, primary: bool) -> AnyView {
    let label = match action.role() {
        DialogRole::Destructive => text(action.title().clone()).color(Color::new(Error)),
        _ if primary => text(action.title().clone()).color(Color::new(Accent)),
        _ => text(action.title().clone()),
    };
    let dialog = dialog.clone();
    AnyView::new(
        button(label)
            .style(ButtonStyle::Plain)
            .action(move |env: Environment| dialog.run_action(&action, &env)),
    )
}

/// The alert card the Hydrolysis modal layer draws: `Surface` container,
/// `Foreground` title, `MutedForeground` message, and the action row —
/// `Accent` primary, `Error` destructive — carrying the `Dialog`
/// accessibility role.
fn dialog_card(dialog: &Dialog) -> impl View + use<> {
    let actions = dialog.resolved_actions();
    let mut seen_default = false;
    let buttons: Vec<AnyView> = actions
        .into_iter()
        .map(|action| {
            let primary = action.role() == DialogRole::Default && !seen_default;
            seen_default |= action.role() == DialogRole::Default;
            action_button(action, dialog, primary)
        })
        .collect();
    // Bound to Return: the first Default action is the primary one. It runs on
    // the key a focused dialog control leaves unconsumed.
    let primary = dialog
        .actions
        .iter()
        .find(|action| action.role() == DialogRole::Default)
        .cloned();

    let mut contents: Vec<AnyView> = vec![AnyView::new(
        text(dialog.title.clone()).color(Color::new(Foreground)),
    )];
    if let Some(message) = &dialog.message {
        contents.push(AnyView::new(
            text(message.clone()).color(Color::new(MutedForeground)),
        ));
    }
    // The action row hugs the trailing edge (M3): a trailing-aligned full-width
    // frame carries the content-sized button stack.
    contents.push(AnyView::new(
        Frame::new(
            hstack(buttons)
                .spacing(ACTIONS_SPACING)
                .padding_with(EdgeInsets::new(ACTIONS_TOP_PADDING, 0.0, 0.0, 0.0)),
        )
        .max_width(f32::INFINITY)
        .alignment(alignment::Trailing),
    ));

    let card = dialog.clone();
    Frame::new(
        vstack(contents)
            .spacing(CARD_CONTENT_SPACING)
            .padding_with(EdgeInsets::all(CARD_PADDING))
            .background(FixedRoundedRectangle::new(CARD_CORNER_RADIUS).fill(Color::new(Surface)))
            .a11y_role(AccessibilityRole::Dialog),
    )
    .min_width(CARD_MIN_WIDTH)
    .max_width(CARD_MAX_WIDTH)
    .on_key_press(move |Use(press): Use<KeyPress>, env: Environment| {
        if press.key == Key::Named(NamedKey::Enter)
            && let Some(primary) = &primary
        {
            card.run_action(primary, &env);
            return KeyHandling::Handled;
        }
        KeyHandling::Ignored
    })
}

impl View for Dialog {
    /// The self-drawn realization: a full-window `Scrim` backdrop — tapping it
    /// runs the cancel path — with the alert card centered above it.
    fn body(self, _env: &Environment) -> impl View {
        let scrim_dialog = self.clone();
        zstack((
            Color::new(Scrim).on_tap(move |env: Environment| scrim_dialog.run_cancel(&env)),
            dialog_card(&self),
        ))
    }
}
