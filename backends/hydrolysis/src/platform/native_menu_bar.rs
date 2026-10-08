//! The native menu-bar surface for `App::menu_bar` — the winit platforms
//! that own a real menu-bar object (see `runner/menu_bar.rs` for the
//! per-platform contract).
//!
//! `waterkit-menu` projects the resolved top-level `Menu`s onto the
//! platform's menu bar: `NSApp.mainMenu` on macOS (`MenuBar::install`), a
//! Win32 `HMENU` attached to each application window's `HWND` on Windows
//! (`MenuBar::attach`, called from `attach_hwnd` once the window exists —
//! one bar per window, because a bar allows a single attachment). A
//! `Command` carries its `shortcut` as the item's key equivalent, a `Menu`
//! recurses into a `Submenu`, and a `Divider` maps to a separator item.
//!
//! Choosing an item — clicking it, or pressing its accelerator where the
//! host translates them — reports the item's `CommandId` on the bar's
//! `events()` stream. The runner drains it on the event-loop thread
//! (`pump_menu_events`) and runs the command's `SharedAction` against the
//! app environment — the same dispatch the `MenuShortcutRegistry` uses.
//!
//! Chord dispatch stays exactly-once on both platforms, but for different
//! reasons:
//!
//! - **macOS**: `AppKit` matches the `NSMenuItem` key equivalent in
//!   `-[NSApplication sendEvent]` before `keyDown` is delivered to the
//!   window (winit's view does not override `performKeyEquivalent`), so a
//!   claimed accelerator never reaches the registry's key path. The
//!   registry stays armed for the keys `AppKit` does not claim — the two
//!   dispatch paths see disjoint keys.
//! - **Windows**: the bar's accelerators are row text plus an `HACCEL`
//!   (`MenuBar::accelerator_table`), which nothing translates — winit's
//!   message pump never calls `TranslateAcceleratorW` — so the chord
//!   itself fires only through the registry. Click goes through the bar's
//!   stream, chord through the registry.
//!
//! Reactivity: every resolved `Menu`'s items signal is watched and a change
//! rebuilds the whole bar (menu trees are small, rebuilds rare). `disabled`
//! and `selected` each carry their own watch into `MenuBar::set_enabled`
//! and `set_checked`, so validation state stays live between rebuilds.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use nami::Computed;
use nami::Signal as _;
use nami::watcher::BoxWatcherGuard;
#[cfg(target_os = "windows")]
use waterkit_menu::NamedKey;
#[cfg(target_os = "macos")]
use waterkit_menu::StandardItem;
use waterkit_menu::{
    Command, CommandId, Entry, Key, MenuBar, Modifiers, Shortcut as MenuShortcut, Submenu,
};
use waterui::Environment;
#[cfg(not(target_os = "macos"))]
use waterui::app::Quit;
#[cfg(target_os = "macos")]
use waterui_controls::menu::CloseWindowPlacement;
use waterui_controls::menu::{ResolvedCommand, ResolvedMenuItem, ResolvedNestedMenu, Shortcut};
use waterui_core::handler::SharedAction;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;

use crate::renderer::{WindowId, call_action_discarding_result, close_window_command};

/// The slot the installed bar lives in, shared by the rebuild watches, the
/// state watches and the event pump. It is `Some` from the end of
/// [`NativeMenuBar::install`] on; a rebuild empties it only while it holds
/// the slot's mutable borrow.
type Slot = Rc<RefCell<Option<Bar>>>;

/// The pieces of one built `MenuBar` that dispatch needs after the item
/// tree is gone: the `CommandId → action` table `pump_menu_events`
/// dispatches through, the bar's `events()` stream, and the per-command
/// state watches. A rebuild swaps the set wholesale.
struct Dispatch {
    actions: HashMap<CommandId, SharedAction<()>>,
    events: BoxStream<'static, CommandId>,
    _state_watches: Vec<BoxWatcherGuard>,
}

