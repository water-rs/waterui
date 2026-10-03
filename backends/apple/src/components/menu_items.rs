//! Menu-item plumbing shared by `menu` and `context_menu`: turning
//! `ResolvedMenuItem`s into platform menu content, watching their live
//! signals, and running changes under the watcher's platform animation.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use cocoa_ui::menu::Command as KitCommand;
#[cfg(feature = "context_menu")]
use cocoa_ui::menu::MenuTreeNode;
#[cfg(feature = "menu")]
use waterui::animation::Animation;
use waterui::component::menu::{CommandRole, ResolvedCommand, ResolvedMenuItem, Shortcut};
use waterui::reactive::Signal;
#[cfg(feature = "menu")]
use waterui::reactive::watcher::{BoxWatcherGuard, Metadata};
use waterui::text::StyledStr;
use waterui_backend_core::Environment;

#[cfg(all(target_os = "macos", feature = "menu"))]
mod platform {
    pub(super) use cocoa_ui::appkit::{Menu, MenuItem};
}

#[cfg(all(target_os = "ios", feature = "menu"))]
mod platform {
    pub(super) use cocoa_ui::uikit::{Menu, MenuAction, MenuElement};
}

/// A styled string as display text: the plain characters with the bidi
/// control characters interpolation inserts for layout stripped.
pub(super) fn item_title(styled: &StyledStr) -> String {
    cocoa_ui::text::strip_bidi_controls(styled.to_plain().as_str())
}

/// `withPlatformAnimation`: the watcher metadata's `Animation` mapped to a
/// kit timing — `Default` parses to the 0.25s bezier the FFI spells it as.
#[cfg(feature = "menu")]
pub(super) fn with_platform_animation(metadata: &Metadata, body: impl FnOnce() + 'static) {
    let timing = match metadata.try_get::<Animation>() {
        None => return body(),
        Some(Animation::Default) => cocoa_ui::core_animation::Timing::Bezier {
            duration: 0.25,
            control_points: [0.42, 0.0, 0.58, 1.0],
        },
        Some(Animation::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        }) => cocoa_ui::core_animation::Timing::Bezier {
            duration: duration.as_secs_f64(),
            control_points: [x1, y1, x2, y2],
        },
        Some(Animation::Spring { stiffness, damping }) => {
            cocoa_ui::core_animation::Timing::Spring {
                stiffness: f64::from(stiffness),
                damping: f64::from(damping),
            }
        }
    };
    cocoa_ui::core_animation::animate_with(timing, body);
}

/// Installs one watcher per live item signal — the command and submenu
/// labels, `disabled` and `selected`, and each nested menu's `items` —
/// recursively, as `WuiMenuTree` did per node. Every signal fires `resync`
/// with the watcher's metadata.
#[cfg(feature = "menu")]
pub(super) fn collect_item_watchers(
    items: &[ResolvedMenuItem],
    resync: &Rc<dyn Fn(&Metadata)>,
    watchers: &mut Vec<BoxWatcherGuard>,
) {
    for item in items {
        match item {
            ResolvedMenuItem::Command(command) => {
                for signal in [&command.disabled, &command.selected] {
                    let resync = Rc::clone(resync);
                    watchers.push(Box::new(signal.watch(move |wctx| {
                        resync(wctx.metadata());
                    })));
                }
                let resync = Rc::clone(resync);
                watchers.push(Box::new(command.label.content.watch(move |wctx| {
                    resync(wctx.metadata());
                })));
            }
            ResolvedMenuItem::Menu(submenu) => {
                watchers.push(Box::new(submenu.label.content.watch({
                    let resync = Rc::clone(resync);
                    move |wctx| {
                        resync(wctx.metadata());
                    }
                })));
                let nested = submenu.items.clone();
                watchers.push(Box::new(nested.watch({
                    let resync = Rc::clone(resync);
                    move |wctx| {
                        resync(wctx.metadata());
                    }
                })));
                collect_item_watchers(&submenu.items.snapshot(), resync, watchers);
            }
            ResolvedMenuItem::Divider => {}
        }
    }
}

