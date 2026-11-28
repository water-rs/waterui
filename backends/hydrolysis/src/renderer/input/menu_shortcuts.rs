//! `Command::shortcut` and `Button::keyboard_shortcut` chord dispatch
//! (water-rs/hydrolysis#247, water-rs/waterui#1245, water-rs/waterui#4456).
//!
//! A mounted `Menu` registers the chords of every `Command` carrying a
//! `shortcut`, and a mounted `Button` carrying `.keyboard_shortcut(...)`
//! registers its chord on the same app-shared [`MenuShortcutRegistry`] while
//! mounted. A `.context_menu`'s commands register on open and are live only
//! while the menu is open. The runner owns the registry and seeds it into the
//! environment it hands every window — winit, headless, and test runners alike
//! — so a popup window holding keyboard focus resolves the same table. The
//! registry is consulted before the key-dispatch modifier early return and
//! before the focused text input sees the key; a matching chord claims the
//! event, a disabled command or button claims it without firing, and the most
//! recently registered source wins a conflict. A press without Control, Alt or
//! Super reaches the registry only while no text editor holds focus — while
//! one does the press is typing, so a bare character or named-key chord leaves
//! the field its key (water-rs/waterui#2118).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use nami::{Computed, Signal as _};
#[cfg(feature = "menu")]
use waterui_backend_core::app::Quit;
#[cfg(feature = "menu")]
use waterui_controls::NoopSlot;
#[cfg(feature = "menu")]
use waterui_controls::menu::{CloseWindowChord, MenuItem, ResolvedCommand, ResolvedMenuItem};
use waterui_controls::shortcut::Shortcut;
use waterui_core::Environment;
use waterui_core::Str;
use waterui_core::contract_violation;
#[cfg(feature = "menu")]
use waterui_core::extract::State;
use waterui_core::handler::ReportExtraction as _;
#[cfg(feature = "menu")]
use waterui_core::handler::SharedAction;

#[cfg(feature = "menu")]
use crate::renderer::HeldChild;
#[cfg(feature = "menu")]
use crate::renderer::call_action_discarding_result;
use crate::widgets::controls::button::ButtonRenderState;
#[cfg(feature = "menu")]
use crate::widgets::controls::button::MenuRenderState;

use super::WindowId;
#[cfg(feature = "menu")]
use super::popup_menu::{PopupMenuNode, PopupMenuStateGroup};
use crate::HydrolysisRenderer;
use crate::platform::Modifiers;

/// Converts Hydrolysis's platform modifier struct into W3C [`Modifiers`](waterui_interaction::key::Modifiers)
/// so chord matching goes through [`Shortcut::matches_key_and_modifiers`], the
/// single source of truth for platform command-key mapping.
const fn to_w3c_modifiers(modifiers: Modifiers) -> waterui_interaction::key::Modifiers {
    let mut flags = waterui_interaction::key::Modifiers::empty();
    if modifiers.control {
        flags = flags.union(waterui_interaction::key::Modifiers::CONTROL);
    }
    if modifiers.alt {
        flags = flags.union(waterui_interaction::key::Modifiers::ALT);
    }
    if modifiers.shift {
        flags = flags.union(waterui_interaction::key::Modifiers::SHIFT);
    }
    if modifiers.super_key {
        flags = flags.union(waterui_interaction::key::Modifiers::META);
    }
    flags
}

/// The action triggered when a registered shortcut fires: either a menu
/// command's [`SharedAction`] or a mounted button's retained render state.
enum ShortcutTarget<E: crate::text::TextEngine> {
    #[cfg(feature = "menu")]
    Command(SharedAction<()>),
    Button(Weak<RefCell<ButtonRenderState<E>>>),
}

impl<E: crate::text::TextEngine> Clone for ShortcutTarget<E> {
    fn clone(&self) -> Self {
        match self {
            #[cfg(feature = "menu")]
            Self::Command(action) => Self::Command(action.clone()),
            Self::Button(state) => Self::Button(Weak::clone(state)),
        }
    }
}

/// One armed chord: the [`Shortcut`] it matches, its target action, its live
/// disabled flag, the label a conflict warning names it by, and the rendered
/// hint text.
struct MenuShortcut<E: crate::text::TextEngine> {
    shortcut: Shortcut,
    target: ShortcutTarget<E>,
    disabled: Computed<bool>,
    label: Str,
    hint: Str,
}

