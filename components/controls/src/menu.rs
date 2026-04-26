//! Semantic menu and command APIs shared by popup menus and future system chrome.

use alloc::{vec, vec::Vec};

use nami::{Computed, SignalExt, impl_constant, signal::IntoComputed};
use waterui_core::Str;
use waterui_core::{
    AnyView, Environment, View,
    extract::State,
    handler::{Handler, SharedAction, shared_action},
    layout::StretchAxis,
    raw_view,
};
use waterui_icon::SystemIcon;
use waterui_layout::Divider;
use waterui_locale::locale_binding;
use waterui_text::{TextConfig, styled::StyledStr};

use crate::{
    button::Button,
    label::{IntoLabel, Label},
};

/// Keyboard shortcut modifiers used by commands and system menus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShortcutModifiers {
    flags: u8,
}

impl ShortcutModifiers {
    const COMMAND: u8 = 1 << 0;
    const SHIFT: u8 = 1 << 1;
    const OPTION: u8 = 1 << 2;
    const CONTROL: u8 = 1 << 3;

    /// Returns whether the command modifier is active.
    #[must_use]
    pub const fn command(self) -> bool {
        self.flags & Self::COMMAND != 0
    }

    /// Returns whether the shift modifier is active.
    #[must_use]
    pub const fn shift(self) -> bool {
        self.flags & Self::SHIFT != 0
    }

    /// Returns whether the option/alt modifier is active.
    #[must_use]
    pub const fn option(self) -> bool {
        self.flags & Self::OPTION != 0
    }

    /// Returns whether the control modifier is active.
    #[must_use]
    pub const fn control(self) -> bool {
        self.flags & Self::CONTROL != 0
    }

    const fn inserting(mut self, flag: u8) -> Self {
        self.flags |= flag;
        self
    }
}

/// Keyboard shortcut metadata attached to a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// The key equivalent used to trigger the command.
    pub key: Str,
    /// Modifier flags required alongside the key.
    pub modifiers: ShortcutModifiers,
}

impl Shortcut {
    /// Creates a shortcut from a key equivalent.
    #[must_use]
    pub fn new(key: impl Into<Str>) -> Self {
        Self {
            key: key.into(),
            modifiers: ShortcutModifiers::default(),
        }
    }

    /// Adds the command modifier.
    #[must_use]
    pub const fn command(mut self) -> Self {
        self.modifiers = self.modifiers.inserting(ShortcutModifiers::COMMAND);
        self
    }

    /// Adds the shift modifier.
    #[must_use]
    pub const fn shift(mut self) -> Self {
        self.modifiers = self.modifiers.inserting(ShortcutModifiers::SHIFT);
        self
    }

    /// Adds the option/alt modifier.
    #[must_use]
    pub const fn option(mut self) -> Self {
        self.modifiers = self.modifiers.inserting(ShortcutModifiers::OPTION);
        self
    }

    /// Adds the control modifier.
    #[must_use]
    pub const fn control(mut self) -> Self {
        self.modifiers = self.modifiers.inserting(ShortcutModifiers::CONTROL);
        self
    }
}

impl_constant!(Shortcut, ShortcutModifiers);

/// A semantic menu that can be rendered as a popup button or nested inside other menus.
#[derive(Debug, Clone)]
pub struct Menu {
    label: Label,
    /// The menu items to display when the menu is opened.
    pub items: Computed<Vec<MenuItem>>,
}

impl Menu {
    /// Creates a menu with a semantic label.
    #[must_use]
    pub fn new(label: impl IntoLabel, items: impl MenuView) -> Self {
        Self {
            label: label.into_label(),
            items: items.into_menu_items(),
        }
    }

    #[must_use]
    fn resolve(self, env: &Environment) -> ResolvedNestedMenu {
        let label = self.label;
        let resolved_label = label.semantic_text().resolve(env);
        let icon = label.semantic_icon();
        ResolvedNestedMenu {
            label: resolved_label,
            semantic_label: label,
            icon,
            items: resolve_menu_items(&self.items, env),
        }
    }
}

