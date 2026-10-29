//! The application's menu bar.
//!
//! `WaterUIMainMenu.create()` + `WuiRootContext`'s
//! `installMenuBar`/`menuBarDidChange` ported: the standard App, Edit (the
//! responder-chain items keyboard shortcuts route through) and Window menus,
//! with the declared `menu_bar` content appended — macOS rebuilds the whole
//! bar on every change. iOS builds it through `waterui_apple_menu::ios`.
//!
//! The standard Window menu carries Close unless a declared menu places
//! `MenuItem::CloseWindow` itself — [`CloseWindowPlacement`] decides which,
//! from the declared items alone — and its chord is the application's one
//! decision, [`CloseWindowChord`], which `App::into_parts` installs: ⌘W
//! unless the application binds it, ⇧⌘W then, none when it binds both.

#[cfg(target_os = "macos")]
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
    use waterui::component::menu::{
        CloseWindowChord, CloseWindowPlacement, Menu as DeclaredMenu, ResolvedMenuItem, Shortcut,
        resolve_menu_bar_items,
    };
    use waterui::reactive::{Computed, Signal};
    use waterui_apple_menu::menu_tree;
    use waterui_backend_core::Environment;

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
    /// action the same untargeted `terminate:` the application menu's item
    /// sends.
    #[cfg(any(feature = "context_menu", feature = "text_field"))]
    pub fn standard_quit_node() -> cocoa_ui::menu::MenuTreeNode {
        cocoa_ui::menu::MenuTreeNode::Standard(
            cocoa_ui::menu::Command {
                label: quit_title(),
                key_equivalent: String::from("q"),
                modifiers: KeyModifiers::COMMAND,
                ..cocoa_ui::menu::Command::default()
            },
            MenuAction::Quit,
        )
    }

    /// The standard Close Window item — `performClose:` sent untargeted up
    /// the responder chain, so the key window closes as its close button
    /// would close it, and `AppKit` disables the item while no window can
    /// close. Its key equivalent is the application's decided chord.
    /// The Window menu carries it unless the application declares
    /// `MenuItem::CloseWindow`; a declared one in a menu a window mounts
    /// renders as this same item.
    pub fn standard_close_window_item(mtm: MainThreadMarker, chord: Option<&Shortcut>) -> MenuItem {
        MenuItem::standard(
            mtm,
            &waterui_apple_menu::standard_close_window_command(chord),
            MenuAction::CloseWindow,
        )
    }

    /// The standard menu bar's content: App, Edit, Window — the same menus
    /// `main.swift.tpl` installed before `app.run()`, with Close in the
    /// Window menu when `close_window` places it there, armed with
    /// `close_chord`. Declared menus append to it in [`install`].
    fn build_default(
        mtm: MainThreadMarker,
        application: &Application,
        close_window: CloseWindowPlacement,
        close_chord: Option<&Shortcut>,
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
            window_menu.add_item(standard_close_window_item(mtm, close_chord));
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
    /// the Window menu does, and `close_chord` is the chord it carries.
    pub fn install(
        mtm: MainThreadMarker,
        application: &Application,
        nodes: &[cocoa_ui::menu::MenuTreeNode],
        close_window: CloseWindowPlacement,
        close_chord: Option<&Shortcut>,
    ) {
        let main = build_default(mtm, application, close_window, close_chord);
        main.append_nodes(nodes);
        application.set_main_menu(&main);
    }

    /// The menu bar before `app(env)` reports its declared content: nothing
    /// is declared, so the Window menu carries Close with its free chord.
    pub fn install_default(mtm: MainThreadMarker, application: &Application) {
        install(
            mtm,
            application,
            &[],
            CloseWindowPlacement::WindowMenu,
            ResolvedMenuItem::close_window_chord(&[]).as_ref(),
        );
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
        let resolved = resolve_menu_bar_items(menu_bar, env);
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
                    CloseWindowChord::of(&env).as_ref(),
                );
            })
        };
        rebuild();
        let guard = resolved.watch(move |_| rebuild());
        Box::new(guard)
    }
}