impl<E: crate::text::TextEngine> Clone for MenuShortcut<E> {
    fn clone(&self) -> Self {
        Self {
            shortcut: self.shortcut.clone(),
            target: self.target.clone(),
            disabled: self.disabled.clone(),
            label: self.label.clone(),
            hint: self.hint.clone(),
        }
    }
}

impl<E: crate::text::TextEngine> MenuShortcut<E> {
    #[cfg(feature = "menu")]
    fn command(
        shortcut: &Shortcut,
        action: SharedAction<()>,
        disabled: Computed<bool>,
        label: Str,
    ) -> Self {
        Self {
            shortcut: shortcut.clone(),
            target: ShortcutTarget::Command(action),
            disabled,
            label,
            hint: shortcut.to_string().into(),
        }
    }

    fn button(
        shortcut: &Shortcut,
        button_state: Weak<RefCell<ButtonRenderState<E>>>,
        disabled: Computed<bool>,
        label: Str,
    ) -> Self {
        Self {
            shortcut: shortcut.clone(),
            target: ShortcutTarget::Button(button_state),
            disabled,
            label,
            hint: shortcut.to_string().into(),
        }
    }

    fn matches(&self, key: &keyboard_types::Key, pressed: Modifiers) -> bool {
        self.shortcut
            .matches_key_and_modifiers(key, to_w3c_modifiers(pressed))
    }
}

/// The contract-violation text for a renderer resolving menu chords in an
/// environment no runner prepared: registering and dispatching both name it,
/// so the missing seed names the runner that skipped it.
pub const MISSING_MENU_SHORTCUT_REGISTRY: &str = "menu shortcuts require the runner to seed a MenuShortcutRegistry into the window's \
     environment — the winit runner, headless run and HeadlessRuntime, SemanticRuntime and \
     the web runner all install one; a renderer built outside a runner seeds its own";

/// The [`MenuShortcutRegistry`] the runner seeded into `env`, or `None` after
/// a contract violation when it seeded none; `outcome` states what the caller
/// does without it.
pub fn menu_shortcut_registry<'env, E: crate::text::TextEngine>(
    env: &'env Environment,
    outcome: &str,
) -> Option<&'env MenuShortcutRegistry<E>> {
    let registry = env.get::<MenuShortcutRegistry<E>>();
    if registry.is_none() {
        contract_violation!("{MISSING_MENU_SHORTCUT_REGISTRY}; {outcome}");
    }
    registry
}

/// The window-close primitive a declared `MenuItem::CloseWindow` stands
/// for: handed a [`WindowId`], it asks that window to close through the
/// window's ordinary close path, so closing the last one still follows the
/// application's `LastWindowPolicy`. Which window is handed over is the
/// dispatch's business — a chord's dispatching window, a popup row's owner
/// — never a tracked "key window".
///
/// Only a runner whose windows the application can close installs one — the
/// winit desktop runner. Hosts whose windows the system or the host owns
/// (Android, the web, the headless and semantic runtimes) install none, and
/// a declared Close Window is omitted there.
#[derive(Clone)]
pub struct WindowCloser(Rc<dyn Fn(WindowId)>);

impl std::fmt::Debug for WindowCloser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowCloser").finish_non_exhaustive()
    }
}

impl WindowCloser {
    /// A primitive that runs `close` with the window to close. Only runners
    /// with closable windows build one.
    #[cfg(any(hydrolysis_closable_windows, test))]
    pub fn new(close: impl Fn(WindowId) + 'static) -> Self {
        Self(Rc::new(close))
    }

    /// Asks `window` to close.
    pub fn request(&self, window: WindowId) {
        (self.0)(window);
    }
}

