//! The application's menu bar.
//!
//! `WaterUIMainMenu.create()` + `WuiRootContext`'s
//! `installMenuBar`/`menuBarDidChange` ported: the standard App, Edit (the
//! responder-chain items keyboard shortcuts route through) and Window menus,
//! with the declared `menu_bar` content appended — macOS rebuilds the whole
//! bar on every change, iOS rebuilds through the main menu system's build
//! handler.
//!
//! The standard Window menu carries Close unless a declared menu places
//! `MenuItem::CloseWindow` itself — [`CloseWindowPlacement`] decides which,
//! from the declared items alone — and its chord is the application's one
//! decision, [`CloseWindowChord`], which `App::into_parts` installs: ⌘W
//! unless the application binds it, ⇧⌘W then, none when it binds both.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use cocoa_ui::Retained;
use waterui::component::IconSlot;
#[cfg(target_os = "macos")]
use waterui::component::menu::{CloseWindowChord, CloseWindowPlacement};
use waterui::component::menu::{
    CommandRole, Menu, NamedKey, ResolvedCommand, ResolvedMenuItem, ResolvedNestedMenu,
    ShortcutKey, resolve_menu_bar_items,
};
use waterui::reactive::{Computed, Signal};
use waterui_backend_core::Environment;
use waterui_core::layout::ProposalSize;

use crate::contract::{Held, RenderContext};
use crate::sf_symbol::SfSymbolProbe;

#[cfg(target_os = "macos")]
pub type PlatformMenuImage = cocoa_ui::objc2_app_kit::NSImage;
#[cfg(target_os = "ios")]
pub type PlatformMenuImage = cocoa_ui::objc2_ui_kit::UIImage;

type DeclaredMenu = Menu<waterui::component::label::SemanticLabel>;

/// Resolves a label's icon slot for native Apple menus: direct [`crate::SfSymbol`]
/// travels as a system symbol name via [`IconSlot::probe`], and any other view is
/// realized statically and rasterized into a template platform image.
pub fn resolve_menu_icon(
    icon: Option<&IconSlot<Held>>,
    env: &Environment,
) -> (Option<String>, Option<Retained<PlatformMenuImage>>) {
    let Some(slot) = icon else {
        return (None, None);
    };
    let mut probe = SfSymbolProbe::default();
    if slot.probe(&mut probe, env)
        && let Some(symbol) = probe.name
    {
        return (Some(String::from(symbol)), None);
    }
    let Some(mtm) = cocoa_ui::MainThreadMarker::new() else {
        tracing::warn!("menu icon view could not be rasterized off the main thread");
        return (None, None);
    };
    let leaf = RenderContext::entry(env, mtm).realize(slot.build());
    let size = leaf
        .layout()
        .measure(ProposalSize {
            width: None,
            height: None,
        })
        .size;
    cocoa_ui::view::set_frame(
        leaf.view(),
        cocoa_ui::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
    );
    (
        None,
        cocoa_ui::bitmap::view_template_image(leaf.view(), 16.0),
    )
}

/// Resolves `menu_bar` against `env` — items live-resolve under the
/// environment's locale.
#[must_use]
fn resolve(
    menu_bar: &Computed<Vec<DeclaredMenu>>,
    env: &Environment,
) -> Computed<Vec<ResolvedMenuItem<Held>>> {
    resolve_menu_bar_items::<_, Held>(menu_bar, env)
}

/// A resolved command as the kit's shared `Command` payload — label text,
/// subtitle, icon, destructive flag and shortcut.
fn kit_command(command: &ResolvedCommand<Held>, env: &Environment) -> cocoa_ui::menu::Command {
    let (symbol, image) = resolve_menu_icon(command.icon.as_ref(), env);
    kit_command_fields(
        command.label.content().snapshot().to_plain().to_string(),
        command.subtitle.as_ref().map(ToString::to_string),
        symbol,
        image,
        command.role,
        command.shortcut.as_ref(),
    )
}