/// The installed native menu bar on macOS: the `MenuBar` that owns
/// `NSApp.mainMenu`, its dispatch pieces, and the rebuild watches over the
/// resolved menus' item lists.
#[cfg(target_os = "macos")]
struct Bar {
    native: MenuBar,
    dispatch: Dispatch,
    _structure_watches: Vec<BoxWatcherGuard>,
}

/// The installed native menu bars on Windows — one `MenuBar` per attached
/// window, keyed by `HWND`: `MenuBar::attach` allows a single attachment,
/// so every application window owns a bar built from the same resolved
/// items. The rebuild watches over the resolved menus' item lists are
/// registered once, for all of them.
#[cfg(target_os = "windows")]
struct Bar {
    windows: HashMap<isize, WindowBar>,
    _structure_watches: Vec<BoxWatcherGuard>,
}

/// One window's `MenuBar` and the guard that keeps it attached.
#[cfg(target_os = "windows")]
struct WindowBar {
    /// The attachment guard. It borrows the `MenuBar` behind `bar` — its
    /// drop writes back into that bar — so it is declared first and drops
    /// first. Dropping it removes the window's menu subclass and detaches
    /// the `HMENU`, which the runner orders before the `HWND` is destroyed.
    _attachment: waterkit_menu::Attachment<'static>,
    /// The bar `_attachment` borrows. An `Rc` rather than a `Box`: the
    /// pointee never moves either way, but moving a `Box` asserts unique
    /// access to it, which the guard's shared borrow would contradict. The
    /// handle is never cloned.
    bar: Rc<MenuBar>,
    /// The window this bar's commands act on: `MenuBar::attach` ties the
    /// bar to one window, so an activation on its `HWND` names exactly this
    /// window — the same target `WM_CLOSE` on that window would name.
    window: WindowId,
    dispatch: Dispatch,
}

/// The installed native menu bar: the top-level resolved-items watch
/// (rebuilds the bar on edits) plus the slot the watches and the event
/// pump share.
pub struct NativeMenuBar {
    bar: Slot,
    env: Environment,
    /// The resolved-items signal a Windows `attach_hwnd` builds a new
    /// window's bar from; macOS builds its one bar only on a rebuild.
    #[cfg(target_os = "windows")]
    top_items: Computed<Vec<ResolvedMenuItem>>,
    _watch: BoxWatcherGuard,
}

/// The `MenuBar` a command's state watches update: the application bar on
/// macOS, and the bar of the window `hwnd` on Windows.
#[derive(Clone)]
struct BarRef {
    slot: Slot,
    #[cfg(target_os = "windows")]
    hwnd: isize,
}

impl BarRef {
    /// Runs `update` on the referenced bar. A state watch lives inside the
    /// bar it updates and drops with it, so the bar is always there.
    fn update(&self, update: impl FnOnce(&MenuBar)) {
        let slot = self.slot.borrow();
        let bar = slot
            .as_ref()
            .expect("a menu-bar state watch outlived its bar");
        #[cfg(target_os = "macos")]
        update(&bar.native);
        #[cfg(target_os = "windows")]
        update(
            &bar.windows
                .get(&self.hwnd)
                .expect("a window bar's state watch outlived the window's bar")
                .bar,
        );
    }
}

/// Everything building one `MenuBar` accumulates while it walks the
/// resolved items — one `Builder` per built bar, so each window's bar on
/// Windows gets its own `CommandId`s, actions table and state watches.
struct Builder<'a> {
    env: &'a Environment,
    target: BarRef,
    /// `CommandId`s are caller-supplied and must be unique per bar: the
    /// per-build counter hands them out.
    next_id: u64,
    actions: HashMap<CommandId, SharedAction<()>>,
    state_watches: Vec<BoxWatcherGuard>,
}

impl<'a> Builder<'a> {
    fn new(env: &'a Environment, target: BarRef) -> Self {
        Self {
            env,
            target,
            next_id: 1,
            actions: HashMap::new(),
            state_watches: Vec::new(),
        }
    }

