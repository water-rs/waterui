//! Semantic menu and command APIs shared by popup menus and future system chrome.

use alloc::{rc::Rc, vec, vec::Vec};

use nami::{
    Computed, Signal, SignalExt, SignalIdentity, impl_constant,
    signal::IntoComputed,
    watcher::{BoxWatcherGuard, Context, WatcherGuard},
};
use waterui_core::Str;
use waterui_core::{
    AnyView, Environment, Native, NativeView, View,
    backend::{Backend, ChildSlot, MapSlots},
    contract_violation,
    extract::State,
    handler::{Handler, SharedAction, shared_action},
};
use waterui_interaction::interaction::Disabled;
pub use waterui_interaction::key::NamedKey;
use waterui_locale::locale_binding;
use waterui_text::{IntoText, Text, TextConfig, styled::StyledStr};

pub use crate::shortcut::{
    InvalidShortcutKey, Shortcut, ShortcutKey, ShortcutModifiers, ShortcutNamedKey,
};
use crate::{
    button::{Button, ButtonRole, ButtonStyle},
    label::{
        ControlLabel, IntoAccessibilityLabel, IntoLabel, Label, LabelDisplayMode, LabelKind,
        SemanticLabel,
    },
    size::ControlSize,
    slot::{IconSlot, NoopSlot},
};

/// A semantic menu that can be rendered as a popup button or nested inside other menus.
#[derive(Debug, Clone)]
pub struct Menu<L> {
    label: Label<L>,
    /// The menu items to display when the menu is opened.
    items: Computed<Vec<MenuItem>>,
    identity: Rc<()>,
    style: ButtonStyle,
    size: ControlSize,
}

impl<L> Menu<L> {
    /// Creates a menu with a semantic label. Its trigger is drawn as an
    /// [`Automatic`](ButtonStyle::Automatic) button of
    /// [`ControlSize::Small`], like a [`Button`].
    #[must_use]
    pub fn new<I: IntoLabel<Kind = L>>(label: I, items: impl MenuView) -> Self {
        Self {
            label: label.into_label(),
            items: items.into_menu_items(),
            identity: Rc::new(()),
            style: ButtonStyle::Automatic,
            size: ControlSize::Small,
        }
    }

    /// The menu items to display when the menu is opened.
    #[must_use]
    pub const fn items(&self) -> &Computed<Vec<MenuItem>> {
        &self.items
    }

    /// Sets the style the menu's trigger button is drawn in.
    #[must_use]
    pub const fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets how large the menu's trigger button is drawn.
    #[must_use]
    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Sets the visual presentation mode of the menu's trigger label.
    #[must_use]
    pub const fn label_style(mut self, mode: LabelDisplayMode) -> Self {
        self.label.set_display_mode(mode);
        self
    }

    /// Visually hides the menu's trigger label while keeping its semantic text
    /// for assistive technology.
    #[must_use]
    pub const fn hide_label(self) -> Self {
        self.label_style(LabelDisplayMode::Hidden)
    }

    /// Overrides the spoken accessibility text announced by assistive
    /// technology without changing the visual trigger label.
    #[must_use]
    pub fn a11y_label<M>(mut self, label: impl IntoAccessibilityLabel<M>) -> Self {
        self.label.set_accessibility_text(label);
        self
    }
}

impl<L: LabelKind> Menu<L> {
    /// Strips any custom trigger visual view, leaving the semantic menu item
    /// used inside nested menus and menu bars.
    #[must_use]
    pub fn into_semantic(self) -> Menu<SemanticLabel> {
        Menu {
            label: self.label.into_semantic(),
            items: self.items,
            identity: self.identity,
            style: self.style,
            size: self.size,
        }
    }

    #[must_use]
    fn resolve<S: ChildSlot>(self, env: &Environment) -> ResolvedNestedMenu<S> {
        let label = self.label.into_semantic();
        let resolved_label = label.semantic_text().resolve(env);
        let icon = label.icon_slot::<S>();
        let semantic_label = label.resolve(env);
        ResolvedNestedMenu {
            label: resolved_label,
            semantic_label,
            icon,
            items: resolve_menu_items::<S>(&self.items, env),
            identity: self.identity,
        }
    }

    fn into_resolved<S: ChildSlot>(self, env: &Environment) -> ResolvedMenu<S> {
        let label = self.label.into_control_label::<S>(env);
        let accessibility_label = label.accessibility_label();
        ResolvedMenu {
            label,
            items: resolve_menu_items::<S>(&self.items, env),
            accessibility_label,
            style: self.style.resolve(env),
            size: self.size,
        }
    }
}

impl<L: LabelKind> View for Menu<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_resolved::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_resolved::<B::Slot>(env), env, cx)
    }
}

/// A semantic command that can be reused across menu-like surfaces.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Command {
    /// The semantic label for the command.
    label: Label<SemanticLabel>,
    /// The action to perform when the command is selected.
    action: SharedAction<()>,
    /// Whether the command is currently disabled.
    disabled: Computed<bool>,
    /// Whether the command is currently selected or checked.
    selected: Computed<bool>,
    /// Optional keyboard shortcut metadata.
    shortcut: Option<Shortcut>,
    /// What the command does, which decides its presentation.
    role: Option<ButtonRole>,
    /// An optional secondary line shown under the label.
    subtitle: Option<Text>,
    /// Local state layered onto the environment when the action runs.
    captured_env: Environment,
    identity: Rc<()>,
}

impl Command {
    /// The semantic label for the command.
    #[must_use]
    pub const fn label(&self) -> &Label<SemanticLabel> {
        &self.label
    }

    /// The action to perform when the command is selected.
    #[must_use]
    pub const fn action(&self) -> &SharedAction<()> {
        &self.action
    }

    /// Whether the command is currently disabled.
    #[must_use]
    pub const fn is_disabled(&self) -> &Computed<bool> {
        &self.disabled
    }

    /// Whether the command is currently selected or checked.
    #[must_use]
    pub const fn is_selected(&self) -> &Computed<bool> {
        &self.selected
    }

    /// Optional keyboard shortcut metadata.
    #[must_use]
    pub const fn command_shortcut(&self) -> Option<&Shortcut> {
        self.shortcut.as_ref()
    }

    /// What the command does, which decides its presentation.
    #[must_use]
    pub const fn command_role(&self) -> Option<ButtonRole> {
        self.role
    }

    /// An optional secondary line shown under the label.
    #[must_use]
    pub const fn command_subtitle(&self) -> Option<&Text> {
        self.subtitle.as_ref()
    }

    /// Local state layered onto the environment when the action runs.
    #[must_use]
    pub const fn captured_env(&self) -> &Environment {
        &self.captured_env
    }
}