impl View for Menu {
    fn body(self, env: &Environment) -> impl View {
        let label = self.label;
        let accessibility_label = label.semantic_text().resolve(env).content;
        AnyView::new(ResolvedMenu {
            label: AnyView::new(label),
            items: resolve_menu_items(&self.items, env),
            accessibility_label,
        })
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
}

/// A semantic command that can be reused across menu-like surfaces.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Command {
    /// The semantic label for the command.
    pub label: Label,
    /// The action to perform when the command is selected.
    pub action: SharedAction<()>,
    /// Whether the command is currently disabled.
    pub disabled: Computed<bool>,
    /// Whether the command is currently selected or checked.
    pub selected: Computed<bool>,
    /// Optional keyboard shortcut metadata.
    pub shortcut: Option<Shortcut>,
    /// Local state layered onto the environment when the action runs.
    pub captured_env: Environment,
}

impl_constant!(Command);

impl Command {
    /// Creates a command builder with the given label.
    #[must_use]
    pub fn builder(label: impl IntoLabel) -> CommandBuilder {
        CommandBuilder {
            label: label.into_label(),
        }
    }

    #[must_use]
    fn resolve(self, env: &Environment) -> ResolvedCommand {
        let label = self.label;
        let resolved_label = label.semantic_text().resolve(env);
        let icon = label.semantic_icon();
        let action = self.action;
        let captured_env = self.captured_env;
        ResolvedCommand {
            label: resolved_label,
            semantic_label: label,
            icon,
            action: shared_action(move |env: Environment| {
                action.call(&captured_env.layered_on(&env));
            }),
            disabled: self.disabled,
            selected: self.selected,
            shortcut: self.shortcut,
        }
    }

    /// Marks the command as disabled according to the provided signal.
    #[must_use]
    pub fn disabled(mut self, disabled: impl IntoComputed<bool>) -> Self {
        self.disabled = disabled.into_computed();
        self
    }

    /// Marks the command as selected according to the provided signal.
    #[must_use]
    pub fn selected(mut self, selected: impl IntoComputed<bool>) -> Self {
        self.selected = selected.into_computed();
        self
    }

    /// Attaches keyboard shortcut metadata to the command.
    #[must_use]
    pub fn shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Injects cloneable state into this command's action environment.
    #[must_use]
    pub fn state<T: Clone + 'static>(mut self, state: &T) -> Self {
        self.captured_env = self.captured_env.extending(State(state.clone()));
        self
    }
}

/// Ergonomic extension for constructing commands from label-like values.
pub trait CommandExt: IntoLabel + Sized {
    /// Starts building a command from this label.
    #[must_use]
    fn command(self) -> CommandBuilder {
        Command::builder(self)
    }

    /// Creates a command with an action and no captured state.
    #[must_use]
    fn action<Args>(self, action: impl Handler<Args, ()> + 'static) -> Command {
        self.command().action(action)
    }
}

impl<T: IntoLabel> CommandExt for T {}

/// Builder for creating commands.
#[derive(Debug, Clone)]
pub struct CommandBuilder {
    label: Label,
}

impl CommandBuilder {
    /// Sets the action for this command.
    #[must_use]
    pub fn action<Args>(self, action: impl Handler<Args, ()> + 'static) -> Command {
        Command {
            label: self.label,
            action: shared_action(action),
            disabled: Computed::constant(false),
            selected: Computed::constant(false),
            shortcut: None,
            captured_env: Environment::new(),
        }
    }
}

/// A single item inside menu content.
#[derive(Debug, Clone)]
pub enum MenuItem {
    /// A leaf command.
    Command(Command),
    /// A divider separating adjacent commands or menus.
    Divider,
    /// A nested menu.
    Menu(Menu),
}

impl_constant!(MenuItem);

impl MenuItem {
    #[must_use]
    fn resolve(self, env: &Environment) -> ResolvedMenuItem {
        match self {
            Self::Command(command) => ResolvedMenuItem::Command(command.resolve(env)),
            Self::Divider => ResolvedMenuItem::Divider,
            Self::Menu(menu) => ResolvedMenuItem::Menu(menu.resolve(env)),
        }
    }
}