/// The command a declared `MenuItem::CloseWindow` stands for wherever
/// Hydrolysis renders it as a command — a popup-menu row, a chord in the
/// shortcut table: the framework's [`MenuItem::close_window_command`]
/// (its label and the application's decided close chord —
/// [`CloseWindowChord`], the one `App::into_parts` installs over the
/// resolved menu bar, so a mounted or context menu carries the same chord
/// as the bar: ⌘W on macOS and Ctrl+W elsewhere through
/// [`ChordModifiers`]' command→control mapping) acting through the runner's
/// [`WindowCloser`].
///
/// The window the command acts on rides in the action's environment: the
/// chord registry injects the window it dispatches for, and a popup menu
/// injects its owner window's identity when the menu is built.
/// `owner_closable` is the owner window's `closable` — fixed at mount — so
/// a popup row built for a window without a close button reads disabled;
/// the shared close path still re-checks it. `None` when `env` carries no
/// [`WindowCloser`]: the host's windows are not the application's to
/// close, and the item is omitted.
#[cfg(feature = "menu")]
pub fn close_window_command(env: &Environment, owner_closable: bool) -> Option<ResolvedCommand> {
    let close = env.get::<WindowCloser>()?.clone();
    let mut command = MenuItem::close_window_command::<waterui_controls::NoopSlot, _>(
        env,
        CloseWindowChord::of(env),
        move |window: WindowId| {
            close.request(window);
        },
    );
    if !owner_closable {
        command.set_disabled(Computed::constant(true));
    }
    Some(command)
}

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    /// (Re)register a mounted `Menu` trigger's chords on the app's shared
    /// registry, scoped to this renderer's window — the trigger's render
    /// calls it every flush, so a source the flush no longer visits drops
    /// out with its render state.
    #[cfg(feature = "menu")]
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    pub(crate) fn register_menu_shortcuts(
        &mut self,
        menu_state: Weak<RefCell<MenuRenderState<E>>>,
        items: Computed<Vec<ResolvedMenuItem<HeldChild<E>>>>,
        env: Environment,
    ) {
        let Some(registry) =
            menu_shortcut_registry::<E>(&env, "the menu's shortcuts are not registered").cloned()
        else {
            return;
        };
        registry.register_mounted(self.window_id, menu_state, items, env);
    }

    /// (Re)register a mounted `Button`'s keyboard shortcut on the app's shared
    /// registry, scoped to this renderer's window and modal layer.
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    pub(crate) fn register_button_shortcut(
        &mut self,
        button_state: Weak<RefCell<ButtonRenderState<E>>>,
        shortcut: Shortcut,
        disabled: Computed<bool>,
        label: Str,
        env: Environment,
    ) {
        let Some(registry) =
            menu_shortcut_registry::<E>(&env, "the button's shortcut is not registered").cloned()
        else {
            return;
        };
        registry.register_button(self.window_id, button_state, shortcut, disabled, label, env);
    }
}

/// The commands of a `Menu` resolve into shortcut entries through the items
/// signal, so a menu whose items change mounts its new chords on the next
/// lookup without any registration churn. A declared Quit arms the chord of
/// [`Quit::command`], and nothing where `env` has no application quit; a
/// declared Close Window arms the application's decided chord of
/// [`close_window_command`], and nothing where the host's windows cannot be
/// closed.
#[cfg(feature = "menu")]
fn collect_menu_shortcuts<E: crate::text::TextEngine, S: 'static>(
    items: &[ResolvedMenuItem<S>],
    env: &Environment,
    out: &mut Vec<MenuShortcut<E>>,
) {
    for item in items {
        match item {
            ResolvedMenuItem::Command(command) => collect_command_shortcut(command, out),
            ResolvedMenuItem::Quit => {
                if let Some(command) = env
                    .extract::<Quit>()
                    .ok()
                    .map(|quit| quit.command::<NoopSlot>(env))
                {
                    collect_command_shortcut(&command, out);
                }
            }
            ResolvedMenuItem::CloseWindow => {
                // The mounted/app-bar chord acts on the dispatching window,
                // which the registry injects at dispatch time — whether that
                // window is closable is the shared close path's check, not
                // this chord's, so the command always builds enabled.
                if let Some(command) = close_window_command(env, true) {
                    collect_command_shortcut(&command, out);
                }
            }
            ResolvedMenuItem::Menu(menu) => {
                collect_menu_shortcuts(&menu.items().snapshot(), env, out);
            }
            ResolvedMenuItem::Divider => {}
        }
    }
}

#[cfg(feature = "menu")]
fn collect_command_shortcut<E: crate::text::TextEngine, S>(
    command: &ResolvedCommand<S>,
    out: &mut Vec<MenuShortcut<E>>,
) {
    if let Some(shortcut) = command.shortcut() {
        out.push(MenuShortcut::command(
            shortcut,
            command.action().clone(),
            command.is_disabled().clone(),
            command.label().content().snapshot().to_plain(),
        ));
    }
}