/// The key equivalent and modifier set a `Shortcut` describes; no shortcut
/// is the empty pair.
fn shortcut_parts(shortcut: &Shortcut) -> (String, cocoa_ui::menu::KeyModifiers) {
    use cocoa_ui::menu::KeyModifiers;
    let modifiers = [
        (shortcut.modifiers.command(), KeyModifiers::COMMAND),
        (shortcut.modifiers.option(), KeyModifiers::OPTION),
        (shortcut.modifiers.shift(), KeyModifiers::SHIFT),
        (shortcut.modifiers.control(), KeyModifiers::CONTROL),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .fold(KeyModifiers::empty(), |flags, (_, native)| flags | native);
    (String::from(shortcut.key.as_str()), modifiers)
}

/// A `ResolvedCommand` as a kit `Command` — every presentation field
/// snapshotted; the shortcut's key equivalent travels cross-platform.
fn kit_command(command: &ResolvedCommand) -> KitCommand {
    let (key_equivalent, modifiers) = command.shortcut.as_ref().map_or_else(
        || (String::new(), cocoa_ui::menu::KeyModifiers::empty()),
        shortcut_parts,
    );
    KitCommand {
        label: item_title(&command.label.content.snapshot()),
        subtitle: command.subtitle.as_ref().map(ToString::to_string),
        symbol: command
            .icon
            .as_ref()
            .map(|icon| String::from(icon.name.as_str())),
        destructive: matches!(command.role, CommandRole::Destructive),
        enabled: !command.disabled.snapshot(),
        selected: command.selected.snapshot(),
        key_equivalent,
        modifiers,
    }
}

/// A `ResolvedMenuItem` list as a kit menu tree — commands, separators,
/// nested menus — the input both platform menu builders take.
#[cfg(feature = "context_menu")]
pub(super) fn tree_nodes(items: &[ResolvedMenuItem], env: &Environment) -> Vec<MenuTreeNode> {
    items
        .iter()
        .map(|item| match item {
            ResolvedMenuItem::Divider => MenuTreeNode::Divider,
            ResolvedMenuItem::Command(command) => {
                let action = command.action.clone();
                let env = env.clone();
                MenuTreeNode::Command(
                    kit_command(command),
                    Rc::new(move || {
                        action.call(&env);
                    }),
                )
            }
            ResolvedMenuItem::Menu(submenu) => MenuTreeNode::Submenu(
                KitCommand {
                    label: item_title(&submenu.label.content.snapshot()),
                    symbol: submenu
                        .icon
                        .as_ref()
                        .map(|icon| String::from(icon.name.as_str())),
                    ..KitCommand::default()
                },
                tree_nodes(&submenu.items.snapshot(), env),
            ),
        })
        .collect()
}

/// `wuiApplyCommandPresentation`: title, key equivalent, enabled, checked
/// state, subtitle, destructive red, icon — then the action a pick runs.
#[cfg(all(target_os = "macos", feature = "menu"))]
fn command_item(
    mtm: cocoa_ui::MainThreadMarker,
    command: &ResolvedCommand,
    env: &Environment,
) -> platform::MenuItem {
    let kit = kit_command(command);
    let mut item = platform::MenuItem::new(mtm, &kit.label, None, &kit.key_equivalent)
        .with_key_modifiers(kit.modifiers)
        .with_enabled(kit.enabled)
        .with_selected(kit.selected);
    if let Some(subtitle) = &kit.subtitle {
        item = item.with_subtitle(subtitle.as_str());
    }
    // `attributedTitle` replaces the plain title, so it lands after the
    // subtitle is committed.
    if kit.destructive {
        item = item.with_destructive();
    }
    if let Some(symbol) = &kit.symbol {
        item = item.with_icon(symbol.as_str());
    }
    let action = command.action.clone();
    let env = env.clone();
    item.with_action(move || {
        action.call(&env);
    })
}

/// `appendAppKitMenuItems`: each item appended in order — commands, a
/// separator per divider, nested menus under a titled item.
#[cfg(all(target_os = "macos", feature = "menu"))]
pub(super) fn append_items(
    mtm: cocoa_ui::MainThreadMarker,
    menu: &platform::Menu,
    items: &[ResolvedMenuItem],
    env: &Environment,
) {
    for item in items {
        match item {
            ResolvedMenuItem::Divider => menu.add_separator(),
            ResolvedMenuItem::Command(command) => {
                menu.add_item(command_item(mtm, command, env));
            }
            ResolvedMenuItem::Menu(submenu) => {
                let title = item_title(&submenu.label.content.snapshot());
                let nested = platform::Menu::new(mtm, &title);
                append_items(mtm, &nested, &submenu.items.snapshot(), env);
                let mut item = platform::MenuItem::new(mtm, &title, None, "");
                if let Some(icon) = &submenu.icon {
                    item = item.with_icon(icon.name.as_str());
                }
                menu.add_item(item.with_submenu(&nested));
            }
        }
    }
}

/// Rebuilds the trigger's `UIMenu`: dividers split the items into
/// `.displayInline` groups, flattened when a single group remains —
/// `splitMenuGroups` + `buildUIKitMenu`.
#[cfg(all(target_os = "ios", feature = "menu"))]
pub(super) fn build_menu(
    mtm: cocoa_ui::MainThreadMarker,
    title: &str,
    icon: Option<&str>,
    items: &[ResolvedMenuItem],
    env: &Environment,
) -> platform::Menu {
    let mut groups: Vec<&[ResolvedMenuItem]> = items
        .split(|item| matches!(item, ResolvedMenuItem::Divider))
        .filter(|group| !group.is_empty())
        .collect();
    let flat = groups.len() <= 1;
    if groups.is_empty() {
        groups.push(&[]);
    }
    let children: Vec<platform::MenuElement> = if flat {
        menu_elements(groups[0], env, mtm)
    } else {
        groups
            .iter()
            .map(|group| {
                platform::MenuElement::Submenu(platform::Menu::new(
                    mtm,
                    "",
                    None,
                    true,
                    &menu_elements(group, env, mtm),
                ))
            })
            .collect()
    };
    platform::Menu::new(mtm, title, icon, false, &children)
}

/// `buildUIKitMenuElements`: one element per item — `UIAction`s for
/// commands carrying title, subtitle, icon, disabled/destructive
/// attributes, on-state and handler; nested menus recurse.
#[cfg(all(target_os = "ios", feature = "menu"))]
fn menu_elements(
    items: &[ResolvedMenuItem],
    env: &Environment,
    mtm: cocoa_ui::MainThreadMarker,
) -> Vec<platform::MenuElement> {
    items
        .iter()
        .filter_map(|item| match item {
            ResolvedMenuItem::Divider => None,
            ResolvedMenuItem::Command(command) => {
                let kit = kit_command(command);
                let action = command.action.clone();
                let env = env.clone();
                Some(platform::MenuElement::Action(
                    platform::MenuAction::new(mtm, &kit.label, move || {
                        action.call(&env);
                    })
                    .with_subtitle(kit.subtitle.as_deref())
                    .with_icon(kit.symbol.as_deref())
                    .with_disabled(!kit.enabled)
                    .with_destructive(kit.destructive)
                    .with_selected(kit.selected),
                ))
            }
            ResolvedMenuItem::Menu(submenu) => {
                let title = item_title(&submenu.label.content.snapshot());
                Some(platform::MenuElement::Submenu(build_menu(
                    mtm,
                    &title,
                    submenu.icon.as_ref().map(|icon| icon.name.as_str()),
                    &submenu.items.snapshot(),
                    env,
                )))
            }
        })
        .collect()
}