impl_constant!(Command);

impl Command {
    /// Creates a command builder with the given label.
    #[must_use]
    pub fn builder(label: impl IntoLabel) -> CommandBuilder {
        Self::from_semantic_label(label.into_label().into_semantic())
    }

    pub(crate) const fn from_semantic_label(label: Label<SemanticLabel>) -> CommandBuilder {
        CommandBuilder { label }
    }

    /// Resolves the command against `env` into the payload backends render
    /// — what every declared command goes through, and how a backend builds
    /// a command of its own (the platform's Quit) on the same path.
    #[doc(hidden)]
    #[must_use]
    pub fn resolve<S: ChildSlot>(self, env: &Environment) -> ResolvedCommand<S> {
        let label = self.label;
        let resolved_label = label.semantic_text().resolve(env);
        let icon = label.icon_slot::<S>();
        let semantic_label = label.resolve(env);
        let action = self.action;
        let captured_env = self.captured_env;
        ResolvedCommand {
            label: resolved_label,
            semantic_label,
            icon,
            action: shared_action(move |env: Environment| {
                action.run(&captured_env.layered_on(&env));
            }),
            disabled: Disabled::resolve(env, self.disabled),
            selected: self.selected,
            shortcut: self.shortcut,
            role: self.role,
            subtitle: self.subtitle.map(|subtitle| {
                subtitle
                    .resolve(env)
                    .content()
                    .map(|styled| styled.to_plain())
                    .computed()
            }),
            identity: self.identity,
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
    pub const fn shortcut(mut self, shortcut: Shortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Sets what the command does, which decides its presentation.
    #[must_use]
    pub const fn role(mut self, role: ButtonRole) -> Self {
        self.role = Some(role);
        self
    }

    /// Marks the command as destructive.
    #[must_use]
    pub const fn destructive(self) -> Self {
        self.role(ButtonRole::Destructive)
    }

    /// Shows a secondary line under the command's label.
    ///
    /// Accepts any [`IntoText`] value so string literals resolve through the
    /// active translation catalog in the surrounding [`Environment`].
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl IntoText) -> Self {
        self.subtitle = Some(subtitle.into_text());
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
    label: Label<SemanticLabel>,
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
            role: None,
            subtitle: None,
            captured_env: Environment::new(),
            identity: Rc::new(()),
        }
    }
}

/// A single item inside menu content.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum MenuItem {
    /// A leaf command.
    Command(Command),
    /// A divider separating adjacent commands or menus.
    Divider,
    /// A nested menu.
    Menu(Menu<SemanticLabel>),
    /// The application's own Quit item.
    ///
    /// Declaring it is how an application relocates the standard Quit — it
    /// renders once, with the platform's label and accelerator (⌘Q on
    /// macOS, Ctrl+Q elsewhere), and requests a cancellable termination
    /// through `waterui::app::Quit`, so `App::on_quit_request` still
    /// decides. On macOS it never appears in the menu bar — the standard
    /// application menu already carries the platform Quit — but it can
    /// appear in menus the window mounts.
    ///
    /// Platforms with no application quit — iOS, Android, the web, and
    /// headless or embedded hosts, whose process the system or the host
    /// ends — omit it from every menu and arm no chord for it, so a
    /// portable menu may declare it unconditionally.
    ///
    /// Do not redeclare the platform's quit chord on a plain [`Command`]:
    /// the macOS menu bar rejects ⌘Q, ⌘H and ⌥⌘H on ordinary commands, and
    /// a homemade Quit bypasses the termination hooks.
    Quit,
    /// The application's own Close Window item.
    ///
    /// It renders once, with the platform's label ("Close"), and closes a
    /// window through that window's ordinary close path — the same one its
    /// title-bar close button takes — so the key window closes exactly as
    /// that button would close it, and closing the last window still
    /// follows the `LastWindowPolicy` passed to `App::on_last_window_closed`.
    /// A window declared without a close button ignores it.
    ///
    /// Its accelerator is the platform's close chord — ⌘W on macOS, Ctrl+W
    /// where the command modifier maps to control — unless the application
    /// binds the chord itself, which it may: a tabbed app's ⌘W is Close
    /// Tab. The chord is decided once for the whole application, from the
    /// declared menu bar at any depth — ⌘W when no declared command binds
    /// it, ⇧⌘W when one does and none binds ⇧⌘W, no chord at all when the
    /// application binds both — and every Close Window item carries that
    /// one decision: the menu bar, a window-mounted menu, a context menu
    /// and a chord-table entry alike. On Windows the menu-bar row shows
    /// Alt+F4, the system's own close chord, and the decided chord reaches
    /// the focused window through the chord registry; Linux has no menu
    /// bar, so there it works through the shortcut registry and the item
    /// shows only in mounted and context menus.
    ///
    /// On Windows the menu bar is one menu every window shares, and its
    /// Close row is the platform's own item, which cannot be disabled per
    /// window: it stays enabled while a non-closable window is focused, and
    /// choosing it there does nothing.
    ///
    /// Declaring it is how an application places Close Window. When no menu
    /// in the menu bar declares it, the macOS menu bar carries it in the
    /// standard Window menu; once one does, it appears only where it is
    /// declared, never twice. It can also appear in menus the window mounts
    /// and in context menus.
    ///
    /// Platforms whose windows the application cannot close — iOS, Android,
    /// the web, and FFI hosts, whose windows the host owns — omit it from
    /// every menu and arm no chord for it, so a portable menu may declare
    /// it unconditionally.
    CloseWindow,
}

impl_constant!(MenuItem);

impl MenuItem {
    #[must_use]
    fn resolve<S: ChildSlot>(self, env: &Environment) -> ResolvedMenuItem<S> {
        match self {
            Self::Command(command) => ResolvedMenuItem::Command(command.resolve::<S>(env)),
            Self::Divider => ResolvedMenuItem::Divider,
            Self::Menu(menu) => ResolvedMenuItem::Menu(menu.resolve::<S>(env)),
            Self::Quit => ResolvedMenuItem::Quit,
            Self::CloseWindow => ResolvedMenuItem::CloseWindow,
        }
    }