/// The same flattening for a built popup's nodes — context-menu nodes carry
/// the shortcut the command was resolved with.
#[cfg(feature = "menu")]
fn collect_popup_shortcuts<E: crate::text::TextEngine>(
    nodes: &[PopupMenuNode],
    out: &mut Vec<MenuShortcut<E>>,
) {
    for node in nodes {
        match node {
            PopupMenuNode::Command {
                shortcut: Some(shortcut),
                action,
                disabled,
                plain_label,
                ..
            } => {
                out.push(MenuShortcut::command(
                    shortcut,
                    action.clone(),
                    disabled.clone(),
                    plain_label.clone().into(),
                ));
            }
            PopupMenuNode::Menu { items, .. } => collect_popup_shortcuts(items, out),
            _ => {}
        }
    }
}

/// A source of chords: a mounted `Menu` trigger, a mounted `Button` with
/// `.keyboard_shortcut(..)`, an open popup menu, or the application's
/// `menu_bar`. `seq` orders sources by registration, so the most recently
/// registered one wins a chord conflict however the windows interleave.
enum MenuShortcutSource<E: crate::text::TextEngine> {
    #[cfg(feature = "menu")]
    Mounted {
        /// The window the chords dispatch on — the one the `Menu` mounted in.
        window: WindowId,
        /// The `Menu` trigger's render state: the source's identity —
        /// `Weak::ptr_eq` against a live upgrade — and its liveness (unmount
        /// drops the render state, and the source with it).
        menu_state: Weak<RefCell<MenuRenderState<E>>>,
        items: Computed<Vec<ResolvedMenuItem<HeldChild<E>>>>,
        env: Environment,
        seq: u64,
    },
    Button {
        /// The window the button is mounted in.
        window: WindowId,
        /// The button's retained render state: identity and mount liveness.
        button_state: Weak<RefCell<ButtonRenderState<E>>>,
        shortcut: Shortcut,
        disabled: Computed<bool>,
        label: Str,
        env: Environment,
        seq: u64,
    },
    #[cfg(feature = "menu")]
    Popup {
        shortcuts: Vec<MenuShortcut<E>>,
        group: PopupMenuStateGroup,
        env: Environment,
        seq: u64,
    },
    /// The application-level `App::menu_bar`: resolved to items once by the
    /// runner and kept reactive, so item edits apply on the next lookup.
    #[cfg(feature = "menu")]
    AppBar {
        items: Computed<Vec<ResolvedMenuItem>>,
        env: Environment,
        seq: u64,
    },
}

impl<E: crate::text::TextEngine> MenuShortcutSource<E> {
    const fn seq(&self) -> u64 {
        match self {
            #[cfg(feature = "menu")]
            Self::Mounted { seq, .. } | Self::Popup { seq, .. } | Self::AppBar { seq, .. } => *seq,
            Self::Button { seq, .. } => *seq,
        }
    }

    fn live(&self) -> bool {
        match self {
            #[cfg(feature = "menu")]
            Self::Mounted { menu_state, .. } => menu_state.upgrade().is_some(),
            Self::Button { button_state, .. } => button_state.upgrade().is_some(),
            #[cfg(feature = "menu")]
            Self::Popup { group, .. } => group.is_live(),
            // The menu bar lives for the app's duration — the same span the
            // runner owns this registry for.
            #[cfg(feature = "menu")]
            Self::AppBar { .. } => true,
        }
    }

    fn collect_entries(
        &self,
        window: WindowId,
        modal_active: bool,
        out: &mut Vec<MenuShortcut<E>>,
    ) {
        match self {
            #[cfg(feature = "menu")]
            Self::Mounted {
                window: mounted_window,
                items,
                env,
                ..
            } => {
                if *mounted_window == window {
                    collect_menu_shortcuts(&items.snapshot(), env, out);
                }
            }
            Self::Button {
                window: button_window,
                button_state,
                shortcut,
                disabled,
                label,
                env: button_env,
                ..
            } => {
                let in_modal = button_env
                    .get::<crate::theme::ModalInteraction>()
                    .is_some_and(crate::theme::ModalInteraction::is_active);
                if *button_window == window && in_modal == modal_active {
                    out.push(MenuShortcut::button(
                        shortcut,
                        Weak::clone(button_state),
                        disabled.clone(),
                        label.clone(),
                    ));
                }
            }
            #[cfg(feature = "menu")]
            Self::Popup { shortcuts, .. } => {
                out.clone_from(shortcuts);
            }
            #[cfg(feature = "menu")]
            Self::AppBar { items, env, .. } => {
                collect_menu_shortcuts(&items.snapshot(), env, out);
            }
        }
    }
}