/// A resolved nested menu's header as the kit's shared `Command` payload.
pub fn kit_command_for_menu(
    menu: &ResolvedNestedMenu<Held>,
    env: &Environment,
) -> cocoa_ui::menu::Command {
    let (symbol, image) = resolve_menu_icon(menu.icon.as_ref(), env);
    kit_command_fields(
        menu.label.content().snapshot().to_plain().to_string(),
        None,
        symbol,
        image,
        CommandRole::Standard,
        None,
    )
}

fn kit_command_fields(
    label: String,
    subtitle: Option<String>,
    symbol: Option<String>,
    image: Option<Retained<PlatformMenuImage>>,
    role: CommandRole,
    shortcut: Option<&waterui::component::menu::Shortcut>,
) -> cocoa_ui::menu::Command {
    let mut modifiers = cocoa_ui::menu::KeyModifiers::empty();
    let mut key_equivalent = String::new();
    if let Some(shortcut) = shortcut {
        key_equivalent = key_equivalent_for(shortcut.key());
        if shortcut.modifiers().command() {
            modifiers |= cocoa_ui::menu::KeyModifiers::COMMAND;
        }
        if shortcut.modifiers().shift() {
            modifiers |= cocoa_ui::menu::KeyModifiers::SHIFT;
        }
        if shortcut.modifiers().option() {
            modifiers |= cocoa_ui::menu::KeyModifiers::OPTION;
        }
        if shortcut.modifiers().control() {
            modifiers |= cocoa_ui::menu::KeyModifiers::CONTROL;
        }
    }
    cocoa_ui::menu::Command {
        label,
        subtitle,
        symbol,
        image,
        destructive: matches!(role, CommandRole::Destructive),
        enabled: true,
        selected: false,
        key_equivalent,
        modifiers,
    }
}