impl From<Command> for MenuItem {
    fn from(value: Command) -> Self {
        Self::Command(value)
    }
}

impl From<Divider> for MenuItem {
    fn from(_: Divider) -> Self {
        Self::Divider
    }
}

impl From<Menu> for MenuItem {
    fn from(value: Menu) -> Self {
        Self::Menu(value)
    }
}

/// Converts semantic menu content into menu items.
///
/// This trait is implemented for the supported menu content building blocks:
/// [`Command`], [`Menu`], [`Divider`], ordinary [`Button`] values, and
/// containers such as tuples, arrays, vectors, and [`Option`].
pub trait MenuView {
    /// Converts this value into reactive menu items.
    fn into_menu_items(self) -> Computed<Vec<MenuItem>>;
}

/// Converts top-level system menu bar content into menus.
pub trait MenuBarView {
    /// Converts this value into reactive top-level menus.
    fn into_menus(self) -> Computed<Vec<Menu>>;
}

impl MenuView for () {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        Computed::constant(Vec::new())
    }
}

impl<T: MenuView> MenuView for Option<T> {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        self.map_or_else(|| Computed::constant(Vec::new()), MenuView::into_menu_items)
    }
}

impl MenuView for Computed<Vec<MenuItem>> {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        self
    }
}

impl MenuView for MenuItem {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        Computed::constant(vec![self])
    }
}

impl MenuView for Command {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        Computed::constant(vec![MenuItem::Command(self)])
    }
}

impl MenuView for Menu {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        Computed::constant(vec![MenuItem::Menu(self)])
    }
}

impl MenuView for Divider {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        Computed::constant(vec![MenuItem::Divider])
    }
}

impl<LabelView, Action> MenuView for Button<LabelView, Action>
where
    LabelView: View + 'static,
    Action: FnMut(&Environment) + 'static,
{
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        Computed::constant(vec![self.into()])
    }
}

impl MenuBarView for () {
    fn into_menus(self) -> Computed<Vec<Menu>> {
        Computed::constant(Vec::new())
    }
}

impl<T: MenuBarView> MenuBarView for Option<T> {
    fn into_menus(self) -> Computed<Vec<Menu>> {
        self.map_or_else(|| Computed::constant(Vec::new()), MenuBarView::into_menus)
    }
}

impl MenuBarView for Computed<Vec<Menu>> {
    fn into_menus(self) -> Computed<Vec<Menu>> {
        self
    }
}

impl MenuBarView for Menu {
    fn into_menus(self) -> Computed<Vec<Menu>> {
        Computed::constant(vec![self])
    }
}

impl MenuBarView for Vec<Menu> {
    fn into_menus(self) -> Computed<Vec<Menu>> {
        Computed::constant(self)
    }
}

impl<const N: usize> MenuBarView for [Menu; N] {
    fn into_menus(self) -> Computed<Vec<Menu>> {
        Computed::constant(self.into_iter().collect())
    }
}

fn extend_menus(mut left: Vec<Menu>, right: Vec<Menu>) -> Vec<Menu> {
    left.extend(right);
    left
}

impl<T: MenuView> MenuView for Vec<T> {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        self.into_iter()
            .fold(Computed::constant(Vec::new()), |items, item| {
                let right = item.into_menu_items();
                items
                    .zip(&right)
                    .map(|(left, right)| extend_menu_items(left, right))
                    .computed()
            })
    }
}

impl<T: MenuView, const N: usize> MenuView for [T; N] {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        self.into_iter()
            .fold(Computed::constant(Vec::new()), |items, item| {
                let right = item.into_menu_items();
                items
                    .zip(&right)
                    .map(|(left, right)| extend_menu_items(left, right))
                    .computed()
            })
    }
}

fn extend_menu_items(mut left: Vec<MenuItem>, right: Vec<MenuItem>) -> Vec<MenuItem> {
    left.extend(right);
    left
}

macro_rules! menu_tuples {
    ($macro:ident) => {
        $macro!(T0);
        $macro!(T0, T1);
        $macro!(T0, T1, T2);
        $macro!(T0, T1, T2, T3);
        $macro!(T0, T1, T2, T3, T4);
        $macro!(T0, T1, T2, T3, T4, T5);
        $macro!(T0, T1, T2, T3, T4, T5, T6);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13);
        $macro!(
            T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14
        );
    };
}