    const fn alloc_id(&mut self) -> CommandId {
        let id = CommandId::new(self.next_id);
        self.next_id += 1;
        id
    }

    /// The dispatch pieces of the bar this builder filled.
    fn into_dispatch(self, bar: &MenuBar) -> Dispatch {
        Dispatch {
            actions: self.actions,
            events: bar.events().boxed(),
            _state_watches: self.state_watches,
        }
    }
}

/// Maps a `Shortcut` to a `waterkit_menu` [`MenuShortcut`] on the key's
/// W3C value: modifiers go through the same platform mapping
/// `ChordModifiers` gives the registry (`COMMAND` is the platform's menu
/// accelerator — ⌘ on macOS, Ctrl on Windows; `control` is the literal
/// Control key), and named keys map in the crate — `Delete` is
/// `NSDeleteFunctionKey` (⌦) and `Backspace` `NSDeleteCharacter` (⌫) on
/// macOS, `VK_DELETE`/`VK_BACK` on Windows. A key with no platform
/// equivalent fails `MenuBar::new` with `UnmappableKey`, which panics.
fn menu_shortcut(shortcut: &Shortcut) -> MenuShortcut {
    let mut mods = Modifiers::empty();
    if shortcut.modifiers.command() {
        mods |= Modifiers::COMMAND;
    }
    if shortcut.modifiers.control() {
        mods |= Modifiers::CONTROL;
    }
    if shortcut.modifiers.option() {
        mods |= Modifiers::ALT;
    }
    if shortcut.modifiers.shift() {
        mods |= Modifiers::SHIFT;
    }
    let key = match shortcut.key.to_key() {
        // An uppercase character equivalent implies Shift; Shift comes only
        // from the shortcut's modifiers.
        Key::Character(character) => Key::Character(character.to_lowercase()),
        named @ Key::Named(_) => named,
    };
    MenuShortcut::new(key, mods)
}

fn command_title(command: &ResolvedCommand) -> String {
    command.label.content.snapshot().to_plain().to_string()
}

/// Builds one bar [`Command`]: `checked` reflects `selected` (the gutter
/// is invisible while unselected on every platform, and `set_checked`
/// reflects it live — the same validation contract a mounted `Menu` gives
/// through the registry's per-dispatch snapshot). `accelerator` is the
/// row's key equivalent — normally the command's own `shortcut` mapped
/// through [`menu_shortcut`]; the Windows Close item passes the system's
/// Alt+F4 instead.
fn build_command(
    title: &str,
    command: &ResolvedCommand,
    accelerator: Option<MenuShortcut>,
    ctx: &mut Builder,
) -> Command {
    let id = ctx.alloc_id();
    let mut item = Command::new(id, title)
        .enabled(!command.disabled.snapshot())
        .checked(command.selected.snapshot());
    if let Some(accelerator) = accelerator {
        item = item.shortcut(accelerator);
    }
    ctx.actions.insert(id, command.action.clone());
    let target = ctx.target.clone();
    ctx.state_watches
        .push(command.disabled.watch(move |watch_ctx| {
            let enabled = !watch_ctx.into_value();
            target.update(|bar| bar.set_enabled(id, enabled));
        }));
    let target = ctx.target.clone();
    ctx.state_watches
        .push(command.selected.watch(move |watch_ctx| {
            let selected = watch_ctx.into_value();
            target.update(|bar| bar.set_checked(id, selected));
        }));
    item
}

/// Builds `command`'s item with its own shortcut as the key equivalent.
fn declared_command(title: &str, command: &ResolvedCommand, ctx: &mut Builder) -> Command {
    build_command(
        title,
        command,
        command.shortcut.as_ref().map(menu_shortcut),
        ctx,
    )
}