/// The app's menu-shortcut registry, owned by the runner and seeded into the
/// environment it hands every window — winit, headless, and test runners
/// alike, so the chords a test resolves take the same path as a live app. It
/// holds every chord source — mounted `Menu` triggers keyed by the window
/// that mounted them, and open popup menus — plus the counter that orders
/// them by registration (water-rs/hydrolysis#247).
pub struct MenuShortcutRegistry<E: crate::text::TextEngine>(
    Rc<RefCell<MenuShortcutRegistryState<E>>>,
);

struct MenuShortcutRegistryState<E: crate::text::TextEngine> {
    sources: Vec<MenuShortcutSource<E>>,
    /// Registration order across every window — a chord conflict resolves to
    /// the most recently registered source.
    next_seq: u64,
}

impl<E: crate::text::TextEngine> MenuShortcutRegistryState<E> {
    const fn next_source_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }
}

impl<E: crate::text::TextEngine> MenuShortcutRegistry<E> {
    /// (Re)register a mounted `Menu` trigger's chords — called from the
    /// trigger's render on every flush; a live source just refreshes its
    /// items signal and environment, keeping its registration order.
    #[cfg(feature = "menu")]
    fn register_mounted(
        &self,
        window: WindowId,
        menu_state: Weak<RefCell<MenuRenderState<E>>>,
        items: Computed<Vec<ResolvedMenuItem<HeldChild<E>>>>,
        env: Environment,
    ) {
        let mut inner = self.0.borrow_mut();
        inner.sources.retain(MenuShortcutSource::live);
        if let Some(slot) = inner.sources.iter_mut().find(|source| {
            matches!(source, MenuShortcutSource::Mounted { menu_state: existing, .. } if existing.ptr_eq(&menu_state))
        }) {
            *slot = MenuShortcutSource::Mounted {
                window,
                menu_state,
                items,
                env,
                seq: slot.seq(),
            };
        } else {
            let seq = inner.next_source_seq();
            inner.sources.push(MenuShortcutSource::Mounted {
                window,
                menu_state,
                items,
                env,
                seq,
            });
        }
    }

    /// (Re)register a mounted `Button`'s shortcut — called from the button's
    /// render on every flush; a live source refreshes its disabled signal,
    /// label, and environment while keeping its registration order.
    fn register_button(
        &self,
        window: WindowId,
        button_state: Weak<RefCell<ButtonRenderState<E>>>,
        shortcut: Shortcut,
        disabled: Computed<bool>,
        label: Str,
        env: Environment,
    ) {
        let mut inner = self.0.borrow_mut();
        inner.sources.retain(MenuShortcutSource::live);
        if let Some(slot) = inner.sources.iter_mut().find(|source| {
            matches!(source, MenuShortcutSource::Button { button_state: existing, .. } if existing.ptr_eq(&button_state))
        }) {
            *slot = MenuShortcutSource::Button {
                window,
                button_state,
                shortcut,
                disabled,
                label,
                env,
                seq: slot.seq(),
            };
        } else {
            let seq = inner.next_source_seq();
            inner.sources.push(MenuShortcutSource::Button {
                window,
                button_state,
                shortcut,
                disabled,
                label,
                env,
                seq,
            });
        }
    }

    /// Register the application `menu_bar`'s chords — live for the app's
    /// duration and answering to whichever window dispatches, since app-level
    /// commands are not scoped to one window. The runner resolves the menus
    /// once (`resolve_menu_bar_items` keeps the signal reactive, so item
    /// edits apply on the next lookup) and registers once; re-registration
    /// replaces the source rather than stacking it.
    // Only windowed runners register menus — bare wasm compiles the impl
    // for the rest of its surface and has no caller for this one.
    #[cfg(feature = "menu")]
    #[cfg_attr(all(target_arch = "wasm32", not(feature = "web")), allow(dead_code))]
    pub(crate) fn register_menu_bar(
        &self,
        items: Computed<Vec<ResolvedMenuItem>>,
        env: Environment,
    ) {
        let mut inner = self.0.borrow_mut();
        inner
            .sources
            .retain(|source| !matches!(source, MenuShortcutSource::AppBar { .. }));
        let seq = inner.next_source_seq();
        inner
            .sources
            .push(MenuShortcutSource::AppBar { items, env, seq });
    }

