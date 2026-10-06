//! The application's menu bar.
//!
//! `WaterUIMainMenu.create()` + `WuiRootContext`'s
//! `installMenuBar`/`menuBarDidChange` ported: the standard App, Edit (the
//! responder-chain items keyboard shortcuts route through) and Window menus,
//! with the declared `menu_bar` content appended — macOS rebuilds the whole
//! bar on every change, iOS rebuilds through `application:buildMenuWith:`.
//!
//! The standard Window menu carries Close (⌘W) unless a declared menu
//! places `MenuItem::CloseWindow` itself — [`CloseWindowPlacement`] decides
//! which, from the declared items alone.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use waterui::component::menu::{
    CommandRole, Menu as DeclaredMenu, ResolvedCommand, ResolvedMenuItem, ResolvedNestedMenu,
};
use waterui::reactive::{Computed, Signal};
use waterui_backend_core::Environment;

/// Resolves `menu_bar` against `env` — items live-resolve under the
/// environment's locale.
#[must_use]
fn resolve(
    menu_bar: &Computed<Vec<DeclaredMenu>>,
    env: &Environment,
) -> Computed<Vec<ResolvedMenuItem>> {
    waterui::component::menu::resolve_menu_bar_items(menu_bar, env)
}

/// A resolved command as the kit's shared `Command` payload — label text,
/// subtitle, symbol name, destructive flag and shortcut.
fn kit_command(command: &ResolvedCommand) -> cocoa_ui::menu::Command {
    kit_command_fields(
        command.label.content.snapshot().to_plain().to_string(),
        command.subtitle.as_ref().map(ToString::to_string),
        command.icon.as_ref().map(|icon| icon.name.to_string()),
        command.role,
        command.shortcut.as_ref(),
    )
}

/// A resolved nested menu's header as the kit's shared `Command` payload.
pub fn kit_command_for_menu(menu: &ResolvedNestedMenu) -> cocoa_ui::menu::Command {
    kit_command_fields(
        menu.label.content.snapshot().to_plain().to_string(),
        None,
        menu.icon.as_ref().map(|icon| icon.name.to_string()),
        CommandRole::Standard,
        None,
    )
}

fn kit_command_fields(
    label: String,
    subtitle: Option<String>,
    symbol: Option<String>,
    role: CommandRole,
    shortcut: Option<&waterui::component::menu::Shortcut>,
) -> cocoa_ui::menu::Command {
    let mut modifiers = cocoa_ui::menu::KeyModifiers::empty();
    let mut key_equivalent = String::new();
    if let Some(shortcut) = shortcut {
        key_equivalent = shortcut.key.to_string();
        if shortcut.modifiers.command() {
            modifiers |= cocoa_ui::menu::KeyModifiers::COMMAND;
        }
        if shortcut.modifiers.shift() {
            modifiers |= cocoa_ui::menu::KeyModifiers::SHIFT;
        }
        if shortcut.modifiers.option() {
            modifiers |= cocoa_ui::menu::KeyModifiers::OPTION;
        }
        if shortcut.modifiers.control() {
            modifiers |= cocoa_ui::menu::KeyModifiers::CONTROL;
        }
    }
    cocoa_ui::menu::Command {
        label,
        subtitle,
        symbol,
        destructive: matches!(role, CommandRole::Destructive),
        enabled: true,
        selected: false,
        key_equivalent,
        modifiers,
    }
}