/// The `NSMenuItem` key equivalent a shortcut key arms: the character itself,
/// or for a named key the character `AppKit` reports that key as — the
/// function-key code points (`NSDeleteFunctionKey` for forward delete ⌦,
/// `NSF5FunctionKey`, …) and the characters of Tab, Return and Escape. The
/// ⌫ key, W3C `Backspace`, sends `NSDeleteCharacter` (U+007F), not
/// `NSBackspaceCharacter`.
///
/// A named key `AppKit` has no key equivalent for is a contract violation:
/// a debug build panics, a release build reports it and answers the empty
/// key equivalent, which arms no shortcut.
#[cfg(target_os = "macos")]
pub fn key_equivalent_for(key: &ShortcutKey) -> String {
    use cocoa_ui::objc2_app_kit as appkit;
    let named = match key {
        // An uppercase key equivalent implies Shift; Shift comes only from
        // the shortcut's modifiers.
        ShortcutKey::Character(character) => return character.to_lowercase().collect(),
        ShortcutKey::Named(named) => *named,
    };
    let code = match named {
        NamedKey::Backspace => appkit::NSDeleteCharacter,
        NamedKey::Tab => appkit::NSTabCharacter,
        NamedKey::Enter => appkit::NSCarriageReturnCharacter,
        // AppKit names no constant for Escape; its key equivalent is ESC.
        NamedKey::Escape => 0x1B,
        NamedKey::Delete => appkit::NSDeleteFunctionKey,
        NamedKey::Insert => appkit::NSInsertFunctionKey,
        NamedKey::Home => appkit::NSHomeFunctionKey,
        NamedKey::End => appkit::NSEndFunctionKey,
        NamedKey::PageUp => appkit::NSPageUpFunctionKey,
        NamedKey::PageDown => appkit::NSPageDownFunctionKey,
        NamedKey::ArrowUp => appkit::NSUpArrowFunctionKey,
        NamedKey::ArrowDown => appkit::NSDownArrowFunctionKey,
        NamedKey::ArrowLeft => appkit::NSLeftArrowFunctionKey,
        NamedKey::ArrowRight => appkit::NSRightArrowFunctionKey,
        NamedKey::PrintScreen => appkit::NSPrintScreenFunctionKey,
        NamedKey::Pause => appkit::NSPauseFunctionKey,
        NamedKey::ContextMenu => appkit::NSMenuFunctionKey,
        NamedKey::Help => appkit::NSHelpFunctionKey,
        NamedKey::Clear => appkit::NSClearLineFunctionKey,
        NamedKey::Find => appkit::NSFindFunctionKey,
        NamedKey::Undo => appkit::NSUndoFunctionKey,
        NamedKey::Redo => appkit::NSRedoFunctionKey,
        NamedKey::Select => appkit::NSSelectFunctionKey,
        NamedKey::Execute => appkit::NSExecuteFunctionKey,
        NamedKey::Print => appkit::NSPrintFunctionKey,
        NamedKey::F1 => appkit::NSF1FunctionKey,
        NamedKey::F2 => appkit::NSF2FunctionKey,
        NamedKey::F3 => appkit::NSF3FunctionKey,
        NamedKey::F4 => appkit::NSF4FunctionKey,
        NamedKey::F5 => appkit::NSF5FunctionKey,
        NamedKey::F6 => appkit::NSF6FunctionKey,
        NamedKey::F7 => appkit::NSF7FunctionKey,
        NamedKey::F8 => appkit::NSF8FunctionKey,
        NamedKey::F9 => appkit::NSF9FunctionKey,
        NamedKey::F10 => appkit::NSF10FunctionKey,
        NamedKey::F11 => appkit::NSF11FunctionKey,
        NamedKey::F12 => appkit::NSF12FunctionKey,
        NamedKey::F13 => appkit::NSF13FunctionKey,
        NamedKey::F14 => appkit::NSF14FunctionKey,
        NamedKey::F15 => appkit::NSF15FunctionKey,
        NamedKey::F16 => appkit::NSF16FunctionKey,
        NamedKey::F17 => appkit::NSF17FunctionKey,
        NamedKey::F18 => appkit::NSF18FunctionKey,
        NamedKey::F19 => appkit::NSF19FunctionKey,
        NamedKey::F20 => appkit::NSF20FunctionKey,
        NamedKey::F21 => appkit::NSF21FunctionKey,
        NamedKey::F22 => appkit::NSF22FunctionKey,
        NamedKey::F23 => appkit::NSF23FunctionKey,
        NamedKey::F24 => appkit::NSF24FunctionKey,
        NamedKey::F25 => appkit::NSF25FunctionKey,
        NamedKey::F26 => appkit::NSF26FunctionKey,
        NamedKey::F27 => appkit::NSF27FunctionKey,
        NamedKey::F28 => appkit::NSF28FunctionKey,
        NamedKey::F29 => appkit::NSF29FunctionKey,
        NamedKey::F30 => appkit::NSF30FunctionKey,
        NamedKey::F31 => appkit::NSF31FunctionKey,
        NamedKey::F32 => appkit::NSF32FunctionKey,
        NamedKey::F33 => appkit::NSF33FunctionKey,
        NamedKey::F34 => appkit::NSF34FunctionKey,
        NamedKey::F35 => appkit::NSF35FunctionKey,
        _ => {
            waterui::contract_violation!(
                "the shortcut key `{named}` has no AppKit key equivalent, so a macOS menu item \
                 cannot arm it; choose a key AppKit menus support; the item has no shortcut"
            );
            return String::new();
        }
    };
    String::from(
        char::from_u32(code).expect("AppKit key-equivalent constants are Unicode scalar values"),
    )
}