    /// The command a declared [`MenuItem::CloseWindow`] stands for — how a
    /// backend that draws it as an ordinary command row (a popup-menu row, a
    /// chord in its shortcut table) builds that row: the platform's word for
    /// closing a window, the `chord` [`CloseWindowChord`] resolves from the
    /// environment — the application's one decision, ⌘W on macOS, Ctrl+W
    /// where the backend maps the command modifier to control, ceded when
    /// the application binds it — and `action`, which the backend supplies
    /// because the backend owns window targeting — which window the request
    /// reaches, and how it closes.
    #[doc(hidden)]
    #[must_use]
    pub fn close_window_command<S: ChildSlot, Args>(
        env: &Environment,
        chord: Option<Shortcut>,
        action: impl Handler<Args, ()> + 'static,
    ) -> ResolvedCommand<S> {
        // "Close" is the item AppKit's File and Window menus, the Windows
        // window menu and GTK/Qt desktops all carry for ⌘W / Ctrl+W.
        let command = "Close".action(action);
        let command = if let Some(chord) = chord {
            command.shortcut(chord)
        } else {
            command
        };
        command.resolve::<S>(env)
    }
}

impl From<Command> for MenuItem {
    fn from(value: Command) -> Self {
        Self::Command(value)
    }
}

impl<L: LabelKind> From<Menu<L>> for MenuItem {
    fn from(value: Menu<L>) -> Self {
        Self::Menu(value.into_semantic())
    }
}

/// Converts semantic menu content into menu items.
///
/// This trait is implemented for the supported menu content building blocks:
/// [`Command`], [`Menu`], `Divider` (in `waterui`), ordinary [`Button`] values, and
/// containers such as tuples, arrays, vectors, and [`Option`].
pub trait MenuView {
    /// Converts this value into reactive menu items.
    fn into_menu_items(self) -> Computed<Vec<MenuItem>>;
}

/// Converts top-level system menu bar content into menus.
pub trait MenuBarView {
    /// Converts this value into reactive top-level menus.
    fn into_menus(self) -> Computed<Vec<Menu<SemanticLabel>>>;
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

impl<L: LabelKind> MenuView for Menu<L> {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        Computed::constant(vec![MenuItem::Menu(self.into_semantic())])
    }
}

impl<L: LabelKind, Action> MenuView for Button<L, Action>
where
    Action: FnMut(&Environment) -> Result<(), waterui_core::handler::ExtractionError> + 'static,
{
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        Computed::constant(vec![self.into()])
    }
}

impl MenuBarView for () {
    fn into_menus(self) -> Computed<Vec<Menu<SemanticLabel>>> {
        Computed::constant(Vec::new())
    }
}

impl<T: MenuBarView> MenuBarView for Option<T> {
    fn into_menus(self) -> Computed<Vec<Menu<SemanticLabel>>> {
        self.map_or_else(|| Computed::constant(Vec::new()), MenuBarView::into_menus)
    }
}

impl<L: LabelKind + Clone> MenuBarView for Computed<Vec<Menu<L>>> {
    fn into_menus(self) -> Computed<Vec<Menu<SemanticLabel>>> {
        self.map(|menus| menus.into_iter().map(Menu::into_semantic).collect())
            .computed()
    }
}

impl<L: LabelKind> MenuBarView for Menu<L> {
    fn into_menus(self) -> Computed<Vec<Menu<SemanticLabel>>> {
        Computed::constant(vec![self.into_semantic()])
    }
}

impl<L: LabelKind> MenuBarView for Vec<Menu<L>> {
    fn into_menus(self) -> Computed<Vec<Menu<SemanticLabel>>> {
        Computed::constant(self.into_iter().map(Menu::into_semantic).collect())
    }
}

impl<L: LabelKind, const N: usize> MenuBarView for [Menu<L>; N] {
    fn into_menus(self) -> Computed<Vec<Menu<SemanticLabel>>> {
        Computed::constant(self.into_iter().map(Menu::into_semantic).collect())
    }
}

/// A `Signal` that concatenates the `Vec` outputs of its children.
///
/// One evaluation reads every child once and concatenates the results in
/// order, so an update costs linear work in the total item count instead of
/// propagating through an intermediate `zip` layer per element.
struct ConcatVecs<T> {
    children: Rc<Vec<Computed<Vec<T>>>>,
}

impl<T> Clone for ConcatVecs<T> {
    fn clone(&self) -> Self {
        Self {
            children: self.children.clone(),
        }
    }
}

impl<T: 'static> Signal for ConcatVecs<T> {
    type Output = Vec<T>;
    type Guard = ConcatVecsGuard;

    fn snapshot(&self) -> Vec<T> {
        self.children.iter().flat_map(Computed::snapshot).collect()
    }

    fn identity(&self) -> Option<SignalIdentity> {
        let mut children = self.children.iter();
        let mut identity = children.next()?.identity()?;
        for child in children {
            identity = identity.combine(child.identity()?);
        }
        Some(identity)
    }

    fn watch(&self, watcher: impl Fn(Context<Vec<T>>) + 'static) -> Self::Guard {
        let watcher = Rc::new(watcher);
        let guards = self
            .children
            .iter()
            .map(|child| {
                let watcher = Rc::clone(&watcher);
                let children = Rc::clone(&self.children);
                child.watch(move |ctx| {
                    let items = children.iter().flat_map(Computed::snapshot).collect();
                    watcher(ctx.map(|_| items));
                })
            })
            .collect();
        ConcatVecsGuard { _guards: guards }
    }
}

/// Owns the per-child subscriptions opened for one `ConcatVecs` watcher.
struct ConcatVecsGuard {
    _guards: Vec<BoxWatcherGuard>,
}

impl WatcherGuard for ConcatVecsGuard {}

fn concat_vec_children<T: 'static>(children: Vec<Computed<Vec<T>>>) -> Computed<Vec<T>> {
    Computed::new(ConcatVecs {
        children: Rc::new(children),
    })
}

impl<T: MenuView> MenuView for Vec<T> {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        concat_vec_children(self.into_iter().map(MenuView::into_menu_items).collect())
    }
}

impl<T: MenuView, const N: usize> MenuView for [T; N] {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        concat_vec_children(self.into_iter().map(MenuView::into_menu_items).collect())
    }
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
                concat_vec_children(vec![$T0.into_menu_items(), $($T.into_menu_items()),+])
            }
        }
    };
}

menu_tuples!(impl_tuple_menu_view);

macro_rules! impl_tuple_menu_bar_view {
    ($T0:ident) => {
        impl<$T0: MenuBarView> MenuBarView for ($T0,) {
            fn into_menus(self) -> Computed<Vec<Menu<SemanticLabel>>> {
                self.0.into_menus()
            }
        }
    };
    ($T0:ident, $($T:ident),+) => {
        #[allow(non_snake_case)]
        impl<$T0: MenuBarView, $($T: MenuBarView),+> MenuBarView for ($T0, $($T,)+) {
            fn into_menus(self) -> Computed<Vec<Menu<SemanticLabel>>> {
                let ($T0, $($T),+) = self;
                concat_vec_children(vec![$T0.into_menus(), $($T.into_menus()),+])
            }
        }
    };
}

