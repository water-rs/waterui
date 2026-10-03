//! `App::menu_bar` plumbing shared by the runners.
//!
//! Every runner that destructures the app used to drop `menu_bar`, so an
//! app-level menu bar rendered nothing and armed no shortcuts. The runner
//! now resolves the menus once — `resolve_menu_bar_items` keeps the signal
//! reactive, so item edits apply on the next lookup — and registers their
//! command chords on the window-shared [`MenuShortcutRegistry`] as an
//! app-scoped source (live for the app's duration, answering to whichever
//! window dispatches — the same behaviour a mounted `Menu` gives, minus
//! the window affinity).
//!
//! Rendering is a separate decision the *runner* makes, not the platform
//! triple: `register_menu_bar` returns the resolved items, and only the
//! winit runner turns them into a native surface by calling
//! `platform::native_menu_bar::NativeMenuBar::install` and keeping the
//! result for the app's duration. What hydrolysis renders per platform:
//!
//! - **winit on macOS** — `NSApp.mainMenu` is the system menu bar: the
//!   menus become real `NSMenu` items via `muda`, commands carrying their
//!   `shortcut` as the AppKit key equivalent. AppKit matches a key
//!   equivalent in `-[NSApplication sendEvent]` before the event is
//!   delivered as `keyDown` to the window (winit's view does not override
//!   `performKeyEquivalent`), so a claimed accelerator never reaches the
//!   registry. The registry stays armed as the fallback for chords muda
//!   could not express or AppKit did not claim — the two paths see
//!   disjoint keys, so a chord still fires exactly once.
//! - **winit on Windows** — every window owns a Win32 menu bar: `muda`
//!   builds the `HMENU` and attaches it to each application window's
//!   `HWND`, with the shortcuts as accelerator text. Stock winit's message
//!   pump never calls `TranslateAcceleratorW`, so the accelerators are
//!   display-only and the registry owns the chords: click through muda,
//!   chord through the registry — exactly once.
//! - **winit on Linux, the web runner, and the headless hosts** — no
//!   menu-bar surface (winit exposes none on Linux; a browser page cannot
//!   own the browser's menus; a headless host has no chrome at all).
//!   Shortcuts arm on every window and nothing renders — matching the
//!   gtk-backend, which also does not surface `menu_bar` on Linux.
//!
//! Whichever path renders a native menu, choosing an item posts a
//! `muda::MenuEvent` on muda's channel; the winit runner drains it once
//! per event-loop pass and runs the command's `SharedAction` through
//! `call_action_discarding_result` with the app env — the same dispatch
//! the registry uses for chords.

use nami::Computed;
use waterui_controls::menu::{Menu, ResolvedMenuItem, resolve_menu_bar_items};
use waterui_core::Environment;

use crate::renderer::{MISSING_MENU_SHORTCUT_REGISTRY, MenuShortcutRegistry};

/// Resolves `menu_bar` against `env` and registers its command chords as
/// an app-scoped source on the window-shared registry (the app `env` is
/// what the actions are invoked with, layered over whichever window
/// dispatches — same as a mounted `Menu`'s). Returns the resolved items
/// signal for whichever surface consumes it: the winit runner feeds it to
/// `NativeMenuBar::install` on macOS and Windows; runners with no menu-bar
/// surface just arm the chords and drop it.
pub(crate) fn register_menu_bar(
    menu_bar: &Computed<Vec<Menu>>,
    env: &Environment,
) -> Computed<Vec<ResolvedMenuItem>> {
    let items = resolve_menu_bar_items(menu_bar, env);
    env.get::<MenuShortcutRegistry>()
        .expect(MISSING_MENU_SHORTCUT_REGISTRY)
        .register_menu_bar(items.clone(), env.clone());
    items
}