/// The `UIKeyCommand` input a shortcut key arms: the character itself, the
/// `UIKeyInput*` constant for a named key `UIKit` names, and the control
/// characters of Backspace, Tab and Return, which `UIKit` takes as inputs.
///
/// A named key `UIKit` has no key-command input for is a contract
/// violation: a debug build panics, a release build reports it and answers
/// the empty input, which arms no shortcut.
#[cfg(not(target_os = "macos"))]
pub fn key_equivalent_for(key: &ShortcutKey) -> String {
    use cocoa_ui::objc2_foundation::NSString;
    use cocoa_ui::objc2_ui_kit as uikit;

    // objc2-ui-kit 0.3 binds `UIKeyInputF2`…`F12` but not `UIKeyInputF1`,
    // which `UIResponder.h` declares beside them (iOS 13.4).
    unsafe extern "C" {
        static UIKeyInputF1: &'static NSString;
    }

    let named = match key {
        // An uppercase key equivalent implies Shift; Shift comes only from
        // the shortcut's modifiers.
        ShortcutKey::Character(character) => return character.to_lowercase().collect(),
        ShortcutKey::Named(named) => *named,
    };
    // SAFETY: each `UIKeyInput*` is an immutable `NSString` constant UIKit
    // exports for the process's lifetime.
    let input: &NSString = unsafe {
        match named {
            NamedKey::Backspace => return String::from('\u{8}'),
            NamedKey::Tab => return String::from('\t'),
            NamedKey::Enter => return String::from('\r'),
            NamedKey::Escape => uikit::UIKeyInputEscape,
            NamedKey::Delete => uikit::UIKeyInputDelete,
            NamedKey::Home => uikit::UIKeyInputHome,
            NamedKey::End => uikit::UIKeyInputEnd,
            NamedKey::PageUp => uikit::UIKeyInputPageUp,
            NamedKey::PageDown => uikit::UIKeyInputPageDown,
            NamedKey::ArrowUp => uikit::UIKeyInputUpArrow,
            NamedKey::ArrowDown => uikit::UIKeyInputDownArrow,
            NamedKey::ArrowLeft => uikit::UIKeyInputLeftArrow,
            NamedKey::ArrowRight => uikit::UIKeyInputRightArrow,
            NamedKey::F1 => UIKeyInputF1,
            NamedKey::F2 => uikit::UIKeyInputF2,
            NamedKey::F3 => uikit::UIKeyInputF3,
            NamedKey::F4 => uikit::UIKeyInputF4,
            NamedKey::F5 => uikit::UIKeyInputF5,
            NamedKey::F6 => uikit::UIKeyInputF6,
            NamedKey::F7 => uikit::UIKeyInputF7,
            NamedKey::F8 => uikit::UIKeyInputF8,
            NamedKey::F9 => uikit::UIKeyInputF9,
            NamedKey::F10 => uikit::UIKeyInputF10,
            NamedKey::F11 => uikit::UIKeyInputF11,
            NamedKey::F12 => uikit::UIKeyInputF12,
            _ => {
                waterui::contract_violation!(
                    "the shortcut key `{named}` has no UIKeyCommand input, so a UIKit menu \
                     command cannot arm it; choose a key UIKit key commands support; the \
                     command has no shortcut"
                );
                return String::new();
            }
        }
    };
    input.to_string()
}