/// Appends resolved items to `submenu` — `Submenu::entry` is a builder, so
/// the loop rebinds the accumulator.
fn extend_submenu(mut submenu: Submenu, items: &[ResolvedMenuItem], ctx: &mut Builder) -> Submenu {
    for item in items {
        match item {
            ResolvedMenuItem::Command(command) => {
                #[cfg(target_os = "macos")]
                command.assert_allowed_in_macos_menu_bar();
                submenu = submenu.entry(declared_command(&command_title(command), command, ctx));
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
                if let Some(command) = ctx.env.get::<Quit>().map(|quit| quit.command(ctx.env)) {
                    submenu = submenu.entry(declared_command(
                        &format!("&{}", command_title(&command)),
                        &command,
                        ctx,
                    ));
                }
            }
            ResolvedMenuItem::CloseWindow => {
                submenu = submenu.entry(close_window_item(ctx));
            }
            ResolvedMenuItem::Divider => {
                submenu = submenu.entry(Entry::Separator);
            }
            ResolvedMenuItem::Menu(nested) => {
                submenu = submenu.entry(build_nested(nested, ctx));
            }
        }
    }
    submenu
}

/// The standard Close Window item on macOS — an ordinary command carrying
/// the application's decided chord. Choosing it, or pressing its
/// accelerator, reports the `CommandId`; [`NativeMenuBar::pump_menu_events`]
/// runs the action, which asks the focused window to close through
/// [`WindowCloser`](crate::renderer::WindowCloser) — the one close path
/// the registry's chords share.
#[cfg(target_os = "macos")]
fn close_window_item(ctx: &mut Builder) -> Command {
    let command = close_window_command(ctx.env, true)
        .expect("the winit runner installs a WindowCloser before the menu bar builds");
    declared_command(&command_title(&command), &command, ctx)
}