menu_tuples!(impl_tuple_menu_bar_view);

fn resolve_menu_items_now<S: ChildSlot>(
    items: Vec<MenuItem>,
    env: &Environment,
) -> Vec<ResolvedMenuItem<S>> {
    items
        .into_iter()
        .map(|item| item.resolve::<S>(env))
        .collect()
}

#[doc(hidden)]
#[must_use]
pub fn resolve_menu_items<S: ChildSlot>(
    items: &Computed<Vec<MenuItem>>,
    env: &Environment,
) -> Computed<Vec<ResolvedMenuItem<S>>> {
    let locale = locale_binding(env);
    let env = env.clone();
    items
        .zip(&locale)
        .map(move |(items, _locale)| resolve_menu_items_now::<S>(items, &env))
        .computed()
}

#[doc(hidden)]
#[must_use]
pub fn resolve_menu_bar_items<L: LabelKind + Clone, S: ChildSlot>(
    menus: &Computed<Vec<Menu<L>>>,
    env: &Environment,
) -> Computed<Vec<ResolvedMenuItem<S>>> {
    let locale = locale_binding(env);
    let env = env.clone();
    menus
        .zip(&locale)
        .map(move |(menus, _locale)| {
            menus
                .into_iter()
                .map(|menu| ResolvedMenuItem::Menu(menu.into_semantic().resolve::<S>(&env)))
                .collect()
        })
        .computed()
}

/// Raw resolved command payload consumed by native backends.
#[doc(hidden)]
#[derive(Debug)]
pub struct ResolvedCommand<S = NoopSlot> {
    /// Resolved text config for native menu presentation.
    label: TextConfig,
    /// The full semantic label retained for self-drawn menu renderers.
    semantic_label: Label<SemanticLabel>,
    /// Optional icon slot lifted out of the label for platform chrome.
    icon: Option<IconSlot<S>>,
    /// Shared action invoked by the native backend.
    action: SharedAction<()>,
    /// Whether the command is currently disabled.
    disabled: Computed<bool>,
    /// Whether the command is currently selected or checked.
    selected: Computed<bool>,
    /// Optional keyboard shortcut metadata.
    shortcut: Option<Shortcut>,
    /// What the command does, which decides its presentation.
    role: Option<ButtonRole>,
    /// An optional secondary line shown under the label.
    subtitle: Option<Computed<Str>>,
    identity: Rc<()>,
}

impl<S> Clone for ResolvedCommand<S> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            semantic_label: self.semantic_label.clone(),
            icon: self.icon.clone(),
            action: self.action.clone(),
            disabled: self.disabled.clone(),
            selected: self.selected.clone(),
            shortcut: self.shortcut.clone(),
            role: self.role,
            subtitle: self.subtitle.clone(),
            identity: Rc::clone(&self.identity),
        }
    }
}

impl<S: 'static> nami::Signal for ResolvedCommand<S> {
    type Output = Self;
    type Guard = ();

    fn snapshot(&self) -> Self::Output {
        self.clone()
    }

    fn watch(&self, _watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {}
}

impl<S: 'static> ResolvedCommand<S> {
    /// Maps the optional icon slot through `f`.
    #[must_use]
    pub fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> ResolvedCommand<T> {
        ResolvedCommand {
            label: self.label,
            semantic_label: self.semantic_label,
            icon: self.icon.map(|icon| icon.map_slot(f)),
            action: self.action,
            disabled: self.disabled,
            selected: self.selected,
            shortcut: self.shortcut,
            role: self.role,
            subtitle: self.subtitle,
            identity: self.identity,
        }
    }
}

impl<S> ResolvedCommand<S> {
    /// Resolved text config for native menu presentation.
    #[must_use]
    pub const fn label(&self) -> &TextConfig {
        &self.label
    }

    /// The full semantic label retained for self-drawn menu renderers.
    #[must_use]
    pub const fn semantic_label(&self) -> &Label<SemanticLabel> {
        &self.semantic_label
    }

    /// Optional icon slot lifted out of the label for platform chrome.
    #[must_use]
    pub const fn icon(&self) -> Option<&IconSlot<S>> {
        self.icon.as_ref()
    }

    /// Shared action invoked by the native backend.
    #[must_use]
    pub const fn action(&self) -> &SharedAction<()> {
        &self.action
    }

    /// Whether the command is currently disabled.
    #[must_use]
    pub const fn is_disabled(&self) -> &Computed<bool> {
        &self.disabled
    }

    /// Replaces the disabled signal for a resolved command.
    pub fn set_disabled(&mut self, disabled: Computed<bool>) {
        self.disabled = disabled;
    }

    /// Whether the command is currently selected or checked.
    #[must_use]
    pub const fn is_selected(&self) -> &Computed<bool> {
        &self.selected
    }

    /// Optional keyboard shortcut metadata.
    #[must_use]
    pub const fn shortcut(&self) -> Option<&Shortcut> {
        self.shortcut.as_ref()
    }

    /// What the command does, which decides its presentation.
    #[must_use]
    pub const fn role(&self) -> Option<ButtonRole> {
        self.role
    }

    /// An optional secondary line shown under the label.
    #[must_use]
    pub const fn subtitle(&self) -> Option<&Computed<Str>> {
        self.subtitle.as_ref()
    }

    /// Stable semantic identity used by native collection reconciliation.
    #[doc(hidden)]
    #[must_use]
    pub fn semantic_id(&self) -> usize {
        Rc::as_ptr(&self.identity) as usize
    }

    /// Whether the macOS menu bar may carry this command, reporting a
    /// contract violation when it may not.
    ///
    /// ⌘Q, ⌘H and ⌥⌘H belong to the standard application-menu items — Quit,
    /// Hide, Hide Others. A plain command redeclaring one would shadow the
    /// item the system expects to find, and a homemade Quit bypasses the
    /// termination hooks. Every backend that builds a macOS menu bar calls
    /// this for each declared command, so the rule holds whichever backend
    /// renders the bar.
    ///
    /// # Contract violation
    ///
    /// When the command's shortcut is one of the reserved chords; the
    /// message names `MenuItem::Quit` and `App::on_terminate` as the APIs to
    /// use instead. A debug build panics; a release build logs it, returns
    /// `false`, and the backend leaves the command out of the bar.
    #[doc(hidden)]
    #[must_use]
    pub fn allowed_in_macos_menu_bar(&self) -> bool {
        let Some(shortcut) = &self.shortcut else {
            return true;
        };
        let reserved = [
            (Shortcut::new('q').command(), "⌘Q", "Quit"),
            (Shortcut::new('h').command(), "⌘H", "Hide"),
            (Shortcut::new('h').command().option(), "⌥⌘H", "Hide Others"),
        ];
        for (chord, chord_text, item) in reserved {
            if chord.modifiers() == shortcut.modifiers()
                && shortcut.key().matches(&chord.key().to_key())
            {
                contract_violation!(
                    "a menu command in the macOS menu bar may not use the {chord_text} chord — it \
                     belongs to the standard {item} item; declare `MenuItem::Quit` for Quit \
                     behavior and `App::on_terminate` for shutdown work instead of redeclaring the \
                     chord; the command is left out of the menu bar"
                );
                return false;
            }
        }
        true
    }
}