/// The declared menu bar's resolved items as the kit's shared
/// `MenuTreeNode` list, with each command's action bound to fire under
/// `env` — the content both the macOS and the iOS menu bar build from.
/// On macOS each command is checked against the chords the standard
/// application menu reserves.
pub fn menu_tree(
    items: &[ResolvedMenuItem],
    env: &Environment,
) -> Vec<cocoa_ui::menu::MenuTreeNode> {
    items
        .iter()
        .filter_map(|item| match item {
            ResolvedMenuItem::Divider => Some(cocoa_ui::menu::MenuTreeNode::Divider),
            ResolvedMenuItem::Command(command) => {
                #[cfg(target_os = "macos")]
                command.assert_allowed_in_macos_menu_bar();
                let action = command.action.clone();
                let env = env.clone();
                Some(cocoa_ui::menu::MenuTreeNode::Command(
                    kit_command(command),
                    Rc::new(move || {
                        action.call(&env);
                    }),
                ))
            }
            ResolvedMenuItem::Menu(menu) => Some(cocoa_ui::menu::MenuTreeNode::Submenu(
                kit_command_for_menu(menu),
                menu_tree(&menu.items.snapshot(), env),
            )),
            // macOS: the standard application menu already ends with Quit
            // (`build_default`), so a declared one is dropped rather than
            // shown twice. iOS has no application quit, so it is omitted.
            ResolvedMenuItem::Quit => None,
            // macOS: the standard Close item where it was declared — the
            // Window menu then leaves its own out (`CloseWindowPlacement`).
            #[cfg(target_os = "macos")]
            ResolvedMenuItem::CloseWindow => Some(standard_close_window_node()),
            // iOS: the system owns every scene's window, so a declared Close
            // Window is omitted.
            #[cfg(not(target_os = "macos"))]
            ResolvedMenuItem::CloseWindow => None,
        })
        .collect()
}

/// Where the macOS menu bar's Close Window item lives: in the standard
/// Window menu the backend builds, or only where the application declared
/// `MenuItem::CloseWindow` — never both, so it never appears twice.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseWindowPlacement {
    /// No declared menu carries `MenuItem::CloseWindow`: the standard
    /// Window menu carries Close (⌘W), so ⌘W closes the key window without
    /// the application declaring anything.
    WindowMenu,
    /// A declared menu carries it, at whatever depth; the Window menu does
    /// not repeat it.
    Declared,
}