/// The standard Close Window item on Windows — titled `&Close` (the `&`
/// access key the platform's own Close carries) and showing Alt+F4, the
/// system close chord, which the window manager handles and which reaches
/// the same close path. Its activation asks the bar's own window to close
/// through the `WindowCloser` request — `MenuBar::attach` makes "the window
/// the menu belongs to" exact, the target `WM_CLOSE` on that window would
/// name. The application's decided close chord still answers through the
/// chord registry.
#[cfg(target_os = "windows")]
fn close_window_item(ctx: &mut Builder) -> Command {
    let command = close_window_command(ctx.env, true)
        .expect("the winit runner installs a WindowCloser before the menu bar builds");
    build_command(
        "&Close",
        &command,
        Some(MenuShortcut::new(Key::Named(NamedKey::F4), Modifiers::ALT)),
        ctx,
    )
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

/// A resolved menu's title in plain text.
fn menu_title(menu: &ResolvedNestedMenu) -> String {
    menu.label.content.snapshot().to_plain().to_string()
}

/// A resolved `Menu`'s `Submenu`, its items resolved recursively.
fn build_nested(nested: &ResolvedNestedMenu, ctx: &mut Builder) -> Submenu {
    extend_submenu(
        Submenu::new(menu_title(nested)),
        &nested.items.snapshot(),
        ctx,
    )
}

/// The resolved top-level menus. `resolve_menu_bar_items` produces only
/// `Menu` items at the top level, so a leaf there is the resolver's
/// contract broken, not a case to render.
fn top_level_menus(items: &[ResolvedMenuItem]) -> impl Iterator<Item = &ResolvedNestedMenu> {
    items.iter().map(|item| {
        let ResolvedMenuItem::Menu(menu) = item else {
            unreachable!("`resolve_menu_bar_items` produces only `Menu` items at the top level");
        };
        menu
    })
}

/// Watches the items signal of every resolved `Menu` — the top-level ones
/// and every nested one — into a full rebuild: a menu's own list can change
/// without the top-level list re-emitting. Registered once per rebuild,
/// however many bars it builds.
fn watch_structure(
    menus: &[ResolvedMenuItem],
    slot: &Slot,
    top_items: &Computed<Vec<ResolvedMenuItem>>,
    env: &Environment,
    watches: &mut Vec<BoxWatcherGuard>,
) {
    for item in menus {
        let ResolvedMenuItem::Menu(menu) = item else {
            continue;
        };
        let rebuild_slot = Rc::clone(slot);
        let rebuild_items = top_items.clone();
        let rebuild_env = env.clone();
        watches.push(menu.items.watch(move |_| {
            rebuild_bar(&rebuild_slot, &rebuild_items, &rebuild_env);
        }));
        watch_structure(&menu.items.snapshot(), slot, top_items, env, watches);
    }
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
/// product-named top-level menu.
#[cfg(target_os = "macos")]
fn build_app_menu(
    product_name: &str,
    declared: Option<&ResolvedNestedMenu>,
    ctx: &mut Builder,
) -> Submenu {
    let mut app_menu = Submenu::new(product_name).entry(StandardItem::About {
        name: product_name.to_owned(),
    });
    if let Some(declared) = declared {
        let declared_items = declared.items.snapshot();
        if !declared_items.is_empty() {
            app_menu = extend_submenu(app_menu.entry(Entry::Separator), &declared_items, ctx);
        }
    }
    app_menu
        .entry(Entry::Separator)
        .entry(StandardItem::Services)
        .entry(Entry::Separator)
        .entry(StandardItem::Hide {
            name: product_name.to_owned(),
        })
        .entry(StandardItem::HideOthers)
        .entry(StandardItem::ShowAll)
        .entry(Entry::Separator)
        .entry(StandardItem::Quit {
            name: product_name.to_owned(),
        })
}

/// The standard Window menu — marked `windows_menu` so `MenuBar::install`
/// registers it as `NSApp.windowsMenu`, which is what puts the live window
/// list on it. Close — armed with the application's decided chord —
/// appears only when no declared menu carries it (the
/// `CloseWindowPlacement` rule every backend shares); then the platform's
/// Minimize, Zoom and Bring All to Front items.
#[cfg(target_os = "macos")]
fn build_window_menu(items: &[ResolvedMenuItem], ctx: &mut Builder) -> Submenu {
    let mut window_menu = Submenu::new("Window").windows_menu();
    if CloseWindowPlacement::for_declared(items) == CloseWindowPlacement::WindowMenu {
        window_menu = window_menu.entry(close_window_item(ctx));
    }
    window_menu
        .entry(StandardItem::Minimize)
        .entry(StandardItem::Zoom)
        .entry(Entry::Separator)
        .entry(StandardItem::BringAllToFront)
}

/// Turns the walked menus into a `MenuBar`. Building the app's own menu
/// tree cannot fail — a `MenuError` is a bug and panics.
fn new_menu_bar(menus: Vec<Submenu>) -> MenuBar {
    MenuBar::new(menus).unwrap_or_else(|error| {
        panic!("the resolved menu bar cannot be expressed on this platform: {error}")
    })
}

/// Builds the macOS bar — the standard application menu first (the
/// declared application menu, the first top-level menu titled by the
/// product name, folded into it), the other declared top-level menus, then
/// the marked Window menu — and installs it as `NSApp.mainMenu`.
#[cfg(target_os = "macos")]
fn build_bar(
    items: &[ResolvedMenuItem],
    env: &Environment,
    slot: &Slot,
    structure_watches: Vec<BoxWatcherGuard>,
) -> Bar {
    let product_name = product_name();
    let mut ctx = Builder::new(
        env,
        BarRef {
            slot: Rc::clone(slot),
        },
    );
    let mut app_menu_declared = None;
    let mut others = Vec::with_capacity(items.len());
    for menu in top_level_menus(items) {
        if app_menu_declared.is_none() && menu_title(menu) == product_name {
            app_menu_declared = Some(menu);
        } else {
            others.push(menu);
        }
    }
    let mut menus = Vec::with_capacity(items.len() + 2);
    menus.push(build_app_menu(&product_name, app_menu_declared, &mut ctx));
    for menu in others {
        menus.push(build_nested(menu, &mut ctx));
    }
    menus.push(build_window_menu(items, &mut ctx));
    let bar = new_menu_bar(menus);
    bar.install(
        objc2::MainThreadMarker::new()
            .expect("the winit runner builds the menu bar on the main thread"),
    );
    Bar {
        dispatch: ctx.into_dispatch(&bar),
        native: bar,
        _structure_watches: structure_watches,
    }
}

/// Builds every listed window's bar from the same resolved `items`.
#[cfg(target_os = "windows")]
fn build_bar(
    items: &[ResolvedMenuItem],
    env: &Environment,
    slot: &Slot,
    structure_watches: Vec<BoxWatcherGuard>,
    windows: &[(isize, WindowId)],
) -> Bar {
    Bar {
        windows: windows
            .iter()
            .map(|&(hwnd, window)| (hwnd, build_window_bar(items, hwnd, window, env, slot)))
            .collect(),
        _structure_watches: structure_watches,
    }
}

/// Builds one window's bar on Windows and attaches it to `hwnd`.
#[cfg(target_os = "windows")]
fn build_window_bar(
    items: &[ResolvedMenuItem],
    hwnd: isize,
    window: WindowId,
    env: &Environment,
    slot: &Slot,
) -> WindowBar {
    let mut ctx = Builder::new(
        env,
        BarRef {
            slot: Rc::clone(slot),
            hwnd,
        },
    );
    let menus = top_level_menus(items)
        .map(|menu| build_nested(menu, &mut ctx))
        .collect();
    let bar = Rc::new(new_menu_bar(menus));
    // SAFETY: the reference points at the `MenuBar` inside the `Rc`
    // allocation, which `WindowBar::bar` keeps alive and never clones or
    // hands out mutably. `_attachment`, the only holder of the extended
    // borrow, is declared before `bar` in `WindowBar`, so it drops — and
    // stops using the borrow — before the `Rc` frees the bar.
    let bar_ref: &'static MenuBar = unsafe { &*Rc::as_ptr(&bar) };
    // `hwnd` is the live window the runner just created on this thread;
    // `attach` itself rejects anything else.
    let attachment = bar_ref
        .attach(HWND(hwnd as *mut _))
        .expect("attaching the menu bar to a live HWND failed");
    WindowBar {
        _attachment: attachment,
        dispatch: ctx.into_dispatch(&bar),
        bar,
        window,
    }
}

/// Rebuilds the whole native bar in `slot` from a fresh `top_items`
/// snapshot. Called on install, by the top-level items watch and by each
/// menu's items watch.
///
/// The old bar is dropped before the new one is built. Windows requires
/// that order: dropping an `Attachment` removes the window's menu subclass
/// and `SetMenu(hwnd, NULL)`s it whichever bar installed them, so dropping
/// the old bars after the new ones attached would strip the new ones. On
/// macOS `install` replaces `NSApp.mainMenu` either way.
fn rebuild_bar(slot: &Slot, top_items: &Computed<Vec<ResolvedMenuItem>>, env: &Environment) {
    let mut guard = slot.borrow_mut();
    let old = guard.take();
    // The windows the rebuilt bars attach to: those still attached —
    // `detach_hwnd` already forgot the closed ones.
    #[cfg(target_os = "windows")]
    let windows: Vec<(isize, WindowId)> = old.as_ref().map_or_else(Vec::new, |old| {
        old.windows
            .iter()
            .map(|(&hwnd, window_bar)| (hwnd, window_bar.window))
            .collect()
    });
    drop(old);
    let items = top_items.snapshot();
    let mut structure_watches = Vec::new();
    watch_structure(&items, slot, top_items, env, &mut structure_watches);
    #[cfg(target_os = "macos")]
    let fresh = build_bar(&items, env, slot, structure_watches);
    #[cfg(target_os = "windows")]
    let fresh = build_bar(&items, env, slot, structure_watches, &windows);
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

        let bar: Slot = Rc::new(RefCell::new(None));
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
            #[cfg(target_os = "windows")]
            top_items: items.clone(),
            _watch: watch,
        }
    }

    /// Attaches a fresh `MenuBar` built from the current resolved items to
    /// a Windows application window's `HWND` (`SetMenu` plus the
    /// `WM_COMMAND` subclass). `window` is the window its commands act on.
    /// Called by the runner for each application window as it is created;
    /// a later rebuild re-attaches to every recorded `HWND`.
    #[cfg(target_os = "windows")]
    pub(crate) fn attach_hwnd(&self, hwnd: isize, window: WindowId) {
        let window_bar = build_window_bar(
            &self.top_items.snapshot(),
            hwnd,
            window,
            &self.env,
            &self.bar,
        );
        let replaced = self
            .bar
            .borrow_mut()
            .as_mut()
            .expect("the menu bar is installed for the runner's lifetime")
            .windows
            .insert(hwnd, window_bar);
        // A replaced bar's attachment would detach the new one on drop.
        assert!(replaced.is_none(), "HWND {hwnd:#x} attached twice");
    }

    /// Forgets a Windows window's `HWND` — the runner calls it before the
    /// window (and its `HWND`) is destroyed. Dropping the window's bar
    /// detaches the `HMENU` and removes the subclass while the handle is
    /// still live, so no attachment ever outlives its window. A `HWND` that
    /// was never attached (a popup window) is a no-op.
    #[cfg(target_os = "windows")]
    pub(crate) fn detach_hwnd(&self, hwnd: isize) {
        let removed = self
            .bar
            .borrow_mut()
            .as_mut()
            .expect("the menu bar is installed for the runner's lifetime")
            .windows
            .remove(&hwnd);
        // Dropping the window's bar detaches it.
        drop(removed);
    }

    /// Drains the bar's `events()` stream and dispatches each item's action
    /// through `call_action_discarding_result` — the same entry the
    /// `MenuShortcutRegistry` uses. Called once per event-loop pass. The
    /// `WindowId` an action extracts is `focused` — a menu-bar command acts
    /// on the focused window, as an app-bar chord's action acts on the
    /// window it dispatched for; `WindowId::Orphan` while none is focused,
    /// which the shared close path resolves to no window.
    #[cfg(target_os = "macos")]
    pub(crate) fn pump_menu_events(&self, focused: Option<WindowId>) {
        let env = self.env.extending(focused.unwrap_or(WindowId::Orphan));
        loop {
            // The action may mutate menu state (which rebuilds the bar and
            // borrows this cell again) — clone it under the borrow, then
            // dispatch after releasing.
            let action = {
                let mut slot = self.bar.borrow_mut();
                let dispatch = &mut slot
                    .as_mut()
                    .expect("the menu bar is installed for the runner's lifetime")
                    .dispatch;
                let Some(id) = dispatch.events.next().now_or_never().flatten() else {
                    break;
                };
                dispatch_action(&dispatch.actions, id)
            };
            call_action_discarding_result(&action, &env);
        }
    }

    /// Drains every attached window's `events()` stream and dispatches each
    /// item's action through `call_action_discarding_result` — the same
    /// entry the `MenuShortcutRegistry` uses. Called once per event-loop
    /// pass. Each activation names its bar's own window — `attach` ties a
    /// bar to one `HWND`, so the `WindowId` an action extracts is exactly
    /// the window whose menu was chosen, which is also the window
    /// `WM_CLOSE` would name for Close Window.
    #[cfg(target_os = "windows")]
    pub(crate) fn pump_menu_events(&self) {
        loop {
            // The action may mutate menu state (which rebuilds the bar and
            // borrows this cell again) — dispatch after releasing.
            let dispatched = self
                .bar
                .borrow_mut()
                .as_mut()
                .expect("the menu bar is installed for the runner's lifetime")
                .windows
                .values_mut()
                .find_map(|window_bar| {
                    let dispatch = &mut window_bar.dispatch;
                    let id = dispatch.events.next().now_or_never().flatten()?;
                    Some((dispatch_action(&dispatch.actions, id), window_bar.window))
                });
            let Some((action, window)) = dispatched else {
                break;
            };
            call_action_discarding_result(&action, &self.env.extending(window));
        }
    }
}