/// Raw resolved nested menu payload consumed by native backends.
#[doc(hidden)]
#[derive(Debug)]
pub struct ResolvedNestedMenu<S = NoopSlot> {
    /// Resolved text config for the nested menu title.
    label: TextConfig,
    /// The full semantic label retained for self-drawn menu renderers.
    semantic_label: Label<SemanticLabel>,
    /// Optional icon slot lifted out of the label for platform chrome.
    icon: Option<IconSlot<S>>,
    /// Resolved nested menu items.
    items: Computed<Vec<ResolvedMenuItem<S>>>,
    identity: Rc<()>,
}

impl<S> Clone for ResolvedNestedMenu<S> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            semantic_label: self.semantic_label.clone(),
            icon: self.icon.clone(),
            items: self.items.clone(),
            identity: Rc::clone(&self.identity),
        }
    }
}

impl<S: 'static> nami::Signal for ResolvedNestedMenu<S> {
    type Output = Self;
    type Guard = ();

    fn snapshot(&self) -> Self::Output {
        self.clone()
    }

    fn watch(&self, _watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {}
}

impl<S: 'static> ResolvedNestedMenu<S> {
    /// Maps every icon slot inside this nested menu through `f`.
    #[must_use]
    pub fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> ResolvedNestedMenu<T> {
        let item_f = f.clone();
        ResolvedNestedMenu {
            label: self.label,
            semantic_label: self.semantic_label,
            icon: self.icon.map(|icon| icon.map_slot(f)),
            items: self
                .items
                .map(move |items| {
                    items
                        .into_iter()
                        .map(|item| item.map_slots(item_f.clone()))
                        .collect()
                })
                .computed(),
            identity: self.identity,
        }
    }
}

impl<S> ResolvedNestedMenu<S> {
    /// Resolved text config for the nested menu title.
    #[must_use]
    pub const fn label(&self) -> &TextConfig {
        &self.label
    }

    /// The full semantic label retained for self-drawn menu renderers.
    #[must_use]
    pub const fn semantic_label(&self) -> &Label<SemanticLabel> {
        &self.semantic_label
    }

    /// Optional icon slot lifted out of the label for platform chrome.
    #[must_use]
    pub const fn icon(&self) -> Option<&IconSlot<S>> {
        self.icon.as_ref()
    }

    /// Resolved nested menu items.
    #[must_use]
    pub const fn items(&self) -> &Computed<Vec<ResolvedMenuItem<S>>> {
        &self.items
    }

    /// Stable semantic identity used by native collection reconciliation.
    #[doc(hidden)]
    #[must_use]
    pub fn semantic_id(&self) -> usize {
        Rc::as_ptr(&self.identity) as usize
    }
}

/// Raw menu item config resolved against the environment locale.
#[doc(hidden)]
#[derive(Debug)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each menu item variant by variant; a new menu item must fail their build instead of falling into a wildcard arm"
)]
pub enum ResolvedMenuItem<S = NoopSlot> {
    /// A resolved leaf command.
    Command(ResolvedCommand<S>),
    /// A divider separating sibling items.
    Divider,
    /// A resolved nested menu.
    Menu(ResolvedNestedMenu<S>),
    /// The application's declared Quit item — see [`MenuItem::Quit`].
    Quit,
    /// The application's declared Close Window item — see
    /// [`MenuItem::CloseWindow`].
    CloseWindow,
}

impl<S> Clone for ResolvedMenuItem<S> {
    fn clone(&self) -> Self {
        match self {
            Self::Command(command) => Self::Command(command.clone()),
            Self::Divider => Self::Divider,
            Self::Menu(menu) => Self::Menu(menu.clone()),
            Self::Quit => Self::Quit,
            Self::CloseWindow => Self::CloseWindow,
        }
    }
}

impl<S: 'static> ResolvedMenuItem<S> {
    /// Maps every icon slot inside this resolved menu item through `f`.
    #[must_use]
    pub fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> ResolvedMenuItem<T> {
        match self {
            Self::Command(command) => ResolvedMenuItem::Command(command.map_slots(f)),
            Self::Divider => ResolvedMenuItem::Divider,
            Self::Menu(menu) => ResolvedMenuItem::Menu(menu.map_slots(f)),
            Self::Quit => ResolvedMenuItem::Quit,
            Self::CloseWindow => ResolvedMenuItem::CloseWindow,
        }
    }
}

impl<S: 'static> ResolvedMenuItem<S> {
    /// Whether `items` declare [`MenuItem::CloseWindow`] at any depth.
    ///
    /// A backend that supplies a standard Close Window item when the
    /// application declares none asks this of the resolved menu bar, so the
    /// item never appears twice.
    #[doc(hidden)]
    #[must_use]
    pub fn declares_close_window(items: &[Self]) -> bool {
        items.iter().any(|item| match item {
            Self::CloseWindow => true,
            Self::Menu(menu) => Self::declares_close_window(&menu.items.snapshot()),
            Self::Command(_) | Self::Divider | Self::Quit => false,
        })
    }

    /// The chord a menu bar's Close Window item carries, decided once from
    /// the commands `items` declare at any depth: the platform's close
    /// chord — ⌘W, which backends map to Ctrl+W where the command modifier
    /// is control — while no declared command binds it; ⇧⌘W when one does
    /// and none binds ⇧⌘W (the tabbed-app convention: a browser's ⌘W is
    /// Close Tab); and no chord at all when the application binds both, so
    /// Close Window never shadows a command's own binding.
    ///
    /// The comparison is [`ResolvedCommand::allowed_in_macos_menu_bar`]'s:
    /// equal modifier sets, and the key matched case-insensitively.
    #[doc(hidden)]
    #[must_use]
    pub fn close_window_chord(items: &[Self]) -> Option<Shortcut> {
        let close = Shortcut::new('w').command();
        let shifted = Shortcut::new('w').command().shift();
        if binds_chord(items, &close) {
            (!binds_chord(items, &shifted)).then_some(shifted)
        } else {
            Some(close)
        }
    }
}