macro_rules! impl_tuple_menu_view {
    ($T0:ident) => {
        impl<$T0: MenuView> MenuView for ($T0,) {
            fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
                self.0.into_menu_items()
            }
        }
    };
    ($T0:ident, $($T:ident),+) => {
        #[allow(non_snake_case)]
        impl<$T0: MenuView, $($T: MenuView),+> MenuView for ($T0, $($T,)+) {
            fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
                let ($T0, $($T),+) = self;
                let items = $T0.into_menu_items();
                $(
                    let $T = $T.into_menu_items();
                    let items = items
                        .zip(&$T)
                        .map(|(left, right)| extend_menu_items(left, right))
                        .computed();
                )+
                items
            }
        }
    };
}

menu_tuples!(impl_tuple_menu_view);

macro_rules! impl_tuple_menu_bar_view {
    ($T0:ident) => {
        impl<$T0: MenuBarView> MenuBarView for ($T0,) {
            fn into_menus(self) -> Computed<Vec<Menu>> {
                self.0.into_menus()
            }
        }
    };
    ($T0:ident, $($T:ident),+) => {
        #[allow(non_snake_case)]
        impl<$T0: MenuBarView, $($T: MenuBarView),+> MenuBarView for ($T0, $($T,)+) {
            fn into_menus(self) -> Computed<Vec<Menu>> {
                let ($T0, $($T),+) = self;
                let items = $T0.into_menus();
                $(
                    let $T = $T.into_menus();
                    let items = items
                        .zip(&$T)
                        .map(|(left, right)| extend_menus(left, right))
                        .computed();
                )+
                items
            }
        }
    };
}

menu_tuples!(impl_tuple_menu_bar_view);

fn resolve_menu_items_now(items: Vec<MenuItem>, env: &Environment) -> Vec<ResolvedMenuItem> {
    items.into_iter().map(|item| item.resolve(env)).collect()
}

#[doc(hidden)]
#[must_use]
pub fn resolve_menu_items(
    items: &Computed<Vec<MenuItem>>,
    env: &Environment,
) -> Computed<Vec<ResolvedMenuItem>> {
    let locale = locale_binding(env);
    let env = env.clone();
    items
        .zip(&locale)
        .map(move |(items, _locale)| resolve_menu_items_now(items, &env))
        .computed()
}

#[doc(hidden)]
#[must_use]
pub fn resolve_menu_bar_items(
    menus: &Computed<Vec<Menu>>,
    env: &Environment,
) -> Computed<Vec<ResolvedMenuItem>> {
    let locale = locale_binding(env);
    let env = env.clone();
    menus
        .zip(&locale)
        .map(move |(menus, _locale)| {
            menus
                .into_iter()
                .map(|menu| ResolvedMenuItem::Menu(menu.resolve(&env)))
                .collect()
        })
        .computed()
}

/// Raw resolved command payload consumed by native backends.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ResolvedCommand {
    /// Resolved text config for native menu presentation.
    pub label: TextConfig,
    /// The full semantic label retained for self-drawn menu renderers.
    pub semantic_label: Label,
    /// Optional platform icon shown alongside the label.
    pub icon: Option<SystemIcon>,
    /// Shared action invoked by the native backend.
    pub action: SharedAction<()>,
    /// Whether the command is currently disabled.
    pub disabled: Computed<bool>,
    /// Whether the command is currently selected or checked.
    pub selected: Computed<bool>,
    /// Optional keyboard shortcut metadata.
    pub shortcut: Option<Shortcut>,
}

impl_constant!(ResolvedCommand);

/// Raw resolved nested menu payload consumed by native backends.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ResolvedNestedMenu {
    /// Resolved text config for the nested menu title.
    pub label: TextConfig,
    /// The full semantic label retained for self-drawn menu renderers.
    pub semantic_label: Label,
    /// Optional platform icon shown alongside the title.
    pub icon: Option<SystemIcon>,
    /// Resolved nested menu items.
    pub items: Computed<Vec<ResolvedMenuItem>>,
}

