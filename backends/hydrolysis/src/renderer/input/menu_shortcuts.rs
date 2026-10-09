//! `Command::shortcut` chord dispatch
//! (water-rs/hydrolysis#247, water-rs/waterui#1245).
//!
//! A `Menu` mounted in a window registers the chords of every `Command`
//! carrying a `shortcut` on the app's shared [`MenuShortcutRegistry`] while it
//! is mounted — regardless of whether its popup is open, matching how a
//! desktop menu bar's commands stay armed. A `.context_menu`'s commands
//! register on open and are live only while the menu is open. The runner owns
//! the registry and seeds it into the environment it hands every window —
//! winit, headless, and test runners alike — so a popup window holding
//! keyboard focus resolves the same table. The registry is consulted before
//! the key-dispatch modifier early return and before the focused text input
//! sees the key; a matching chord claims the event, a disabled command claims
//! it without firing, and the most recently registered source wins a
//! conflict. A press without Control, Alt or Super reaches the registry only
//! while no text editor holds focus — while one does the press is typing, so
//! a bare character or named-key chord leaves the field its key
//! (water-rs/waterui#2118).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use nami::{Computed, Signal as _};
use waterui::app::Quit;
use waterui::window::WindowState;
use waterui_controls::menu::{
    CloseWindowChord, MenuItem, ResolvedCommand, ResolvedMenuItem, Shortcut, ShortcutKey,
};
use waterui_core::Environment;
use waterui_core::Str;
use waterui_core::handler::SharedAction;
use waterui_core::impl_extractor;

use super::popup_menu::{PopupMenuNode, PopupMenuStateGroup};
use crate::HydrolysisRenderer;
use crate::platform::Modifiers;
use crate::renderer::call_action_discarding_result;
use crate::widgets::controls::button::MenuRenderState;

/// The normalized modifier set of a `Shortcut`: the command modifier is the
/// platform menu accelerator — super on macOS, control elsewhere.
// A fixed four-flag mirror of the platform modifier set, not a state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
struct ChordModifiers {
    control: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
}

impl ChordModifiers {
    const fn of(shortcut: &Shortcut) -> Self {
        let modifiers = shortcut.modifiers;
        #[cfg(target_os = "macos")]
        {
            Self {
                control: modifiers.control(),
                alt: modifiers.option(),
                shift: modifiers.shift(),
                super_key: modifiers.command(),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self {
                control: modifiers.control() || modifiers.command(),
                alt: modifiers.option(),
                shift: modifiers.shift(),
                super_key: false,
            }
        }
    }
}

/// One command's chord: the normalized modifier set and the key, the action
/// it runs, its live disabled flag, the label a conflict warning names
/// it by, and the rendered hint text.
#[derive(Clone)]
struct MenuShortcut {
    modifiers: ChordModifiers,
    key: ShortcutKey,
    action: SharedAction<()>,
    disabled: Computed<bool>,
    label: Str,
    hint: Str,
}

impl MenuShortcut {
    fn new(
        shortcut: &Shortcut,
        action: SharedAction<()>,
        disabled: Computed<bool>,
        label: Str,
    ) -> Self {
        Self {
            modifiers: ChordModifiers::of(shortcut),
            key: shortcut.key.clone(),
            action,
            disabled,
            label,
            hint: shortcut.to_string().into(),
        }
    }

    fn matches(&self, key: &keyboard_types::Key, pressed: Modifiers) -> bool {
        self.modifiers.control == pressed.control
            && self.modifiers.alt == pressed.alt
            && self.modifiers.shift == pressed.shift
            && self.modifiers.super_key == pressed.super_key
            && self.key.matches(key)
    }
}

/// The panic text for a renderer resolving menu chords in an environment no
/// runner prepared: registering and dispatching both name it, so the missing
/// seed names the runner that skipped it.
pub const MISSING_MENU_SHORTCUT_REGISTRY: &str = "menu shortcuts require the runner to seed a MenuShortcutRegistry into the window's \
     environment — the winit runner, headless run and HeadlessRuntime, SemanticRuntime and \
     the web runner all install one; a renderer built outside a runner seeds its own";

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
pub fn close_window_command(env: &Environment, owner_closable: bool) -> Option<ResolvedCommand> {
    let close = env.get::<WindowCloser>()?.clone();
    let mut command =
        MenuItem::close_window_command(env, CloseWindowChord::of(env), move |window: WindowId| {
            close.request(window);
        });
    if !owner_closable {
        command.disabled = Computed::constant(true);
    }
    Some(command)
}

/// A window's identity for menu-chord dispatch: the runner assigns one when it
/// creates the window and hands it to the window's renderer, so the shared
/// [`MenuShortcutRegistry`] can scope a mounted `Menu`'s chords to the window
/// they mounted in. Unlike a pointer address, it cannot drift when the
/// renderer moves nor alias a recycled allocation (water-rs/hydrolysis#247).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowId {
    /// A winit window — its native `winit::window::WindowId`.
    #[cfg(hydrolysis_winit)]
    Winit(winit::window::WindowId),
    /// A window on a runner without a native window id — an ordinal the
    /// registry hands out in window-creation order.
    Runner(u64),
    /// A renderer no runner claimed — embedded GPU hosts and renderers built
    /// directly in tests. Their mounted sources answer only to their own
    /// dispatch.
    Orphan,
}

