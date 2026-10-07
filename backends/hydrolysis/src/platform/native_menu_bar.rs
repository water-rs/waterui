//! The native menu-bar surface for `App::menu_bar` — the winit platforms
//! that own a real menu-bar object (see `runner/menu_bar.rs` for the
//! per-platform contract).
//!
//! `muda` projects the resolved top-level `Menu`s onto the platform's
//! menu bar: `NSApp.mainMenu` on macOS (`Menu::init_for_nsapp`), a Win32
//! `HMENU` attached to each application window's `HWND` on Windows
//! (`Menu::init_for_hwnd`, called from `attach_hwnd` once the window
//! exists). A `Command` carries its `shortcut` as the item's accelerator, a
//! `Menu` recurses into a `Submenu`, and a `Divider` maps to a separator
//! item.
//!
//! Activating an item — choosing it, or pressing its accelerator — posts a
//! `MenuEvent` on muda's channel. The runner drains that channel on the
//! event-loop thread (`pump_menu_events`) and runs the command's
//! `SharedAction` against the app environment — the same dispatch the
//! `MenuShortcutRegistry` uses.
//!
//! Chord dispatch stays exactly-once on both platforms, but for different
//! reasons:
//!
//! - **macOS**: `AppKit` matches the `NSMenuItem` key equivalent in
//!   `-[NSApplication sendEvent]` before `keyDown` is delivered to the
//!   window (winit's view does not override `performKeyEquivalent`), so a
//!   claimed accelerator never reaches the registry's key path. The
//!   registry remains armed as the fallback for keys `AppKit` does not claim
//!   — the two dispatch paths see disjoint keys.
//! - **Windows**: muda accelerators only fire through
//!   `TranslateAcceleratorW`, which winit's message pump never calls — the
//!   native menu shows the accelerator text but the chord itself fires
//!   only through the registry. Click goes through muda, chord through the
//!   registry.
//!
//! Reactivity: the resolved items signal is watched and a change rebuilds
//! the whole bar (menu trees are small, rebuilds rare). `disabled` and
//! `selected` each carry their own watch into `set_enabled`/`set_checked`,
//! so validation state stays live between rebuilds.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use muda::accelerator::{Key, KeyAccelerator, Modifiers};
use muda::{CheckMenuItem, Menu as NativeMenu, MenuEvent, MenuId, PredefinedMenuItem, Submenu};
use nami::Computed;
use nami::Signal as _;
use nami::watcher::BoxWatcherGuard;
use waterui::Environment;
#[cfg(not(target_os = "macos"))]
use waterui::app::Quit;
#[cfg(target_os = "macos")]
use waterui_controls::menu::ResolvedNestedMenu;
#[cfg(target_os = "macos")]
use waterui_controls::menu::{NamedKey, ShortcutKey};
use waterui_controls::menu::{ResolvedCommand, ResolvedMenuItem, Shortcut};
use waterui_core::handler::SharedAction;

use crate::renderer::call_action_discarding_result;

/// One built native menu bar: the muda `Menu` tree, the `MenuId → action`
/// table the event pump dispatches through, and the live watches.
struct Bar {
    menu: NativeMenu,
    actions: HashMap<MenuId, SharedAction<()>>,
    /// Windows HWNDs this bar is attached to — a rebuild detaches the old
    /// tree from each and attaches the new one; a window's `detach_hwnd`
    /// forgets its HWND before the HWND is destroyed so neither path ever
    /// touches a dead one.
    hwnds: Vec<isize>,
    /// Per-command `disabled`/`selected` watches, plus the per-submenu
    /// item-list watches; a rebuild swaps the set.
    _state_watches: Vec<BoxWatcherGuard>,
}

/// The installed native menu bar: the resolved-items watch (rebuilds the
/// bar on edits) plus the shared `MenuId → action` table.
pub struct NativeMenuBar {
    bar: Rc<RefCell<Option<Bar>>>,
    env: Environment,
    _watch: BoxWatcherGuard,
}