    /// Register an open menu's chords — live while any menu in `group` is
    /// open (the `.context_menu` contract). Popup sources are not window
    /// scoped: a menu that opened as its own window may hold keyboard focus,
    /// and the keys land in that window's dispatch.
    #[cfg(feature = "menu")]
    pub(crate) fn register_popup(
        &self,
        nodes: &[PopupMenuNode],
        group: &PopupMenuStateGroup,
        env: &Environment,
    ) {
        let mut shortcuts = Vec::new();
        collect_popup_shortcuts(nodes, &mut shortcuts);
        if shortcuts.is_empty() {
            return;
        }
        let mut inner = self.0.borrow_mut();
        let seq = inner.next_source_seq();
        inner.sources.push(MenuShortcutSource::Popup {
            shortcuts,
            group: group.clone(),
            env: env.clone(),
            seq,
        });
    }

    /// Dispatch a pressed key — its W3C `KeyboardEvent.key` — for `window`
    /// against the registry. Returns `true` when a chord matched and claimed
    /// the event — whether or not its target fired (disabled targets claim
    /// without firing).
    pub(crate) fn dispatch(
        &self,
        window: WindowId,
        pressed: &keyboard_types::Key,
        modifiers: Modifiers,
        modal_active: bool,
        env: &Environment,
    ) -> bool {
        #[cfg(feature = "menu")]
        let mut winner: Option<(
            MenuShortcut<E>,
            Option<PopupMenuStateGroup>,
            Environment,
        )> = None;
        #[cfg(not(feature = "menu"))]
        let mut winner: Option<(MenuShortcut<E>, Option<()>, Environment)> = None;
        let mut conflicts: Vec<Str> = Vec::new();
        {
            let mut inner = self.0.borrow_mut();
            inner.sources.retain(MenuShortcutSource::live);
            inner
                .sources
                .sort_unstable_by_key(|source| std::cmp::Reverse(source.seq()));
            for source in &inner.sources {
                let mut entries = Vec::new();
                source.collect_entries(window, modal_active, &mut entries);
                for entry in entries {
                    if entry.matches(pressed, modifiers) {
                        if winner.is_some() {
                            conflicts.push(entry.label);
                        } else {
                            let (group, source_env) = match source {
                                #[cfg(feature = "menu")]
                                MenuShortcutSource::Mounted { env, .. }
                                | MenuShortcutSource::AppBar { env, .. } => (None, env.clone()),
                                MenuShortcutSource::Button { env, .. } => (None, env.clone()),
                                #[cfg(feature = "menu")]
                                MenuShortcutSource::Popup { group, env, .. } => {
                                    (Some(group.clone()), env.clone())
                                }
                            };
                            winner = Some((entry, group, source_env));
                        }
                    }
                }
            }
        }
        let Some((shortcut, group, source_env)) = winner else {
            return false;
        };
        if !conflicts.is_empty() {
            tracing::warn!(
                chord = %shortcut.hint,
                winner = %shortcut.label,
                losers = ?conflicts,
                "shortcut chord registered by multiple controls; the most recently mounted wins",
            );
        }
        // A disabled command or button still claims its chord — the keystroke
        // belongs to the control, it just does nothing.
        if shortcut.disabled.snapshot() {
            return true;
        }
        // A `WindowId` a command's action extracts names the window it acts
        // on: mounted and app-bar chords act on the window the chord
        // dispatched for, so that window rides in the action's environment.
        // A popup source's rows carry their owner window's id already — it
        // is the row's target, never the window the chord happened to
        // dispatch on.
        let action_env = source_env.layered_on(env);
        #[cfg(feature = "menu")]
        let action_env = if group.is_none() {
            action_env.extending(State(window))
        } else {
            action_env
        };
        #[cfg(feature = "menu")]
        if let Some(group) = group {
            group.close_all();
        }
        #[cfg(not(feature = "menu"))]
        let _ = group;
        match shortcut.target {
            #[cfg(feature = "menu")]
            ShortcutTarget::Command(action) => {
                call_action_discarding_result(&action, &action_env);
            }
            ShortcutTarget::Button(button_state) => {
                if let Some(state) = button_state.upgrade() {
                    (state.borrow_mut().config.action_mut())(&action_env).report_extraction();
                }
            }
        }
        true
    }
}

impl<E: crate::text::TextEngine> Default for MenuShortcutRegistryState<E> {
    fn default() -> Self {
        Self {
            sources: Vec::default(),
            next_seq: Default::default(),
        }
    }
}

impl<E: crate::text::TextEngine> Default for MenuShortcutRegistry<E> {
    fn default() -> Self {
        Self(Rc::default())
    }
}

impl<E: crate::text::TextEngine> Clone for MenuShortcutRegistry<E> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}