/// The action of a reported `CommandId`. A bar reports only the ids its
/// builder allocated, and every allocated id has an action.
fn dispatch_action(
    actions: &HashMap<CommandId, SharedAction<()>>,
    id: CommandId,
) -> SharedAction<()> {
    actions
        .get(&id)
        .unwrap_or_else(|| panic!("the menu bar reported {id:?}, which it never built"))
        .clone()
}

// `MenuBar::attach` checks `IsWindow`, so the attach/detach bookkeeping runs
// against real windows — the runner's carry semantics end to end: attach
// each created window, forget each closed one before its `HWND` dies, and a
// rebuild re-attaches only the still-open windows.
#[cfg(all(test, target_os = "windows"))]
mod tests {
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, GetMenu, RegisterClassW,
        WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPED,
    };
    use windows::core::PCWSTR;

    use super::*;

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: forwards every message to the default procedure.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// `count` live overlapped windows on this thread for bars to attach
    /// to; the caller destroys them.
    fn test_windows(count: usize) -> Vec<HWND> {
        let class_name: Vec<u16> = "HydrolysisMenuBarTest".encode_utf16().chain([0]).collect();
        // SAFETY: registers a private window class — a registration an
        // earlier test in this process left is reused — and creates
        // windows of it on this thread; `class_name` outlives every call.
        unsafe {
            let instance = GetModuleHandleW(None).expect("GetModuleHandleW failed");
            let class = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            let _ = RegisterClassW(&raw const class);
            (0..count)
                .map(|_| {
                    CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        PCWSTR(class_name.as_ptr()),
                        PCWSTR::null(),
                        WS_OVERLAPPED,
                        CW_USEDEFAULT,
                        CW_USEDEFAULT,
                        CW_USEDEFAULT,
                        CW_USEDEFAULT,
                        None,
                        None,
                        Some(instance.into()),
                        None,
                    )
                    .expect("CreateWindowExW failed")
                })
                .collect()
        }
    }

    fn has_menu(hwnd: HWND) -> bool {
        // SAFETY: `hwnd` is a live window this test created.
        !unsafe { GetMenu(hwnd) }.0.is_null()
    }

    #[test]
    fn a_rebuild_reattaches_only_the_open_windows() {
        let items: Computed<Vec<ResolvedMenuItem>> = Computed::constant(Vec::new());
        let env = Environment::new();
        let native = NativeMenuBar::install(&items, &env);
        let hwnds = test_windows(3);
        let key = |hwnd: HWND| hwnd.0 as isize;
        for (ordinal, &hwnd) in (0_u64..).zip(&hwnds) {
            native.attach_hwnd(key(hwnd), WindowId::Runner(ordinal));
        }

        // Window 2 closes — the runner detaches it before destroy.
        native.detach_hwnd(key(hwnds[1]));
        assert!(!has_menu(hwnds[1]), "detaching takes the window's menu off");
        native.detach_hwnd(key(hwnds[1])); // detaching twice is a no-op
        native.detach_hwnd(9); // an HWND never attached is a no-op

        rebuild_bar(&native.bar, &items, &env);
        let mut carried: Vec<isize> = native
            .bar
            .borrow()
            .as_ref()
            .expect("a bar is installed")
            .windows
            .keys()
            .copied()
            .collect();
        carried.sort_unstable();
        let mut open = vec![key(hwnds[0]), key(hwnds[2])];
        open.sort_unstable();
        assert_eq!(carried, open, "the rebuild keeps only the open windows");
        assert!(
            has_menu(hwnds[0]) && has_menu(hwnds[2]),
            "the rebuilt bars stay attached after the old ones drop"
        );

        for &hwnd in &hwnds {
            native.detach_hwnd(key(hwnd));
            // SAFETY: `hwnd` is a window this test created on this thread.
            unsafe { DestroyWindow(hwnd) }.expect("DestroyWindow failed");
        }
    }
}