/// Whether a native menu item may carry `shortcut`'s accelerator.
///
/// Upstream muda 0.20 defect: on macOS it gives `Delete` the key equivalent
/// U+007F, which is `NSDeleteCharacter`, the ⌫ key; forward delete (⌦) is
/// `NSDeleteFunctionKey` (U+F728). It gives `Backspace` U+0008
/// (`NSBackspaceCharacter`), which the ⌫ key does not send. The native item
/// would claim the wrong key or none, so it carries no accelerator for either
/// there; the registry still fires both chords.
#[cfg(target_os = "macos")]
const fn native_item_arms(shortcut: &Shortcut) -> bool {
    !matches!(
        shortcut.key,
        ShortcutKey::Named(NamedKey::Delete | NamedKey::Backspace)
    )
}

/// Whether a native menu item may carry `shortcut`'s accelerator — always:
/// the only exclusions are the macOS `Delete`/`Backspace` mapping defects
/// above.
#[cfg(not(target_os = "macos"))]
const fn native_item_arms(_shortcut: &Shortcut) -> bool {
    true
}

/// Maps a `Shortcut` to a muda [`KeyAccelerator`] on the key's W3C value:
/// modifiers go through the same platform mapping `ChordModifiers` gives the
/// registry (the command modifier is the menu accelerator — ⌘ on macOS, Ctrl
/// elsewhere).
fn accelerator_for(shortcut: &Shortcut) -> KeyAccelerator {
    let mut mods = Modifiers::empty();
    let modifiers = shortcut.modifiers;
    #[cfg(target_os = "macos")]
    {
        if modifiers.command() {
            mods |= Modifiers::META;
        }
        if modifiers.control() {
            mods |= Modifiers::CONTROL;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if modifiers.command() || modifiers.control() {
            mods |= Modifiers::CONTROL;
        }
    }
    if modifiers.option() {
        mods |= Modifiers::ALT;
    }
    if modifiers.shift() {
        mods |= Modifiers::SHIFT;
    }
    let key = match shortcut.key.to_key() {
        // An uppercase character accelerator implies Shift; Shift comes only
        // from the shortcut's modifiers.
        Key::Character(character) => Key::Character(character.to_lowercase()),
        named @ Key::Named(_) => named,
    };
    KeyAccelerator::new(mods, key)
}

fn command_title(command: &ResolvedCommand) -> String {
    command.label.content.snapshot().to_plain().to_string()
}

/// Builds one `CheckMenuItem` per command: the check gutter is invisible
/// while unselected on every platform, and `set_checked` can then reflect
/// `selected` live — the same validation contract a mounted `Menu` gives
/// through the registry's per-dispatch snapshot. `title` is the native
/// item's text, which on Windows may mark an access key with `&`.
fn build_command(
    title: &str,
    command: &ResolvedCommand,
    actions: &mut HashMap<MenuId, SharedAction<()>>,
    state_watches: &mut Vec<BoxWatcherGuard>,
) -> CheckMenuItem {
    let item = CheckMenuItem::new(
        title,
        !command.disabled.snapshot(),
        command.selected.snapshot(),
        None,
    );
    if let Some(shortcut) = command
        .shortcut
        .as_ref()
        .filter(|shortcut| native_item_arms(shortcut))
    {
        item.set_key_accelerator(Some(accelerator_for(shortcut)))
            .unwrap_or_else(|error| {
                panic!(
                    "the native menu bar cannot arm `{title}`'s shortcut {shortcut:?} on this \
                     platform: {error}"
                )
            });
    }
    actions.insert(item.id().clone(), command.action.clone());
    let disabled_item = item.clone();
    state_watches.push(command.disabled.watch(move |ctx| {
        disabled_item.set_enabled(!ctx.into_value());
    }));
    let selected_item = item.clone();
    state_watches.push(command.selected.watch(move |ctx| {
        selected_item.set_checked(ctx.into_value());
    }));
    item
}

/// Appends resolved items to a muda container — `append` has the same shape
/// on `Menu` and `Submenu`, so the caller passes it as a closure. A nested
/// menu's own items signal can change without the top-level list
/// re-emitting, so it gets a watch that rebuilds the whole bar. Building
/// the app's own menu tree cannot fail — a failure is a bug and panics.
fn append_items(
    parent: &dyn Fn(&dyn muda::IsMenuItem),
    items: &[ResolvedMenuItem],
    env: &Environment,
    actions: &mut HashMap<MenuId, SharedAction<()>>,
    state_watches: &mut Vec<BoxWatcherGuard>,
    slot: &Rc<RefCell<Option<Bar>>>,
    top_items: &Computed<Vec<ResolvedMenuItem>>,
) {
    for item in items {
        match item {
            ResolvedMenuItem::Command(command) => {
                #[cfg(target_os = "macos")]
                command.assert_allowed_in_macos_menu_bar();
                parent(&build_command(
                    &command_title(command),
                    command,
                    actions,
                    state_watches,
                ));
            }
            ResolvedMenuItem::Quit => {
                // macOS's standard application menu already carries the
                // platform Quit (`build_app_menu`), so a declared one never
                // repeats it there. On Windows the bar shows the quit
                // command — "Exit", Ctrl+Q — whose chord also arms on the
                // registry, with the `&` access key the platform's own Exit
                // item carries. The mnemonic belongs to the Win32 menu
                // alone: the command's label is also a self-drawn popup
                // row, which would print the `&`.
                #[cfg(not(target_os = "macos"))]
                if let Some(command) = env.get::<Quit>().map(|quit| quit.command(env)) {
                    parent(&build_command(
                        &format!("&{}", command_title(&command)),
                        &command,
                        actions,
                        state_watches,
                    ));
                }
            }
            ResolvedMenuItem::Divider => {
                parent(&PredefinedMenuItem::separator());
            }
            ResolvedMenuItem::Menu(nested) => {
                let submenu = Submenu::new(nested.label.content.snapshot().to_plain(), true);
                append_items(
                    &|entry| {
                        submenu
                            .append(entry)
                            .expect("appending a submenu item failed");
                    },
                    &nested.items.snapshot(),
                    env,
                    actions,
                    state_watches,
                    slot,
                    top_items,
                );
                parent(&submenu);
                let slot = Rc::clone(slot);
                let top_items = top_items.clone();
                let env = env.clone();
                state_watches.push(nested.items.watch(move |_| {
                    rebuild_bar(&slot, &top_items, &env);
                }));
            }
        }
    }
}

/// The product name the macOS application menu and its named items are
/// labeled with: `CFBundleName` from the main bundle's Info.plist (the key
/// Finder and the Dock read), then `CFBundleDisplayName` for bundles that
/// set only the display form, and the process name when the binary runs
/// with no bundle at all.
#[cfg(target_os = "macos")]
fn product_name() -> String {
    use objc2_foundation::{NSBundle, NSProcessInfo, NSString, ns_string};
    let bundle = NSBundle::mainBundle();
    for key in [
        ns_string!("CFBundleName"),
        ns_string!("CFBundleDisplayName"),
    ] {
        if let Some(name) = bundle
            .objectForInfoDictionaryKey(key)
            .and_then(|value| value.downcast::<NSString>().ok())
            .map(|name| name.to_string())
            .filter(|name| !name.trim().is_empty())
        {
            return name;
        }
    }
    NSProcessInfo::processInfo().processName().to_string()
}

/// A resolved top-level menu's title in plain text.
#[cfg(target_os = "macos")]
fn menu_title(menu: &ResolvedNestedMenu) -> String {
    menu.label.content.snapshot().to_plain().to_string()
}

/// Builds the standard macOS application menu — About, Services, Hide,
/// Hide Others, Show All, Quit — as a submenu titled by the product name.
/// Every bar gets one, so an application that declares no app-level Quit
/// still has a working one and `NSApp.mainMenu` always opens with the
/// conventional item set (water-rs/hydrolysis#321).
///
/// When the application declares its own application menu — a top-level
/// `Menu` titled by the product name — its items fold in between About and
/// Services, the spot macOS conventions reserve for app-level entries like
/// Settings, instead of standing next to the standard menu as a duplicate
/// product-named top-level menu. The declared menu's own items signal
/// still rebuilds the bar on change, like every other nested-items watch.
#[cfg(target_os = "macos")]
fn build_app_menu(
    product_name: &str,
    declared: Option<&ResolvedNestedMenu>,
    env: &Environment,
    actions: &mut HashMap<MenuId, SharedAction<()>>,
    state_watches: &mut Vec<BoxWatcherGuard>,
    slot: &Rc<RefCell<Option<Bar>>>,
    top_items: &Computed<Vec<ResolvedMenuItem>>,
) -> Submenu {
    let app_menu = Submenu::new(product_name, true);
    let append = |entry: &dyn muda::IsMenuItem| {
        app_menu
            .append(entry)
            .expect("appending to the application menu failed");
    };
    append(&PredefinedMenuItem::about(
        Some(&format!("About {product_name}")),
        None,
    ));
    if let Some(declared) = declared {
        let declared_items = declared.items.snapshot();
        if !declared_items.is_empty() {
            append(&PredefinedMenuItem::separator());
            append_items(
                &append,
                &declared_items,
                env,
                actions,
                state_watches,
                slot,
                top_items,
            );
        }
        let slot = Rc::clone(slot);
        let top_items = top_items.clone();
        let env = env.clone();
        state_watches.push(declared.items.watch(move |_| {
            rebuild_bar(&slot, &top_items, &env);
        }));
        append(&PredefinedMenuItem::separator());
    } else {
        append(&PredefinedMenuItem::separator());
    }
    append(&PredefinedMenuItem::services(None));
    append(&PredefinedMenuItem::separator());
    append(&PredefinedMenuItem::hide(Some(&format!(
        "Hide {product_name}"
    ))));
    append(&PredefinedMenuItem::hide_others(None));
    append(&PredefinedMenuItem::show_all(None));
    append(&PredefinedMenuItem::separator());
    append(&PredefinedMenuItem::quit(Some(&format!(
        "Quit {product_name}"
    ))));
    app_menu
}

/// The index of the application's own application menu in the resolved
/// top-level list — the first `Menu` titled by the product name, which the
/// macOS bar folds into the standard application menu instead of showing
/// twice.
#[cfg(target_os = "macos")]
fn declared_app_menu_index(items: &[ResolvedMenuItem], product_name: &str) -> Option<usize> {
    items.iter().position(
        |item| matches!(item, ResolvedMenuItem::Menu(menu) if menu_title(menu) == product_name),
    )
}

fn build_bar(
    items: &[ResolvedMenuItem],
    env: &Environment,
    slot: &Rc<RefCell<Option<Bar>>>,
    top_items: &Computed<Vec<ResolvedMenuItem>>,
) -> Bar {
    let menu = NativeMenu::new();
    let mut actions = HashMap::new();
    let mut state_watches = Vec::new();
    let parent = |entry: &dyn muda::IsMenuItem| {
        menu.append(entry)
            .expect("appending a top-level item to the menu bar failed");
    };
    #[cfg(target_os = "macos")]
    {
        let product_name = product_name();
        let app_index = declared_app_menu_index(items, &product_name);
        let declared = app_index.map(|index| {
            let ResolvedMenuItem::Menu(menu) = &items[index] else {
                unreachable!("the app-menu index points at a menu");
            };
            menu
        });
        parent(&build_app_menu(
            &product_name,
            declared,
            env,
            &mut actions,
            &mut state_watches,
            slot,
            top_items,
        ));
        if let Some(index) = app_index {
            append_items(
                &parent,
                &items[..index],
                env,
                &mut actions,
                &mut state_watches,
                slot,
                top_items,
            );
            append_items(
                &parent,
                &items[index + 1..],
                env,
                &mut actions,
                &mut state_watches,
                slot,
                top_items,
            );
        } else {
            append_items(
                &parent,
                items,
                env,
                &mut actions,
                &mut state_watches,
                slot,
                top_items,
            );
        }
    }
    #[cfg(not(target_os = "macos"))]
    append_items(
        &parent,
        items,
        env,
        &mut actions,
        &mut state_watches,
        slot,
        top_items,
    );
    Bar {
        menu,
        actions,
        hwnds: Vec::new(),
        _state_watches: state_watches,
    }
}

impl Bar {
    /// Installs the tree on the platform surface: `NSApp.mainMenu` on
    /// macOS; `SetMenu` on each attached HWND on Windows.
    fn install_native(&self) {
        #[cfg(target_os = "macos")]
        {
            self.menu.init_for_nsapp();
        }
        #[cfg(target_os = "windows")]
        {
            for &hwnd in &self.hwnds {
                // SAFETY: each `hwnd` is a live application window owned by
                // this process, recorded at window creation.
                unsafe {
                    self.menu
                        .init_for_hwnd(hwnd)
                        .expect("attaching the menu bar to a live HWND failed");
                }
            }
        }
    }

    /// Detaches the tree from its platform surface (pre-rebuild; on macOS
    /// `init_for_nsapp` replaces the bar outright, so only Windows needs
    /// this).
    #[cfg(target_os = "windows")]
    fn remove_native(&self) {
        for &hwnd in &self.hwnds {
            // SAFETY: same liveness contract as `install_native`.
            unsafe {
                self.menu
                    .remove_for_hwnd(hwnd)
                    .expect("detaching the menu bar from an attached HWND failed");
            }
        }
    }

    /// Records a live application-window HWND the bar attaches to.
    #[cfg(target_os = "windows")]
    fn record_hwnd(&mut self, hwnd: isize) {
        self.hwnds.push(hwnd);
    }

    /// Forgets a closed window's HWND: the runner calls it before the HWND
    /// is destroyed, and the removal drives the native `remove_for_hwnd`.
    /// `false` when the HWND was never attached — a no-op, and no native
    /// call is made.
    #[cfg(target_os = "windows")]
    fn forget_hwnd(&mut self, hwnd: isize) -> bool {
        self.hwnds
            .iter()
            .position(|attached| *attached == hwnd)
            .map(|position| self.hwnds.swap_remove(position))
            .is_some()
    }
}

/// The HWNDs a rebuilt bar attaches to: the windows still attached when
/// the rebuild runs — `detach_hwnd` has already forgotten the closed
/// ones.
fn carried_hwnds(old: Option<Bar>) -> Vec<isize> {
    old.map(|old| old.hwnds).unwrap_or_default()
}

/// Rebuilds the whole native bar in `slot` from a fresh `top_items`
/// snapshot: detach the old tree, build, re-install, store. Called by the
/// top-level items watch and by each nested-items watch.
fn rebuild_bar(
    slot: &Rc<RefCell<Option<Bar>>>,
    top_items: &Computed<Vec<ResolvedMenuItem>>,
    env: &Environment,
) {
    let mut guard = slot.borrow_mut();
    let old = guard.take();
    #[cfg(target_os = "windows")]
    if let Some(old) = &old {
        old.remove_native();
    }
    let mut fresh = build_bar(&top_items.snapshot(), env, slot, top_items);
    fresh.hwnds = carried_hwnds(old);
    fresh.install_native();
    *guard = Some(fresh);
}

impl NativeMenuBar {
    /// Builds the native bar for `items` and installs it on the platform
    /// surface that exists already: `NSApp.mainMenu` on macOS immediately;
    /// on Windows nothing renders until the runner hands over the first
    /// HWND (`attach_hwnd`).
    ///
    /// The runner calls this on the event loop's main thread — `AppKit`
    /// requires it.
    pub(crate) fn install(items: &Computed<Vec<ResolvedMenuItem>>, env: &Environment) -> Self {
        #[cfg(target_os = "macos")]
        {
            let _ = objc2::MainThreadMarker::new()
                .expect("the winit runner installs the menu bar on the main thread");
        }

        let bar: Rc<RefCell<Option<Bar>>> = Rc::new(RefCell::new(None));
        rebuild_bar(&bar, items, env);
        let watch = {
            let bar = Rc::clone(&bar);
            let items_for_watch = items.clone();
            let env = env.clone();
            items.watch(move |_| rebuild_bar(&bar, &items_for_watch, &env))
        };
        Self {
            bar,
            env: env.clone(),
            _watch: watch,
        }
    }

    /// Attaches the bar to a Windows application's HWND (`SetMenu`). Called
    /// by the runner for each application window as it is created; a later
    /// rebuild re-attaches to every recorded HWND.
    #[cfg(target_os = "windows")]
    pub(crate) fn attach_hwnd(&self, hwnd: isize) {
        let mut slot = self.bar.borrow_mut();
        let Some(bar) = slot.as_mut() else {
            return;
        };
        bar.record_hwnd(hwnd);
        // SAFETY: the caller guarantees `hwnd` is a live window owned by
        // this process.
        unsafe {
            bar.menu
                .init_for_hwnd(hwnd)
                .expect("attaching the menu bar to a live HWND failed");
        }
    }

    /// Forgets a Windows window's HWND — the runner calls it before the
    /// window (and its HWND) is destroyed, so the next rebuild and the
    /// native detach only ever see live windows. A HWND that was never
    /// attached (a popup window) is a no-op.
    #[cfg(target_os = "windows")]
    pub(crate) fn detach_hwnd(&self, hwnd: isize) {
        let mut slot = self.bar.borrow_mut();
        let Some(bar) = slot.as_mut() else {
            return;
        };
        if !bar.forget_hwnd(hwnd) {
            return;
        }
        // SAFETY: the runner calls this before destroying the window, so
        // `hwnd` is still live.
        unsafe {
            bar.menu
                .remove_for_hwnd(hwnd)
                .expect("detaching the menu bar from an attached HWND failed");
        }
    }

    /// Drains muda's `MenuEvent` channel and dispatches each item's action
    /// through `call_action_discarding_result` — the same entry the
    /// `MenuShortcutRegistry` uses. Called once per event-loop pass.
    pub(crate) fn pump_menu_events(&self) {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let action = self
                .bar
                .borrow()
                .as_ref()
                .and_then(|bar| bar.actions.get(event.id()).cloned());
            // The action may mutate menu state (which rebuilds the bar and
            // borrows this cell again) — dispatch after releasing the borrow.
            if let Some(action) = action {
                call_action_discarding_result(&action, &self.env);
            }
        }
    }
}