/// Whether a declared command in `items` — at any depth — binds `chord`,
/// compared the way [`ResolvedCommand::allowed_in_macos_menu_bar`]
/// does: equal modifiers, key matched case-insensitively.
fn binds_chord<S: 'static>(items: &[ResolvedMenuItem<S>], chord: &Shortcut) -> bool {
    items.iter().any(|item| match item {
        ResolvedMenuItem::Command(command) => command.shortcut.as_ref().is_some_and(|shortcut| {
            chord.modifiers() == shortcut.modifiers()
                && shortcut.key().matches(&chord.key().to_key())
        }),
        ResolvedMenuItem::Menu(menu) => binds_chord(&menu.items.snapshot(), chord),
        ResolvedMenuItem::Divider | ResolvedMenuItem::Quit | ResolvedMenuItem::CloseWindow => false,
    })
}

/// The application's decided Close Window chord: one decision for the whole
/// application, taken from its declared menu bar through
/// [`ResolvedMenuItem::close_window_chord`], that every Close Window item
/// carries wherever it renders — the menu bar, window-mounted menus,
/// context menus and chord-table entries alike.
///
/// `App::into_parts` installs it into the application environment as a
/// signal over the resolved menu bar, so a bar whose declared items change
/// re-decides it.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct CloseWindowChord(Computed<Option<Shortcut>>);

impl CloseWindowChord {
    /// The standard close-window chord when an application declares no menu bar.
    #[must_use]
    pub fn standard() -> Self {
        Self(Computed::constant(Some(Shortcut::new('w').command())))
    }

    /// The decision over `menu_bar`, resolved under `env` — a signal, so a
    /// menu bar whose items change re-decides the chord.
    #[must_use]
    pub fn new(menu_bar: &Computed<Vec<Menu<SemanticLabel>>>, env: &Environment) -> Self {
        Self(
            resolve_menu_bar_items::<SemanticLabel, NoopSlot>(menu_bar, env)
                .map(|items| ResolvedMenuItem::close_window_chord(&items))
                .computed(),
        )
    }

    /// The chord this environment's application decided.
    ///
    /// # Panics
    ///
    /// When `env` carries no decision — every environment a runner hands
    /// its menus comes from `App::into_parts`, which installs the decision;
    /// a bare `Environment` must install it itself.
    #[must_use]
    pub fn of(env: &Environment) -> Option<Shortcut> {
        env.get::<Self>()
            .expect("the Close Window chord is installed by `App::into_parts`")
            .0
            .snapshot()
    }
}

/// Where a menu bar's Close Window item lives: in the standard Window menu
/// the platform or backend builds, or only where the application declared
/// [`MenuItem::CloseWindow`] — never both, so it never appears twice.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloseWindowPlacement {
    /// No declared menu carries `MenuItem::CloseWindow`: the standard
    /// Window menu carries Close, so the close chord works without the
    /// application declaring anything.
    WindowMenu,
    /// A declared menu carries it, at whatever depth; the standard Window
    /// menu does not repeat it.
    Declared,
}

impl CloseWindowPlacement {
    /// The placement the declared menu bar's resolved `items` call for.
    #[must_use]
    pub fn for_declared<S: 'static>(items: &[ResolvedMenuItem<S>]) -> Self {
        if ResolvedMenuItem::declares_close_window(items) {
            Self::Declared
        } else {
            Self::WindowMenu
        }
    }
}

impl<S: 'static> nami::Signal for ResolvedMenuItem<S> {
    type Output = Self;
    type Guard = ();

    fn snapshot(&self) -> Self::Output {
        self.clone()
    }

    fn watch(&self, _watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {}
}

/// Raw native popup menu config resolved against the environment locale.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ResolvedMenu<S> {
    /// The label displayed on the menu button.
    pub label: ControlLabel<S>,
    /// The fully resolved menu items.
    pub items: Computed<Vec<ResolvedMenuItem<S>>>,
    /// Semantic accessibility label for the menu trigger.
    pub accessibility_label: Computed<StyledStr>,
    /// The style the trigger button is drawn in, `Automatic` already
    /// resolved against the ambient [`ButtonStyle`].
    pub style: ButtonStyle,
    /// How large the trigger button is drawn.
    pub size: ControlSize,
}

impl<S: 'static> NativeView for ResolvedMenu<S> {}

impl<S: 'static> MapSlots<S> for ResolvedMenu<S> {
    type With<T: 'static> = ResolvedMenu<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> ResolvedMenu<T> {
        let item_f = f.clone();
        ResolvedMenu {
            label: self.label.map_slots(f),
            items: self
                .items
                .map(move |items| {
                    items
                        .into_iter()
                        .map(|item| item.map_slots(item_f.clone()))
                        .collect()
                })
                .computed(),
            accessibility_label: self.accessibility_label,
            style: self.style,
            size: self.size,
        }
    }
}

