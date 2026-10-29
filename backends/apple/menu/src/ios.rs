//! The iPadOS and iOS main menu: the declared content fills the builder the
//! main menu system hands its build handler, and a watch on the resolved
//! items requests each rebuild.
//!
//! A command with a shortcut becomes a `UIKeyCommand`, armed in the scope
//! the builder carries; `UIKit` matches it before the key reaches a
//! responder's `pressesBegan`, so a chord the bar claims never reaches the
//! application as a key event.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use cocoa_ui::MainThreadMarker;
use cocoa_ui::uikit::MenuBuilder;
use nami::{Computed, Signal};
use waterui_controls::menu::{Menu as DeclaredMenu, ResolvedMenuItem, resolve_menu_bar_items};
use waterui_core::Environment;

use crate::{kit_command_for_menu, menu_tree};

/// What [`install_declared`] shares with the [`build_handler`]: the resolved
/// items plus the environment their actions run under, filled once the
/// application has returned its own environment.
pub type Declared = Rc<RefCell<Option<(Computed<Vec<ResolvedMenuItem>>, Environment)>>>;

/// The slot `build_menus` reads — empty until the application has run.
#[must_use]
pub fn declared() -> Declared {
    Rc::new(RefCell::new(None))
}

/// The `build_menus` handler: each top-level declared `Menu` becomes a
/// `UIMenu` identified `dev.waterui.menu.{index}` at the end of the root
/// bar, replaced in place on rebuild.
///
/// # Panics
///
/// When a declared top-level item is not a `Menu`.
pub fn build_handler(declared: Declared) -> impl Fn(&MenuBuilder<'_>) + 'static {
    move |builder| {
        let Some((resolved, env)) = declared.borrow().clone() else {
            tracing::debug!("main menu built before the application declared its menus");
            return;
        };
        let mtm = MainThreadMarker::new().expect("build_menus runs on the main thread");
        let items = resolved.snapshot();
        for (index, item) in items.iter().enumerate() {
            let ResolvedMenuItem::Menu(menu) = item else {
                panic!("App::menu_bar only accepts top-level Menu values");
            };
            let identifier = format!("dev.waterui.menu.{index}");
            let command = kit_command_for_menu(menu);
            let nodes = menu_tree(&menu.items.snapshot(), &env);
            let ui_menu = cocoa_ui::uikit::menu_with_identifier(
                mtm,
                builder.key_commands(),
                &command,
                Some(&identifier),
                &nodes,
            );
            if builder.contains(&identifier) {
                builder.replace(&identifier, &ui_menu);
            } else {
                builder.insert_at_root_end(&ui_menu);
            }
        }
        tracing::debug!(
            menus = items.len(),
            "main menu built with the declared menus"
        );
    }
}

/// Fills `declared` and flags each change for rebuild. The returned guard
/// keeps the watcher alive; hold it for the process.
pub fn install_declared(
    mtm: MainThreadMarker,
    menu_bar: &Computed<Vec<DeclaredMenu>>,
    env: &Environment,
    declared: &Declared,
) -> Box<dyn Any> {
    let resolved = resolve_menu_bar_items(menu_bar, env);
    let guard = resolved.watch(move |_| cocoa_ui::uikit::request_main_menu_rebuild(mtm));
    *declared.borrow_mut() = Some((resolved, env.clone()));
    // `UIKit` may have built the main menu before the slot was filled; the
    // watcher only flags later changes.
    cocoa_ui::uikit::request_main_menu_rebuild(mtm);
    Box::new(guard)
}