/// The declared menu bar's resolved items as the kit's shared
/// `MenuTreeNode` list, with each command's action bound to fire under
/// `env` — the content both the macOS and the iOS menu bar build from.
/// On macOS each command is checked against the chords the standard
/// application menu reserves.
pub fn menu_tree(
    items: &[ResolvedMenuItem<Held>],
    env: &Environment,
) -> Vec<cocoa_ui::menu::MenuTreeNode> {
    items
        .iter()
        .filter_map(|item| match item {
            ResolvedMenuItem::Divider => Some(cocoa_ui::menu::MenuTreeNode::Divider),
            ResolvedMenuItem::Command(command) => {
                #[cfg(target_os = "macos")]
                if !command.allowed_in_macos_menu_bar() {
                    return None;
                }
                let action = command.action.clone();
                let env = env.clone();
                Some(cocoa_ui::menu::MenuTreeNode::Command(
                    kit_command(command, &env),
                    Rc::new(move || {
                        action.run(&env);
                    }),
                ))
            }
            ResolvedMenuItem::Menu(menu) => Some(cocoa_ui::menu::MenuTreeNode::Submenu(
                kit_command_for_menu(menu, env),
                menu_tree(&menu.items.snapshot(), env),
            )),
            // macOS: the standard application menu already ends with Quit
            // (`build_default`), so a declared one is dropped rather than
            // shown twice. iOS has no application quit, so it is omitted.
            ResolvedMenuItem::Quit => None,
            // macOS: the standard Close item where it was declared, with the
            // application's decided chord — the Window menu then leaves its
            // own out (`CloseWindowPlacement`).
            #[cfg(target_os = "macos")]
            ResolvedMenuItem::CloseWindow => Some(standard_close_window_node(
                CloseWindowChord::of(env).as_ref(),
            )),
            // iOS: the system owns every scene's window, so a declared Close
            // Window is omitted.
            #[cfg(not(target_os = "macos"))]
            ResolvedMenuItem::CloseWindow => None,
        })
        .collect()
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
    use waterui::component::menu::{CloseWindowChord, CommandRole, ResolvedMenuItem, Shortcut};
    use waterui::reactive::{Computed, Signal};
    use waterui_backend_core::Environment;

    use crate::contract::Held;

    use super::{CloseWindowPlacement, DeclaredMenu, kit_command_fields, menu_tree, resolve};

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

    /// The standard Close Window item's title — `AppKit`'s own word for
    /// it, untranslated like the other standard items' titles here.
    const CLOSE_WINDOW_TITLE: &str = "Close";

    /// The standard Close Window item as a kit [`Command`] payload: the
    /// title and the application's decided chord — ⌘W while the
    /// application leaves it free, ⇧⌘W when a declared command holds ⌘W,
    /// no key equivalent when it binds both.
    fn standard_close_window_command(chord: Option<&Shortcut>) -> cocoa_ui::menu::Command {
        kit_command_fields(
            String::from(CLOSE_WINDOW_TITLE),
            None,
            None,
            None,
            CommandRole::Standard,
            chord,
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
            &standard_close_window_command(chord),
            MenuAction::CloseWindow,
        )
    }

    /// The standard Close Window item as a kit menu-tree node, for the
    /// menus built from `MenuTreeNode`s (the declared menu bar, context
    /// menus): the same title and chord, its action the same untargeted
    /// `performClose:` sent up the responder chain — `AppKit` greys the
    /// row out while no window can close.
    pub fn standard_close_window_node(chord: Option<&Shortcut>) -> cocoa_ui::menu::MenuTreeNode {
        cocoa_ui::menu::MenuTreeNode::Standard(
            standard_close_window_command(chord),
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
            ResolvedMenuItem::<Held>::close_window_chord(&[]).as_ref(),
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
                    CloseWindowChord::of(&env).as_ref(),
                );
            })
        };
        rebuild();
        let guard = resolved.watch(move |_| rebuild());
        Box::new(guard)
    }
}

/// iOS: the declared content fills the builder the main menu system hands
/// its build handler; a watch on the resolved items requests each rebuild.
#[cfg(target_os = "ios")]
mod imp {
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    use core::any::Any;

    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::uikit::MenuBuilder;
    use waterui::component::menu::ResolvedMenuItem;
    use waterui::reactive::{Computed, Signal};
    use waterui_backend_core::Environment;

    use crate::contract::Held;

    use super::{DeclaredMenu, kit_command_for_menu, menu_tree, resolve};