impl<S: View> View for ResolvedMenu<S> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.map_slots(AnyView::new))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.map_slots(B::Slot::from_view), env, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::button::button;
    use alloc::rc::Rc;
    use alloc::string::ToString;
    use core::cell::Cell;
    use nami::Signal;
    use waterui_interaction::key::Key;

    #[test]
    fn shortcut_key_parses_characters_and_armable_named_keys() {
        assert_eq!("q".parse(), Ok(ShortcutKey::Character('q')));
        assert_eq!("Q".parse(), Ok(ShortcutKey::Character('Q')));
        assert_eq!(
            "Delete".parse(),
            Ok(ShortcutKey::Named(ShortcutNamedKey::Delete))
        );
        assert_eq!("F5".parse(), Ok(ShortcutKey::Named(ShortcutNamedKey::F5)));
        assert_eq!("F24".parse(), Ok(ShortcutKey::Named(ShortcutNamedKey::F24)));
        for rejected in [
            "",
            "ab",
            "Unidentified",
            "Dead",
            "\t",
            "Shift",
            "CapsLock",
            "MediaPlayPause",
            "AudioVolumeUp",
            "F25",
        ] {
            assert_eq!(
                rejected.parse::<ShortcutKey>(),
                Err(InvalidShortcutKey::new(rejected.to_string())),
                "`{rejected}` is not a shortcut key"
            );
        }
    }

    #[test]
    fn shortcut_key_matches_letters_regardless_of_case() {
        let lower = ShortcutKey::from('q');
        let upper = ShortcutKey::from('Q');
        for pressed in ["q", "Q"] {
            let pressed = Key::Character(pressed.to_string());
            assert!(lower.matches(&pressed));
            assert!(upper.matches(&pressed));
        }
        assert!(!lower.matches(&Key::Character("w".to_string())));
        assert!(!lower.matches(&Key::Character("qq".to_string())));
        assert!(!lower.matches(&Key::Named(NamedKey::Delete)));
        let delete = ShortcutKey::from(ShortcutNamedKey::Delete);
        assert!(delete.matches(&Key::Named(NamedKey::Delete)));
        assert!(!delete.matches(&Key::Named(NamedKey::Backspace)));
    }

    #[test]
    fn try_from_named_key_accepts_armable_and_rejects_unarmable_keys() {
        assert_eq!(
            ShortcutKey::try_from(NamedKey::Delete),
            Ok(ShortcutKey::Named(ShortcutNamedKey::Delete))
        );
        assert_eq!(
            ShortcutKey::try_from(NamedKey::F24),
            Ok(ShortcutKey::Named(ShortcutNamedKey::F24))
        );
        for unsupported in [
            NamedKey::Unidentified,
            NamedKey::Dead,
            NamedKey::Shift,
            NamedKey::MediaPlayPause,
            NamedKey::AudioVolumeUp,
            NamedKey::BrowserBack,
            NamedKey::F25,
        ] {
            assert_eq!(
                ShortcutKey::try_from(unsupported),
                Err(InvalidShortcutKey::new(unsupported.to_string()))
            );
        }
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "`ShortcutNamedKey::Tab`"))]
    fn shortcut_key_refuses_a_control_character_naming_its_key() {
        let key = ShortcutKey::from('\t');
        assert_eq!(key, ShortcutKey::Character('\t'));
        assert!(!key.matches(&Key::Named(NamedKey::Tab)));
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "may not use the ⌘Q chord"))]
    fn macos_menu_bar_refuses_the_reserved_quit_chord() {
        crate::init_test_executor();
        let env = Environment::default();
        let items = vec![
            Command::builder("Quit")
                .action(|| {})
                .shortcut(Shortcut::new('q').command())
                .into(),
            Command::builder("Open")
                .action(|| {})
                .shortcut(Shortcut::new('o').command())
                .into(),
        ];
        let resolved = resolve_menu_items_now::<NoopSlot>(items, &env);
        let ResolvedMenuItem::Command(quit) = &resolved[0] else {
            panic!("first resolved item should be a command");
        };
        let ResolvedMenuItem::Command(open) = &resolved[1] else {
            panic!("second resolved item should be a command");
        };
        assert!(open.allowed_in_macos_menu_bar());
        // Release outcome: the command is reported not allowed, so the
        // backend leaves it out of the bar.
        assert!(!quit.allowed_in_macos_menu_bar());
    }

    /// `Display` is the platform's menu hint: `⇧⌘P` on Apple targets,
    /// `Ctrl+Shift+P` where the command modifier is control.
    #[test]
    fn shortcut_displays_its_platform_hint() {
        let shortcut = Shortcut::new('p').command().shift();
        #[cfg(target_vendor = "apple")]
        assert_eq!(shortcut.to_string(), "⇧⌘P");
        #[cfg(not(target_vendor = "apple"))]
        assert_eq!(shortcut.to_string(), "Ctrl+Shift+P");
    }

    /// A named key draws the platform's own label — `⌦` on Apple targets,
    /// `Del` elsewhere.
    #[test]
    fn shortcut_displays_named_key_labels() {
        let shortcut = Shortcut::new(ShortcutNamedKey::Delete).command();
        #[cfg(target_vendor = "apple")]
        assert_eq!(shortcut.to_string(), "⌘⌦");
        #[cfg(not(target_vendor = "apple"))]
        assert_eq!(shortcut.to_string(), "Ctrl+Del");
    }

    #[test]
    fn menu_view_accepts_plain_buttons() {
        crate::init_test_executor();
        let items = button(crate::label::label("Search").icon("Q"))
            .action(|| {})
            .into_menu_items()
            .snapshot();
        assert_eq!(items.len(), 1);
        let MenuItem::Command(command) = &items[0] else {
            panic!("button menu content should resolve to a command");
        };
        assert_eq!(
            command
                .label
                .semantic_text()
                .resolve(&Environment::default())
                .content()
                .snapshot()
                .to_plain()
                .as_str(),
            "Search"
        );
    }

    #[test]
    fn menu_view_flattens_vectors_of_supported_items() {
        let items: Vec<MenuItem> = vec![
            button("Edit").action(|| {}).into(),
            MenuItem::Divider,
            Menu::new("More", (button("Duplicate").action(|| {}),)).into(),
        ];
        let items = items.into_menu_items().snapshot();
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], MenuItem::Command(_)));
        assert!(matches!(items[1], MenuItem::Divider));
        assert!(matches!(items[2], MenuItem::Menu(_)));
    }

    #[test]
    fn resolve_menu_items_carries_role_and_subtitle() {
        crate::init_test_executor();
        let env = Environment::default();
        let items = vec![
            Command::builder("Reply").action(|| {}).into(),
            Command::builder("Delete")
                .action(|| {})
                .destructive()
                .subtitle("For everyone")
                .into(),
        ];

        let resolved = resolve_menu_items_now::<NoopSlot>(items, &env);
        let ResolvedMenuItem::Command(reply) = &resolved[0] else {
            panic!("first resolved item should be a command");
        };
        assert_eq!(reply.role, None);
        assert!(reply.subtitle.is_none());

        let ResolvedMenuItem::Command(delete) = &resolved[1] else {
            panic!("second resolved item should be a command");
        };
        assert_eq!(delete.role, Some(ButtonRole::Destructive));
        assert_eq!(
            delete.subtitle.as_ref().map(Signal::snapshot).as_deref(),
            Some("For everyone")
        );
    }

    #[test]
    fn command_subtitle_resolves_through_translation_catalog() {
        use waterui_locale::{TranslationCatalog, locales};

        crate::init_test_executor();
        let mut env = Environment::default();
        env.insert(locales::FR);
        env.insert(
            TranslationCatalog::new()
                .add_toml(locales::FR, "\"For everyone\" = \"Pour tous\"")
                .expect("test catalog must parse"),
        );

        let resolved = Command::builder("Delete")
            .action(|| {})
            .subtitle("For everyone")
            .resolve::<NoopSlot>(&env);

        assert_eq!(
            resolved.subtitle.as_ref().map(Signal::snapshot).as_deref(),
            Some("Pour tous")
        );
    }

    #[test]
    fn resolve_menu_items_preserves_nested_labels_and_selected_state() {
        crate::init_test_executor();
        let env = Environment::default();
        let items = vec![
            Command::builder("Refresh").action(|| {}).into(),
            Command::builder("Pinned")
                .action(|| {})
                .selected(Computed::constant(true))
                .into(),
            Menu::new("Advanced", (button("Archive").action(|| {}),)).into(),
        ];

        let resolved = resolve_menu_items_now::<NoopSlot>(items, &env);
        assert_eq!(resolved.len(), 3);

        let ResolvedMenuItem::Command(refresh) = &resolved[0] else {
            panic!("first resolved item should be a command");
        };
        assert_eq!(refresh.label.content().snapshot().to_plain(), "Refresh");
        assert!(!refresh.selected.snapshot());

        let ResolvedMenuItem::Command(pinned) = &resolved[1] else {
            panic!("second resolved item should be a command");
        };
        assert_eq!(pinned.label.content().snapshot().to_plain(), "Pinned");
        assert!(pinned.selected.snapshot());

        let ResolvedMenuItem::Menu(advanced) = &resolved[2] else {
            panic!("third resolved item should be a nested menu");
        };
        assert_eq!(advanced.label.content().snapshot().to_plain(), "Advanced");
        assert_eq!(advanced.items.snapshot().len(), 1);

        let ResolvedMenuItem::Command(archive) = &advanced.items.snapshot()[0] else {
            panic!("nested menu should resolve its child command");
        };
        assert_eq!(archive.label.content().snapshot().to_plain(), "Archive");
    }

    /// A leaf `Computed<Vec<MenuItem>>` that counts how often it is evaluated.
    #[derive(Clone)]
    struct CountedItems {
        source: nami::Binding<Vec<MenuItem>>,
        evaluations: Rc<Cell<usize>>,
    }

    impl Signal for CountedItems {
        type Output = Vec<MenuItem>;
        type Guard = nami::watcher::BoxWatcherGuard;

        fn snapshot(&self) -> Vec<MenuItem> {
            self.evaluations.set(self.evaluations.get() + 1);
            self.source.snapshot()
        }

        fn watch(
            &self,
            watcher: impl Fn(nami::watcher::Context<Vec<MenuItem>>) + 'static,
        ) -> Self::Guard {
            self.source.watch(watcher)
        }
    }

    #[test]
    fn tuple_menu_reads_each_child_once_per_change() {
        const N: usize = 14;
        crate::init_test_executor();
        let evaluations = Rc::new(Cell::new(0usize));
        let sources: [nami::Binding<Vec<MenuItem>>; N] =
            core::array::from_fn(|_| nami::binding(vec![MenuItem::Divider]));
        let leaf = |source: &nami::Binding<Vec<MenuItem>>| {
            Computed::new(CountedItems {
                source: source.clone(),
                evaluations: evaluations.clone(),
            })
        };
        let items = (
            leaf(&sources[0]),
            leaf(&sources[1]),
            leaf(&sources[2]),
            leaf(&sources[3]),
            leaf(&sources[4]),
            leaf(&sources[5]),
            leaf(&sources[6]),
            leaf(&sources[7]),
            leaf(&sources[8]),
            leaf(&sources[9]),
            leaf(&sources[10]),
            leaf(&sources[11]),
            leaf(&sources[12]),
            leaf(&sources[13]),
        )
            .into_menu_items();

        let _guard = items.watch(|_| {});
        assert!(
            evaluations.get() <= N,
            "subscribing evaluated the leaves {} times; evaluation must be linear in tuple arity",
            evaluations.get()
        );

        evaluations.set(0);
        sources[0].set(vec![MenuItem::Divider, MenuItem::Divider]);
        assert_eq!(
            evaluations.get(),
            N,
            "one leaf change must evaluate each of the {N} children exactly once"
        );
    }

    /// The Close Window chord the declared `items` leave: ⌘W while nothing
    /// binds it, ⇧⌘W once a declared command takes it, and no chord at all
    /// when the application binds both — the deciding command counts at any
    /// depth.
    #[test]
    fn close_window_chord_yields_to_a_declared_binding() {
        crate::init_test_executor();
        let env = Environment::default();
        let command = |label, shortcut: Shortcut| -> MenuItem {
            Command::builder(label)
                .action(|| {})
                .shortcut(shortcut)
                .into()
        };

        assert_eq!(
            ResolvedMenuItem::close_window_chord(&resolve_menu_items_now::<NoopSlot>(
                vec![MenuItem::CloseWindow],
                &env
            )),
            Some(Shortcut::new('w').command()),
            "⌘W is Close Window's while no command binds it"
        );

        let bound = resolve_menu_items_now::<NoopSlot>(
            vec![
                Menu::new(
                    "File",
                    (command("Close Tab", Shortcut::new('w').command()),),
                )
                .into(),
                MenuItem::CloseWindow,
            ],
            &env,
        );
        assert_eq!(
            ResolvedMenuItem::close_window_chord(&bound),
            Some(Shortcut::new('w').command().shift()),
            "a declared ⌘W at any depth moves Close Window to ⇧⌘W"
        );

        let both = resolve_menu_items_now::<NoopSlot>(
            vec![
                command("Close Tab", Shortcut::new('w').command()),
                command("Close All Tabs", Shortcut::new('w').command().shift()),
                MenuItem::CloseWindow,
            ],
            &env,
        );
        assert_eq!(
            ResolvedMenuItem::close_window_chord(&both),
            None,
            "with both chords bound, Close Window carries none"
        );
    }

    #[test]
    fn close_window_resolves_and_is_found_at_any_depth() {
        crate::init_test_executor();
        let env = Environment::default();
        let top_level = resolve_menu_items_now::<NoopSlot>(vec![MenuItem::CloseWindow], &env);
        assert!(matches!(top_level[..], [ResolvedMenuItem::CloseWindow]));
        assert!(ResolvedMenuItem::declares_close_window(&top_level));

        let nested = resolve_menu_items_now::<NoopSlot>(
            vec![
                Menu::new("File", (button("Open").action(|| {}),)).into(),
                Menu::new(
                    "Window",
                    (Menu::new("Arrange", vec![MenuItem::CloseWindow]),),
                )
                .into(),
            ],
            &env,
        );
        assert!(ResolvedMenuItem::declares_close_window(&nested));

        let without = resolve_menu_items_now::<NoopSlot>(
            vec![
                MenuItem::Quit,
                Menu::new("File", (button("Open").action(|| {}),)).into(),
            ],
            &env,
        );
        assert!(!ResolvedMenuItem::declares_close_window(&without));
    }

    #[test]
    fn resolved_command_action_uses_captured_state() {
        crate::init_test_executor();
        let count = Rc::new(Cell::new(0));
        let command = Command::builder("Increment")
            .action(|State(count): State<Rc<Cell<i32>>>| count.set(count.get() + 1))
            .state(&count);
        let resolved = command.resolve::<NoopSlot>(&Environment::default());

        resolved.action.run(&Environment::default());
        resolved.action.run(&Environment::default());

        assert_eq!(count.get(), 2);
    }
}
