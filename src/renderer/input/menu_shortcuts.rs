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
//! conflict.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use nami::{Computed, Signal as _};
use waterui::window::WindowState;
use waterui_controls::menu::{ResolvedMenuItem, Shortcut};
use waterui_core::Environment;
use waterui_core::Str;
use waterui_core::handler::SharedAction;

use super::popup_menu::{PopupMenuNode, PopupMenuStateGroup};
use crate::HydrolysisRenderer;
use crate::platform::{KeyCode, Modifiers};
use crate::renderer::call_action_discarding_result;
use crate::widgets::controls::button::MenuRenderState;

/// The trailing hint a menu row draws for a `Command`'s shortcut — `⌃⌥⇧⌘`
/// symbols on macOS, `Ctrl+Alt+Shift+` text elsewhere (where the command
/// modifier is control, the platform's menu accelerator).
pub(crate) fn shortcut_hint_text(shortcut: &Shortcut) -> Str {
    let modifiers = shortcut.modifiers;
    let key = shortcut.key.to_uppercase();
    #[cfg(target_os = "macos")]
    {
        let mut hint = String::new();
        if modifiers.control() {
            hint.push('⌃');
        }
        if modifiers.option() {
            hint.push('⌥');
        }
        if modifiers.shift() {
            hint.push('⇧');
        }
        if modifiers.command() {
            hint.push('⌘');
        }
        hint.push_str(&key);
        Str::from(hint)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut hint = String::new();
        if modifiers.control() || modifiers.command() {
            hint.push_str("Ctrl+");
        }
        if modifiers.option() {
            hint.push_str("Alt+");
        }
        if modifiers.shift() {
            hint.push_str("Shift+");
        }
        hint.push_str(&key);
        Str::from(hint)
    }
}

/// The normalized modifier set of a `Shortcut`: the command modifier is the
/// platform menu accelerator — super on macOS, control elsewhere.
#[derive(Clone, Copy)]
struct ChordModifiers {
    control: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
}

impl ChordModifiers {
    fn of(shortcut: &Shortcut) -> Self {
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

/// One command's chord: the normalized modifier set and lowercased key, the
/// action it runs, its live disabled flag, the label a conflict warning names
/// it by, and the rendered hint text.
#[derive(Clone)]
struct MenuShortcut {
    modifiers: ChordModifiers,
    key: Str,
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
            key: shortcut.key.to_lowercase().into(),
            action,
            disabled,
            label,
            hint: shortcut_hint_text(shortcut),
        }
    }

    fn matches(&self, key: &str, pressed: Modifiers) -> bool {
        self.modifiers.control == pressed.control
            && self.modifiers.alt == pressed.alt
            && self.modifiers.shift == pressed.shift
            && self.modifiers.super_key == pressed.super_key
            && key.eq_ignore_ascii_case(&self.key)
    }
}

/// The panic text for a renderer resolving menu chords in an environment no
/// runner prepared: registering and dispatching both name it, so the missing
/// seed names the runner that skipped it.
pub(crate) const MISSING_MENU_SHORTCUT_REGISTRY: &str = "menu shortcuts require the runner to seed a MenuShortcutRegistry into the window's \
     environment — the winit runner, headless run and HeadlessRuntime, SemanticRuntime and \
     the web runner all install one; a renderer built outside a runner seeds its own";

/// A window's identity for menu-chord dispatch: the runner assigns one when it
/// creates the window and hands it to the window's renderer, so the shared
/// [`MenuShortcutRegistry`] can scope a mounted `Menu`'s chords to the window
/// they mounted in. Unlike a pointer address, it cannot drift when the
/// renderer moves nor alias a recycled allocation (water-rs/hydrolysis#247).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowId {
    /// A winit window — its native `winit::window::WindowId`.
    #[cfg(feature = "winit")]
    Winit(winit::window::WindowId),
    /// A window on a runner without a native window id — an ordinal the
    /// registry hands out in window-creation order.
    Runner(u64),
    /// A renderer no runner claimed — embedded GPU hosts and renderers built
    /// directly in tests. Their mounted sources answer only to their own
    /// dispatch.
    Orphan,
}

impl HydrolysisRenderer {
    /// (Re)register a mounted `Menu` trigger's chords on the app's shared
    /// registry, scoped to this renderer's window — the trigger's render
    /// calls it every flush, so a source the flush no longer visits drops
    /// out with its render state.
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
/// lookup without any registration churn.
fn collect_menu_shortcuts(items: &[ResolvedMenuItem], out: &mut Vec<MenuShortcut>) {
    for item in items {
        match item {
            ResolvedMenuItem::Command(command) => {
                if let Some(shortcut) = &command.shortcut {
                    out.push(MenuShortcut::new(
                        shortcut,
                        command.action.clone(),
                        command.disabled.clone(),
                        command.label.content.snapshot().to_plain(),
                    ));
                }
            }
            ResolvedMenuItem::Menu(menu) => collect_menu_shortcuts(&menu.items.snapshot(), out),
            ResolvedMenuItem::Divider => {}
        }
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
    fn seq(&self) -> u64 {
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
pub(crate) struct MenuShortcutRegistry(Rc<RefCell<MenuShortcutRegistryState>>);

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
    fn next_source_seq(&mut self) -> u64 {
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

    /// Dispatch a pressed key for `window` against the registry. Returns
    /// `true` when a chord matched and claimed the event — whether or not its
    /// command fired (disabled commands claim without firing).
    pub(crate) fn dispatch(
        &self,
        window: WindowId,
        key: &KeyCode,
        modifiers: Modifiers,
        env: &Environment,
    ) -> bool {
        let KeyCode::Character(pressed) = key else {
            return false;
        };
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
                        ..
                    } => {
                        if *mounted_window != window {
                            continue;
                        }
                        collect_menu_shortcuts(&items.snapshot(), &mut entries);
                    }
                    MenuShortcutSource::Popup { shortcuts, .. } => {
                        entries.clone_from(shortcuts);
                    }
                    MenuShortcutSource::AppBar { items, .. } => {
                        collect_menu_shortcuts(&items.snapshot(), &mut entries);
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
        if let Some(group) = group {
            group.close_all();
        }
        call_action_discarding_result(&shortcut.action, &menu_env.layered_on(env));
        true
    }
}