    fn build_menus(
        resolved: &Computed<Vec<ResolvedMenuItem<Held>>>,
        env: &Environment,
        builder: &MenuBuilder<'_>,
    ) {
        let mtm = MainThreadMarker::new().expect("UIKit calls `build_menus` on the main thread");
        let items = resolved.snapshot();
        for (index, item) in items.iter().enumerate() {
            let ResolvedMenuItem::Menu(menu) = item else {
                waterui::contract_violation!(
                    "App::menu_bar only accepts top-level Menu values; the item is skipped"
                );
                continue;
            };
            let identifier = alloc::format!("dev.waterui.menu.{index}");
            let command = kit_command_for_menu(menu, env);
            let nodes = menu_tree(&menu.items.snapshot(), env);
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

    /// Installs `menu_bar` as the main menu system's build handler and rebuilds
    /// when items change. The returned guard keeps the watcher alive; hold it
    /// for the process.
    pub fn install_declared(
        mtm: MainThreadMarker,
        menu_bar: &Computed<Vec<DeclaredMenu>>,
        env: &Environment,
    ) -> Box<dyn Any> {
        let resolved = resolve(menu_bar, env);
        let guard = resolved.watch(move |_| cocoa_ui::uikit::request_main_menu_rebuild(mtm));
        let handler_resolved = resolved.clone();
        let handler_env = env.clone();
        cocoa_ui::uikit::set_main_menu_builder(mtm, move |builder| {
            build_menus(&handler_resolved, &handler_env, builder);
        });
        Box::new(guard)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use cocoa_ui::appkit::{KeyModifiers, MenuAction};
    use cocoa_ui::menu::MenuTreeNode;
    use cocoa_ui::objc2_app_kit::{NSDeleteFunctionKey, NSF5FunctionKey};
    use waterui::component::label::{LabelKind, label};
    use waterui::component::menu::{
        CloseWindowChord, Command, Menu, MenuItem, ResolvedMenuItem, resolve_menu_bar_items,
    };
    use waterui::reactive::{Computed, Signal};
    use waterui_backend_core::Environment;

    use crate::SfSymbol;
    use crate::contract::Held;

    use super::{NamedKey, ShortcutKey, key_equivalent_for, menu_tree};

    fn function_key(code: u32) -> String {
        String::from(char::from_u32(code).expect("function keys are scalar values"))
    }

    #[test]
    fn named_keys_map_to_appkit_function_key_equivalents() {
        assert_eq!(
            key_equivalent_for(&ShortcutKey::from(NamedKey::Delete)),
            function_key(NSDeleteFunctionKey)
        );
        assert_eq!(
            key_equivalent_for(&ShortcutKey::from(NamedKey::F5)),
            function_key(NSF5FunctionKey)
        );
        assert_eq!(key_equivalent_for(&ShortcutKey::from('q')), "q");
    }

    #[test]
    fn an_uppercase_character_key_equivalent_is_lowercased() {
        assert_eq!(key_equivalent_for(&ShortcutKey::from('Q')), "q");
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "has no AppKit key equivalent")
    )]
    fn a_named_key_without_an_appkit_equivalent_arms_no_shortcut() {
        assert_eq!(
            key_equivalent_for(&ShortcutKey::from(NamedKey::AudioVolumeUp)),
            ""
        );
    }

    /// `menus` resolved as the declared menu bar.
    fn declared<L: LabelKind + Clone>(menus: Vec<Menu<L>>) -> Vec<ResolvedMenuItem<Held>> {
        resolve_menu_bar_items(&Computed::constant(menus), &Environment::new()).snapshot()
    }

    /// A bare environment with the application's decided chord installed,
    /// as `App::into_parts` installs it — here over an empty bar, so the
    /// chord is ⌘W.
    fn chord_env() -> Environment {
        let mut env = Environment::new();
        env.insert(CloseWindowChord::new(&Computed::constant(Vec::new()), &env));
        env
    }

    /// A declared Close Window is the standard item, armed with the chord
    /// the menu bar leaves free — ⌘W here, since nothing binds it.
    #[test]
    fn a_declared_close_window_emits_the_standard_item() {
        let env = chord_env();
        let nodes = menu_tree(&[ResolvedMenuItem::CloseWindow], &env);
        let [MenuTreeNode::Standard(command, MenuAction::CloseWindow)] = nodes.as_slice() else {
            panic!("a declared Close Window emits the standard Close node: {nodes:?}");
        };
        assert_eq!(command.label, "Close");
        assert_eq!(command.key_equivalent, "w");
        assert_eq!(command.modifiers, KeyModifiers::COMMAND);
    }

    #[test]
    fn a_nested_declared_close_window_emits_the_standard_item() {
        let env = chord_env();
        let items = declared(vec![Menu::new("Window", (MenuItem::CloseWindow,))]);
        let nodes = menu_tree(&items, &env);
        let [MenuTreeNode::Submenu(_, children)] = nodes.as_slice() else {
            panic!("a declared menu nests: {nodes:?}");
        };
        assert!(
            matches!(
                children.as_slice(),
                [MenuTreeNode::Standard(_, MenuAction::CloseWindow)]
            ),
            "a nested Close Window emits the standard Close node: {children:?}"
        );
    }

    #[test]
    fn sf_symbol_command_projects_directly_without_rasterization() {
        let env = chord_env();
        let command = Command::builder(label("Add").icon(SfSymbol::PLUS))
            .action(|| {})
            .resolve::<Held>(&env);
        let nodes = menu_tree(&[ResolvedMenuItem::Command(command)], &env);
        let [MenuTreeNode::Command(cmd, _)] = nodes.as_slice() else {
            panic!("expected command node: {nodes:?}");
        };
        assert_eq!(cmd.symbol.as_deref(), Some("plus"));
        assert!(cmd.image.is_none());
    }
}