impl_constant!(ResolvedNestedMenu);

/// Raw menu item config resolved against the environment locale.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum ResolvedMenuItem {
    /// A resolved leaf command.
    Command(ResolvedCommand),
    /// A divider separating sibling items.
    Divider,
    /// A resolved nested menu.
    Menu(ResolvedNestedMenu),
}

impl_constant!(ResolvedMenuItem);

/// Raw native popup menu config resolved against the environment locale.
#[doc(hidden)]
#[derive(Debug)]
pub struct ResolvedMenu {
    /// The label view displayed on the menu button.
    pub label: AnyView,
    /// The fully resolved menu items.
    pub items: Computed<Vec<ResolvedMenuItem>>,
    /// Semantic accessibility label for the menu trigger.
    pub accessibility_label: Computed<StyledStr>,
}

raw_view!(ResolvedMenu, StretchAxis::None);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::button::button;
    use alloc::rc::Rc;
    use core::cell::Cell;
    use nami::Signal;
    use waterui_icon::SystemIcon;

    #[test]
    fn menu_view_accepts_plain_buttons() {
        let items = button(Label::new("Search").system_icon(SystemIcon::SEARCH))
            .action(|| {})
            .into_menu_items()
            .get();
        assert_eq!(items.len(), 1);
        let MenuItem::Command(command) = &items[0] else {
            panic!("button menu content should resolve to a command");
        };
        assert_eq!(
            command
                .label
                .semantic_text()
                .resolve(&Environment::default())
                .content
                .get()
                .to_plain()
                .as_str(),
            "Search"
        );
    }

    #[test]
    fn menu_view_flattens_vectors_of_supported_items() {
        let items: Vec<MenuItem> = vec![
            button("Edit").action(|| {}).into(),
            Divider.into(),
            Menu::new("More", (button("Duplicate").action(|| {}),)).into(),
        ];
        let items = items.into_menu_items().get();
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], MenuItem::Command(_)));
        assert!(matches!(items[1], MenuItem::Divider));
        assert!(matches!(items[2], MenuItem::Menu(_)));
    }

    #[test]
    fn resolve_menu_items_preserves_nested_labels_and_selected_state() {
        let env = Environment::default();
        let items = vec![
            Command::builder("Refresh").action(|| {}).into(),
            Command::builder("Pinned")
                .action(|| {})
                .selected(Computed::constant(true))
                .into(),
            Menu::new("Advanced", (button("Archive").action(|| {}),)).into(),
        ];

        let resolved = resolve_menu_items_now(items, &env);
        assert_eq!(resolved.len(), 3);

        let ResolvedMenuItem::Command(refresh) = &resolved[0] else {
            panic!("first resolved item should be a command");
        };
        assert_eq!(refresh.label.content.get().to_plain(), "Refresh");
        assert!(!refresh.selected.get());

        let ResolvedMenuItem::Command(pinned) = &resolved[1] else {
            panic!("second resolved item should be a command");
        };
        assert_eq!(pinned.label.content.get().to_plain(), "Pinned");
        assert!(pinned.selected.get());

        let ResolvedMenuItem::Menu(advanced) = &resolved[2] else {
            panic!("third resolved item should be a nested menu");
        };
        assert_eq!(advanced.label.content.get().to_plain(), "Advanced");
        assert_eq!(advanced.items.get().len(), 1);

        let ResolvedMenuItem::Command(archive) = &advanced.items.get()[0] else {
            panic!("nested menu should resolve its child command");
        };
        assert_eq!(archive.label.content.get().to_plain(), "Archive");
    }

    #[test]
    fn resolved_command_action_uses_captured_state() {
        let count = Rc::new(Cell::new(0));
        let command = Command::builder("Increment")
            .action(|State(count): State<Rc<Cell<i32>>>| count.set(count.get() + 1))
            .state(&count);
        let resolved = command.resolve(&Environment::default());

        resolved.action.call(&Environment::default());
        resolved.action.call(&Environment::default());

        assert_eq!(count.get(), 2);
    }
}