// `muda::Menu::new` requires the main thread on macOS, so the slot test
// builds its bars where HWNDs exist — Windows.
#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    /// `detach_hwnd`'s bookkeeping: a closed window's HWND leaves the
    /// attach list, and the rebuild carries forward only the windows still
    /// open — so `remove_for_hwnd`/`init_for_hwnd` never run on a dead
    /// HWND.
    #[test]
    fn a_detached_hwnd_is_not_carried_into_the_next_bar() {
        let slot: Rc<RefCell<Option<Bar>>> = Rc::new(RefCell::new(None));
        let items: Computed<Vec<ResolvedMenuItem>> = Computed::constant(Vec::new());
        let mut bar = build_bar(&items.snapshot(), &Environment::new(), &slot, &items);
        bar.record_hwnd(1);
        bar.record_hwnd(2);
        bar.record_hwnd(3);

        // Window 2 closes — the runner detaches it before destroy.
        assert!(bar.forget_hwnd(2), "the attached hwnd detaches");
        assert!(!bar.forget_hwnd(2), "detaching twice is a no-op");
        assert!(!bar.forget_hwnd(9), "an hwnd never attached is a no-op");
        *slot.borrow_mut() = Some(bar);

        // Rebuild: the fresh bar carries only the still-open windows
        // (carried_hwnds is the carry step `rebuild_bar` runs).
        let old = slot.borrow_mut().take().expect("a bar is installed");
        assert_eq!(carried_hwnds(Some(old)), [1, 3]);
    }
}