impl WindowId {
    /// The winit window this id names, when it names one.
    #[cfg(hydrolysis_winit)]
    pub(crate) const fn as_winit(self) -> Option<winit::window::WindowId> {
        match self {
            Self::Winit(id) => Some(id),
            Self::Runner(_) | Self::Orphan => None,
        }
    }
}

impl_extractor!(WindowId);

impl HydrolysisRenderer {
    /// (Re)register a mounted `Menu` trigger's chords on the app's shared
    /// registry, scoped to this renderer's window — the trigger's render
    /// calls it every flush, so a source the flush no longer visits drops
    /// out with its render state.
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    pub(crate) fn register_menu_shortcuts(
        &mut self,
        menu_state: Weak<RefCell<MenuRenderState>>,
        items: Computed<Vec<ResolvedMenuItem>>,
        env: Environment,
    ) {
        env.get::<MenuShortcutRegistry>()
            .cloned()
            .expect(MISSING_MENU_SHORTCUT_REGISTRY)
            .register_mounted(self.window_id, menu_state, items, env);
    }
}

/// The commands of a `Menu` resolve into shortcut entries through the items
/// signal, so a menu whose items change mounts its new chords on the next
/// lookup without any registration churn. A declared Quit arms the chord of
/// [`Quit::command`], and nothing where `env` has no application quit; a
/// declared Close Window arms the application's decided chord of
/// [`close_window_command`], and nothing where the host's windows cannot be
/// closed.
fn collect_menu_shortcuts(
    items: &[ResolvedMenuItem],
    env: &Environment,
    out: &mut Vec<MenuShortcut>,
) {
    for item in items {
        match item {
            ResolvedMenuItem::Command(command) => collect_command_shortcut(command, out),
            ResolvedMenuItem::Quit => {
                if let Some(command) = env.get::<Quit>().map(|quit| quit.command(env)) {
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
                collect_menu_shortcuts(&menu.items.snapshot(), env, out);
            }
            ResolvedMenuItem::Divider => {}
        }
    }
}

fn collect_command_shortcut(command: &ResolvedCommand, out: &mut Vec<MenuShortcut>) {
    if let Some(shortcut) = &command.shortcut {
        out.push(MenuShortcut::new(
            shortcut,
            command.action.clone(),
            command.disabled.clone(),
            command.label.content.snapshot().to_plain(),
        ));
    }
}

/// The same flattening for a built popup's nodes — context-menu nodes carry
/// the shortcut the command was resolved with.
fn collect_popup_shortcuts(nodes: &[PopupMenuNode], out: &mut Vec<MenuShortcut>) {
    for node in nodes {
        match node {
            PopupMenuNode::Command {
                shortcut: Some(shortcut),
                action,
                disabled,
                plain_label,
                ..
            } => {
                out.push(MenuShortcut::new(
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

/// A source of chords: a mounted `Menu` trigger (live while mounted,
/// resolved from its items signal so edits apply, and answering only to the
/// window that mounted it), an open popup menu (live while any menu in its
/// group is open, answering to whichever window holds focus — a popup window
/// that took keyboard focus dispatches into the same registry), or the
/// application's `menu_bar` (live for the app's duration — the runner owns
/// the registry for exactly as long — and answering to whichever window
/// dispatches, since app-level commands are not scoped to one window).
/// `seq` orders sources by registration, so the most recently registered one
/// wins a chord conflict however the windows interleave.
enum MenuShortcutSource {
    Mounted {
        /// The window the chords dispatch on — the one the `Menu` mounted in.
        window: WindowId,
        /// The `Menu` trigger's render state: the source's identity —
        /// `Weak::ptr_eq` against a live upgrade — and its liveness (unmount
        /// drops the render state, and the source with it).
        menu_state: Weak<RefCell<MenuRenderState>>,
        items: Computed<Vec<ResolvedMenuItem>>,
        env: Environment,
        seq: u64,
    },
    Popup {
        shortcuts: Vec<MenuShortcut>,
        group: PopupMenuStateGroup,
        env: Environment,
        seq: u64,
    },
    /// The application-level `App::menu_bar`: resolved to items once by the
    /// runner and kept reactive, so item edits apply on the next lookup.
    AppBar {
        items: Computed<Vec<ResolvedMenuItem>>,
        env: Environment,
        seq: u64,
    },
}

impl MenuShortcutSource {
    const fn seq(&self) -> u64 {
        match self {
            Self::Mounted { seq, .. } | Self::Popup { seq, .. } | Self::AppBar { seq, .. } => *seq,
        }
    }

    fn live(&self) -> bool {
        match self {
            Self::Mounted { menu_state, .. } => menu_state.upgrade().is_some(),
            Self::Popup { group, .. } => group
                .0
                .borrow()
                .iter()
                .any(|state| state.snapshot() != WindowState::Closed),
            // The menu bar lives for the app's duration — the same span the
            // runner owns this registry for.
            Self::AppBar { .. } => true,
        }
    }
}

/// The app's menu-shortcut registry, owned by the runner and seeded into the
/// environment it hands every window — winit, headless, and test runners
/// alike, so the chords a test resolves take the same path as a live app. It
/// holds every chord source — mounted `Menu` triggers keyed by the window
/// that mounted them, and open popup menus — plus the counter that orders
/// them by registration (water-rs/hydrolysis#247).
#[derive(Clone, Default)]
pub struct MenuShortcutRegistry(Rc<RefCell<MenuShortcutRegistryState>>);

#[derive(Default)]
struct MenuShortcutRegistryState {
    sources: Vec<MenuShortcutSource>,
    /// Registration order across every window — a chord conflict resolves to
    /// the most recently registered source.
    next_seq: u64,
    /// Ordinals minted in creation order for windows whose runner has no
    /// native window id.
    next_window_ordinal: u64,
}

impl MenuShortcutRegistryState {
    const fn next_source_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }
}

impl MenuShortcutRegistry {
    /// A fresh window id for a runner with no native one — minted in creation
    /// order. Winit windows take [`WindowId::Winit`] instead.
    pub(crate) fn mint_window_id(&self) -> WindowId {
        let mut inner = self.0.borrow_mut();
        let id = WindowId::Runner(inner.next_window_ordinal);
        inner.next_window_ordinal += 1;
        id
    }

    /// (Re)register a mounted `Menu` trigger's chords — called from the
    /// trigger's render on every flush; a live source just refreshes its
    /// items signal and environment, keeping its registration order.
    fn register_mounted(
        &self,
        window: WindowId,
        menu_state: Weak<RefCell<MenuRenderState>>,
        items: Computed<Vec<ResolvedMenuItem>>,
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

    /// Register the application `menu_bar`'s chords — live for the app's
    /// duration and answering to whichever window dispatches, since app-level
    /// commands are not scoped to one window. The runner resolves the menus
    /// once (`resolve_menu_bar_items` keeps the signal reactive, so item
    /// edits apply on the next lookup) and registers once; re-registration
    /// replaces the source rather than stacking it.
    // Only windowed runners register menus — bare wasm compiles the impl
    // for the rest of its surface and has no caller for this one.
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
    /// the event — whether or not its command fired (disabled commands claim
    /// without firing).
    pub(crate) fn dispatch(
        &self,
        window: WindowId,
        pressed: &keyboard_types::Key,
        modifiers: Modifiers,
        env: &Environment,
    ) -> bool {
        let mut winner: Option<(MenuShortcut, Option<PopupMenuStateGroup>, Environment)> = None;
        let mut conflicts: Vec<Str> = Vec::new();
        {
            let mut inner = self.0.borrow_mut();
            inner.sources.retain(MenuShortcutSource::live);
            inner
                .sources
                .sort_unstable_by_key(|source| std::cmp::Reverse(source.seq()));
            for source in &inner.sources {
                let mut entries = Vec::new();
                match source {
                    MenuShortcutSource::Mounted {
                        window: mounted_window,
                        items,
                        env,
                        ..
                    } => {
                        if *mounted_window != window {
                            continue;
                        }
                        collect_menu_shortcuts(&items.snapshot(), env, &mut entries);
                    }
                    MenuShortcutSource::Popup { shortcuts, .. } => {
                        entries.clone_from(shortcuts);
                    }
                    MenuShortcutSource::AppBar { items, env, .. } => {
                        collect_menu_shortcuts(&items.snapshot(), env, &mut entries);
                    }
                }
                for entry in entries {
                    if entry.matches(pressed, modifiers) {
                        if winner.is_some() {
                            conflicts.push(entry.label.clone());
                        } else {
                            let (group, env) = match source {
                                MenuShortcutSource::Mounted { env, .. }
                                | MenuShortcutSource::AppBar { env, .. } => (None, env.clone()),
                                MenuShortcutSource::Popup { group, env, .. } => {
                                    (Some(group.clone()), env.clone())
                                }
                            };
                            winner = Some((entry, group, env));
                        }
                    }
                }
            }
        }
        let Some((shortcut, group, menu_env)) = winner else {
            return false;
        };
        if !conflicts.is_empty() {
            tracing::warn!(
                chord = %shortcut.hint,
                winner = %shortcut.label,
                losers = ?conflicts,
                "menu shortcut chord registered by multiple menus; the most recently mounted wins",
            );
        }
        // A disabled command still claims its chord — the keystroke is the
        // menu's, it just does nothing.
        if shortcut.disabled.snapshot() {
            return true;
        }
        // A `WindowId` a command's action extracts names the window it acts
        // on: mounted and app-bar chords act on the window the chord
        // dispatched for, so that window rides in the action's environment.
        // A popup source's rows carry their owner window's id already — it
        // is the row's target, never the window the chord happened to
        // dispatch on.
        let action_env = menu_env.layered_on(env);
        let action_env = if group.is_none() {
            action_env.extending(window)
        } else {
            action_env
        };
        if let Some(group) = group {
            group.close_all();
        }
        call_action_discarding_result(&shortcut.action, &action_env);
        true
    }
}