#[cfg(target_os = "macos")]
impl CloseWindowPlacement {
    /// The placement the declared menu bar's resolved `items` call for.
    #[must_use]
    pub fn for_declared(items: &[ResolvedMenuItem]) -> Self {
        if ResolvedMenuItem::declares_close_window(items) {
            Self::Declared
        } else {
            Self::WindowMenu
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use imp::*;

#[cfg(target_os = "macos")]
mod imp {
    use alloc::boxed::Box;
    use alloc::rc::Rc;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::any::Any;

    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::appkit::{Application, KeyModifiers, Menu, MenuAction, MenuItem};
    use waterui::component::menu::Menu as DeclaredMenu;
    use waterui::reactive::{Computed, Signal};
    use waterui_backend_core::Environment;

    use super::{CloseWindowPlacement, menu_tree, resolve};

    /// What this application calls itself in its own menu: the display name a
    /// bundle chooses for people to read, then the bundle name, then the
    /// process name — `WaterUIMainMenu.appName`'s order.
    fn app_name() -> String {
        cocoa_ui::bundle::info_string("CFBundleDisplayName")
            .or_else(|| cocoa_ui::bundle::info_string("CFBundleName"))
            .unwrap_or_else(cocoa_ui::process::name)
    }

    /// The standard Quit item's title.
    fn quit_title() -> String {
        alloc::format!("Quit {}", app_name())
    }

    /// The standard Quit item — ⌘Q sending `terminate:` to the application —
    /// that ends the application menu. A declared `MenuItem::Quit` in a menu
    /// a window mounts renders as this same item.
    pub fn standard_quit_item(mtm: MainThreadMarker) -> MenuItem {
        MenuItem::new(mtm, &quit_title(), Some(MenuAction::Quit), "q")
    }

    /// The standard Quit item as a kit menu-tree node, for the menus built
    /// from `MenuTreeNode`s (context menus): the same title and chord, its
    /// action the same `terminate:` the application menu's item sends.
    pub fn standard_quit_node() -> cocoa_ui::menu::MenuTreeNode {
        cocoa_ui::menu::MenuTreeNode::Command(
            cocoa_ui::menu::Command {
                label: quit_title(),
                enabled: true,
                key_equivalent: String::from("q"),
                modifiers: KeyModifiers::COMMAND,
                ..cocoa_ui::menu::Command::default()
            },
            Rc::new(|| {
                let mtm = MainThreadMarker::new().expect("menu actions run on the main thread");
                Application::shared(mtm).terminate();
            }),
        )
    }

    /// The standard Close Window item's title — `AppKit`'s own word for
    /// it, untranslated like the other standard items' titles here.
    const CLOSE_WINDOW_TITLE: &str = "Close";

    /// The standard Close Window item — ⌘W sending `performClose:` up the
    /// responder chain, so the key window closes as its close button would
    /// close it, and `AppKit` disables the item while no window can close.
    /// The Window menu carries it unless the application declares
    /// `MenuItem::CloseWindow`; a declared one in a menu a window mounts
    /// renders as this same item.
    pub fn standard_close_window_item(mtm: MainThreadMarker) -> MenuItem {
        MenuItem::new(mtm, CLOSE_WINDOW_TITLE, Some(MenuAction::CloseWindow), "w")
    }

    /// The standard Close Window item as a kit menu-tree node, for the
    /// menus built from `MenuTreeNode`s (the declared menu bar, context
    /// menus): the same title and chord, its action the same
    /// `performClose:` sent up the responder chain.
    pub fn standard_close_window_node() -> cocoa_ui::menu::MenuTreeNode {
        cocoa_ui::menu::MenuTreeNode::Command(
            cocoa_ui::menu::Command {
                label: String::from(CLOSE_WINDOW_TITLE),
                enabled: true,
                key_equivalent: String::from("w"),
                modifiers: KeyModifiers::COMMAND,
                ..cocoa_ui::menu::Command::default()
            },
            Rc::new(|| {
                let mtm = MainThreadMarker::new().expect("menu actions run on the main thread");
                if !Application::shared(mtm).send_action(MenuAction::CloseWindow) {
                    tracing::debug!("Close Window chosen with no window to close");
                }
            }),
        )
    }

    /// The standard menu bar's content: App, Edit, Window — the same menus
    /// `main.swift.tpl` installed before `app.run()`, with Close in the
    /// Window menu when `close_window` places it there. Declared menus
    /// append to it in [`install`].
    fn build_default(
        mtm: MainThreadMarker,
        application: &Application,
        close_window: CloseWindowPlacement,
    ) -> Menu {
        let name = app_name();
        let main = Menu::new(mtm, "");

        // App menu: About, Preferences, Services, Hide, Hide Others, Show All,
        // Quit.
        let app_menu = Menu::new(mtm, "");
        app_menu.add_item(MenuItem::new(
            mtm,
            &alloc::format!("About {name}"),
            Some(MenuAction::About),
            "",
        ));
        app_menu.add_separator();
        app_menu.add_item(MenuItem::new(mtm, "Preferences…", None, ","));
        app_menu.add_separator();
        let services = Menu::new(mtm, "Services");
        application.set_services_menu(&services);
        app_menu.add_item(MenuItem::new(mtm, "Services", None, "").with_submenu(&services));
        app_menu.add_separator();
        app_menu.add_item(MenuItem::new(
            mtm,
            &alloc::format!("Hide {name}"),
            Some(MenuAction::Hide),
            "h",
        ));
        app_menu.add_item(
            MenuItem::new(mtm, "Hide Others", Some(MenuAction::HideOthers), "h")
                .with_key_modifiers(KeyModifiers::COMMAND | KeyModifiers::OPTION),
        );
        app_menu.add_item(MenuItem::new(
            mtm,
            "Show All",
            Some(MenuAction::ShowAll),
            "",
        ));
        app_menu.add_separator();
        app_menu.add_item(standard_quit_item(mtm));
        main.add_item(MenuItem::new(mtm, "", None, "").with_submenu(&app_menu));

        // Edit menu: the responder-chain commands that make ⌘C/⌘V/⌘X/⌘A work
        // in text fields.
        let edit_menu = Menu::new(mtm, "Edit");
        edit_menu.add_item(MenuItem::new(mtm, "Undo", Some(MenuAction::Undo), "z"));
        edit_menu.add_item(
            MenuItem::new(mtm, "Redo", Some(MenuAction::Redo), "z")
                .with_key_modifiers(KeyModifiers::COMMAND | KeyModifiers::SHIFT),
        );
        edit_menu.add_separator();
        edit_menu.add_item(MenuItem::new(mtm, "Cut", Some(MenuAction::Cut), "x"));
        edit_menu.add_item(MenuItem::new(mtm, "Copy", Some(MenuAction::Copy), "c"));
        edit_menu.add_item(MenuItem::new(mtm, "Paste", Some(MenuAction::Paste), "v"));
        edit_menu.add_item(MenuItem::new(mtm, "Delete", Some(MenuAction::Delete), ""));
        edit_menu.add_item(MenuItem::new(
            mtm,
            "Select All",
            Some(MenuAction::SelectAll),
            "a",
        ));
        main.add_item(MenuItem::new(mtm, "Edit", None, "").with_submenu(&edit_menu));

        // Window menu: registered so AppKit fills it with the window list.
        // Close comes first, in the order of the title-bar buttons it
        // shares a window with: close, minimize, zoom.
        let window_menu = Menu::new(mtm, "Window");
        if close_window == CloseWindowPlacement::WindowMenu {
            window_menu.add_item(standard_close_window_item(mtm));
        }
        window_menu.add_item(MenuItem::new(
            mtm,
            "Minimize",
            Some(MenuAction::Minimize),
            "m",
        ));
        window_menu.add_item(MenuItem::new(mtm, "Zoom", Some(MenuAction::Zoom), ""));
        window_menu.add_separator();
        window_menu.add_item(MenuItem::new(
            mtm,
            "Bring All to Front",
            Some(MenuAction::BringAllToFront),
            "",
        ));
        application.set_windows_menu(&window_menu);
        main.add_item(MenuItem::new(mtm, "Window", None, "").with_submenu(&window_menu));

        main
    }

    /// The standard macOS menu bar: the default menus plus the declared
    /// `nodes` appended in order — `menuBarDidChange`'s full rebuild.
    /// `close_window` says whether the declared menus carry Close Window or
    /// the Window menu does.
    pub fn install(
        mtm: MainThreadMarker,
        application: &Application,
        nodes: &[cocoa_ui::menu::MenuTreeNode],
        close_window: CloseWindowPlacement,
    ) {
        let main = build_default(mtm, application, close_window);
        main.append_nodes(nodes);
        application.set_main_menu(&main);
    }

    /// The menu bar before `app(env)` reports its declared content: nothing
    /// is declared, so the Window menu carries Close.
    pub fn install_default(mtm: MainThreadMarker, application: &Application) {
        install(mtm, application, &[], CloseWindowPlacement::WindowMenu);
    }

    /// Installs `menu_bar` and rebuilds the bar on every change —
    /// `installMenuBar` + `menuBarDidChange`. The returned guard keeps the
    /// watcher alive; hold it for the process.
    pub fn install_declared(
        mtm: MainThreadMarker,
        application: &Application,
        menu_bar: &Computed<Vec<DeclaredMenu>>,
        env: &Environment,
    ) -> Box<dyn Any> {
        let resolved = resolve(menu_bar, env);
        let application = application.clone();
        let env = env.clone();
        let rebuild = {
            let resolved = resolved.clone();
            Rc::new(move || {
                let items = resolved.snapshot();
                let nodes = menu_tree(&items, &env);
                install(
                    mtm,
                    &application,
                    &nodes,
                    CloseWindowPlacement::for_declared(&items),
                );
            })
        };
        rebuild();
        let guard = resolved.watch(move |_| rebuild());
        Box::new(guard)
    }
}

/// iOS: the declared content fills the builder `application:buildMenuWith:`
/// hands the delegate; a watch on the resolved items requests each rebuild.
#[cfg(target_os = "ios")]
mod imp {
    use alloc::boxed::Box;
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::any::Any;
    use core::cell::RefCell;

    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::uikit::MenuBuilder;
    use waterui::component::menu::{Menu as DeclaredMenu, ResolvedMenuItem};
    use waterui::reactive::{Computed, Signal};
    use waterui_backend_core::Environment;

    use super::{kit_command_for_menu, menu_tree, resolve};

    /// What `install_declared` shares with the `build_menus` handler: the
    /// resolved items plus the environment their actions run under, filled
    /// once `app(env)` has returned its own environment.
    pub type Declared = Rc<RefCell<Option<(Computed<Vec<ResolvedMenuItem>>, Environment)>>>;

    /// The slot `build_menus` reads — empty until `app(env)` has run.
    #[must_use]
    pub fn declared() -> Declared {
        Rc::new(RefCell::new(None))
    }

    /// The `build_menus` handler: each top-level declared `Menu` becomes a
    /// `UIMenu` at the end of the root bar, replaced in place on rebuild.
    pub fn build_handler(declared: Declared) -> impl Fn(&MenuBuilder<'_>) + 'static {
        move |builder| {
            let Some((resolved, env)) = declared.borrow().clone() else {
                return;
            };
            let mtm = MainThreadMarker::new().expect("build_menus runs on the main thread");
            for (index, item) in resolved.snapshot().iter().enumerate() {
                let ResolvedMenuItem::Menu(menu) = item else {
                    panic!("App::menu_bar only accepts top-level Menu values");
                };
                let identifier = alloc::format!("dev.waterui.menu.{index}");
                let command = kit_command_for_menu(menu);
                let nodes = menu_tree(&menu.items.snapshot(), &env);
                let ui_menu =
                    cocoa_ui::uikit::menu_with_identifier(mtm, &command, Some(&identifier), &nodes);
                if builder.contains(&identifier) {
                    builder.replace(&identifier, &ui_menu);
                } else {
                    builder.insert_at_root_end(&ui_menu);
                }
            }
        }
    }

    /// Fills `declared` and flags each change for rebuild. The returned
    /// guard keeps the watcher alive; hold it for the process.
    pub fn install_declared(
        menu_bar: &Computed<Vec<DeclaredMenu>>,
        env: &Environment,
        declared: &Declared,
    ) -> Box<dyn Any> {
        let resolved = resolve(menu_bar, env);
        let guard = resolved.watch(|_| {
            if let Some(mtm) = MainThreadMarker::new() {
                cocoa_ui::uikit::request_main_menu_rebuild(mtm);
            }
        });
        *declared.borrow_mut() = Some((resolved, env.clone()));
        Box::new(guard)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use waterui::component::menu::{CommandExt as _, Menu, MenuItem, ResolvedMenuItem};
    use waterui::reactive::{Computed, Signal};
    use waterui_backend_core::Environment;

    use super::{CloseWindowPlacement, resolve};

    /// `menus` resolved as the declared menu bar.
    fn declared(menus: Vec<Menu>) -> Vec<ResolvedMenuItem> {
        resolve(&Computed::constant(menus), &Environment::new()).snapshot()
    }

    #[test]
    fn the_window_menu_carries_close_when_nothing_declares_it() {
        assert_eq!(
            CloseWindowPlacement::for_declared(&[]),
            CloseWindowPlacement::WindowMenu
        );
        let items = declared(vec![Menu::new(
            "File",
            (
                "Open".action(|| {}),
                MenuItem::Divider,
                MenuItem::Quit,
                Menu::new("Recent", ("Clear".action(|| {}),)),
            ),
        )]);
        assert_eq!(
            CloseWindowPlacement::for_declared(&items),
            CloseWindowPlacement::WindowMenu
        );
    }

    #[test]
    fn a_declared_close_window_keeps_it_out_of_the_window_menu() {
        let top_level = declared(vec![Menu::new(
            "File",
            ("Open".action(|| {}), MenuItem::CloseWindow),
        )]);
        assert_eq!(
            CloseWindowPlacement::for_declared(&top_level),
            CloseWindowPlacement::Declared
        );
        let nested = declared(vec![
            Menu::new("File", ("Open".action(|| {}),)),
            Menu::new("View", (Menu::new("Windows", (MenuItem::CloseWindow,)),)),
        ]);
        assert_eq!(
            CloseWindowPlacement::for_declared(&nested),
            CloseWindowPlacement::Declared
        );
    }
}
