//! Context menu metadata.

use crate::menu::{MenuItem, MenuView, ResolvedMenuItem, resolve_menu_items};
use alloc::vec::Vec;
use core::fmt;
use nami::{Binding, Computed, SignalExt, binding};
use waterui_core::{
    AnyView, Environment, Metadata, View,
    backend::{Backend, ChildSlot, MapSlots},
    env::with,
    extract::State,
    metadata::MetadataKey,
};

/// A context menu with an optional custom preview and an interactive
/// accessory presented alongside its commands.
///
/// The menu appears when the user long-presses (iOS, Android, touch and
/// pen elsewhere) or secondary-clicks (macOS, desktop pointers). Menu rows
/// stay within what every platform's own menu can render; custom content
/// belongs in the [`accessory`](Self::accessory). Placement is
/// platform-specific: macOS presents the accessory as a native
/// `NSMenuItem` view inside the menu. On iOS, menus with an accessory
/// present the custom preview, accessory and commands together in a
/// native popover; menus without an accessory use the system context
/// menu and lifted preview.
///
/// The two placements are not interchangeable capabilities. In a
/// tracked `NSMenu` the accessory is pointer-interactive only — text
/// entry and other keyboard input are unsupported — its row
/// dimensions are fixed while the menu tracks, and a changed
/// intrinsic size is applied on the next open. `Binding`-driven
/// updates inside the accessory still render in place while it is
/// open. On iOS, the accessory remains interactive and its intrinsic
/// size can change while the popover is open.
///
/// `P` and `A` are the view types of the preview and accessory, `()` while
/// none is set.
///
/// # Example
///
/// ```rust
/// use waterui::prelude::*;
///
/// let message = text!("Hello")
///     .context_menu(
///         ContextMenu::new((
///             "Reply".action(|| {}),
///             "Delete".command().action(|| {}).destructive(),
///         ))
///         .accessory(
///             button("Like").action(|dismiss: DismissContextMenu| dismiss.dismiss()),
///         ),
///     );
/// ```
#[non_exhaustive]
pub struct ContextMenu<P, A> {
    /// The menu items to display in the context menu.
    items: Computed<Vec<MenuItem>>,
    /// The custom preview shown while the menu is open. `None` uses
    /// the platform's default presentation of the source view.
    preview: Option<P>,
    /// An interactive view presented alongside the menu — a
    /// pointer-interactive `NSMenuItem` view row on macOS, alongside
    /// the preview and commands in a native popover on iOS.
    accessory: Option<A>,
}

impl<P, A> fmt::Debug for ContextMenu<P, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextMenu")
            .field("items", &self.items)
            .field("preview", &self.preview.is_some())
            .field("accessory", &self.accessory.is_some())
            .finish_non_exhaustive()
    }
}

impl<P, A> ContextMenu<P, A> {
    /// The menu items to display in the context menu.
    #[must_use]
    pub const fn items(&self) -> &Computed<Vec<MenuItem>> {
        &self.items
    }

    /// The custom preview shown while the menu is open. `None` uses
    /// the platform's default presentation of the source view.
    #[must_use]
    pub const fn get_preview(&self) -> Option<&P> {
        self.preview.as_ref()
    }

    /// An interactive view presented alongside the menu — a
    /// pointer-interactive `NSMenuItem` view row on macOS, alongside
    /// the preview and commands in a native popover on iOS.
    #[must_use]
    pub const fn get_accessory(&self) -> Option<&A> {
        self.accessory.as_ref()
    }
}

impl<P: 'static, A: 'static> MetadataKey for ContextMenu<P, A> {}

impl ContextMenu<(), ()> {
    /// Creates a context menu with the given items, lifting the source
    /// view and showing no accessory.
    #[must_use]
    pub fn new(items: impl MenuView) -> Self {
        Self {
            items: items.into_menu_items(),
            preview: None,
            accessory: None,
        }
    }
}

impl<P, A> ContextMenu<P, A> {
    /// Shows `preview` while the menu is open, using the platform's
    /// presentation: a lifted preview for an iOS system context menu,
    /// or content inside its accessory-bearing popover.
    #[must_use]
    pub fn preview<NewP: View>(self, preview: NewP) -> ContextMenu<NewP, A> {
        ContextMenu {
            items: self.items,
            preview: Some(preview),
            accessory: self.accessory,
        }
    }

    /// Presents an interactive `accessory` alongside the menu — as a
    /// native row inside the `NSMenu` on macOS, alongside the preview
    /// and commands in a native popover on iOS.
    ///
    /// Inside a tracked `NSMenu` the accessory is pointer-interactive
    /// only: keyboard input is unsupported, and its dimensions are
    /// fixed until the menu reopens.
    ///
    /// Choosing a menu item dismisses the menu; acting inside the
    /// accessory does not. The accessory's environment carries a
    /// [`DismissContextMenu`] to call when an action should close the
    /// menu.
    #[must_use]
    pub fn accessory<NewA: View>(self, accessory: NewA) -> ContextMenu<P, NewA> {
        ContextMenu {
            items: self.items,
            preview: self.preview,
            accessory: Some(accessory),
        }
    }
}

impl<P: View, A: View> ContextMenu<P, A> {
    fn resolve<S: ChildSlot>(self, env: &Environment) -> ResolvedContextMenu<S> {
        let Self {
            items,
            preview,
            accessory,
        } = self;
        let requests = binding(0_i32);
        let dismiss = DismissContextMenu {
            requests: requests.clone(),
        };
        ResolvedContextMenu {
            items: resolve_menu_items(&items, env),
            preview: preview.map(S::from_view),
            accessory: accessory.map(|accessory| S::from_view(with(accessory, State(dismiss)))),
            dismiss_requests: requests.computed(),
        }
    }
}

/// Converts a value into a [`ContextMenu`]: either a full `ContextMenu`
/// or any [`MenuView`], which becomes a menu with no preview or
/// accessory.
pub trait IntoContextMenu {
    /// The preview view type, `()` when the menu carries none.
    type Preview: View;
    /// The accessory view type, `()` when the menu carries none.
    type Accessory: View;

    /// Performs the conversion.
    fn into_context_menu(self) -> ContextMenu<Self::Preview, Self::Accessory>;
}

impl<P: View, A: View> IntoContextMenu for ContextMenu<P, A> {
    type Preview = P;
    type Accessory = A;

    fn into_context_menu(self) -> Self {
        self
    }
}

impl<T: MenuView> IntoContextMenu for T {
    type Preview = ();
    type Accessory = ();

    fn into_context_menu(self) -> ContextMenu<(), ()> {
        ContextMenu::new(self)
    }
}

/// Closes the context menu whose accessory is being acted on.
///
/// Present in the environment of a context menu's
/// [`accessory`](ContextMenu::accessory). Extract it in a handler directly
/// as `dismiss: DismissContextMenu`.
#[waterui_macros::state]
#[derive(Debug, Clone)]
pub struct DismissContextMenu {
    requests: Binding<i32>,
}

impl DismissContextMenu {
    /// Asks the backend to close the context menu.
    pub fn dismiss(&self) {
        self.requests
            .with_mut(|requests| *requests = requests.wrapping_add(1));
    }
}

/// Resolved context menu metadata consumed by the native backends, with
/// its preview and accessory in the backend's child slot `S`.
#[doc(hidden)]
#[derive(Debug)]
pub struct ResolvedContextMenu<S> {
    /// The resolved menu items for the current environment.
    pub items: Computed<Vec<ResolvedMenuItem<S>>>,
    /// The custom preview shown while the menu is open; `None` uses
    /// the platform's default presentation of the source view.
    pub preview: Option<S>,
    /// The accessory presented alongside the menu, with
    /// [`DismissContextMenu`] installed in its environment.
    pub accessory: Option<S>,
    /// Counts the accessory's dismiss requests, wrapping on overflow.
    /// Only a change matters: every change asks the backend to close the
    /// open menu.
    pub dismiss_requests: Computed<i32>,
}

impl<S: 'static> MetadataKey for ResolvedContextMenu<S> {}

impl<S: 'static> ResolvedContextMenu<S> {
    /// The same resolved menu with its items, preview and accessory mapped
    /// through `f`: a backend holds them in its own slot.
    #[must_use]
    pub fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> ResolvedContextMenu<T> {
        let item_f = f.clone();
        ResolvedContextMenu {
            items: self
                .items
                .map(move |items| {
                    items
                        .into_iter()
                        .map(|item| item.map_slots(item_f.clone()))
                        .collect()
                })
                .computed(),
            preview: self.preview.map(f.clone()),
            accessory: self.accessory.map(f),
            dismiss_requests: self.dismiss_requests,
        }
    }
}

impl<S: 'static> MapSlots<S> for ResolvedContextMenu<S> {
    type With<T: 'static> = ResolvedContextMenu<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> ResolvedContextMenu<T> {
        Self::map_slots(self, f)
    }
}

/// View wrapper that resolves a context menu against the environment.
#[doc(hidden)]
#[non_exhaustive]
pub struct ContextMenuView<Content, P, A> {
    /// The wrapped content view.
    content: Content,
    /// The context menu to resolve.
    menu: ContextMenu<P, A>,
}

impl<Content, P, A> fmt::Debug for ContextMenuView<Content, P, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextMenuView")
            .field("menu", &self.menu)
            .finish_non_exhaustive()
    }
}

impl<Content, P, A> ContextMenuView<Content, P, A> {
    /// Wraps `content` with the `menu` to resolve against the environment.
    pub const fn new(content: Content, menu: ContextMenu<P, A>) -> Self {
        Self { content, menu }
    }

    /// The wrapped content view.
    #[must_use]
    pub const fn content(&self) -> &Content {
        &self.content
    }

    /// The context menu to resolve.
    #[must_use]
    pub const fn menu(&self) -> &ContextMenu<P, A> {
        &self.menu
    }
}

impl<Content: View, P: View, A: View> View for ContextMenuView<Content, P, A> {
    /// The erased form, for code that expands views by hand.
    fn body(self, env: &Environment) -> impl View {
        Metadata::new(self.content, self.menu.resolve::<AnyView>(env))
    }

    /// Reaches the backend with the content, preview, and accessory in the
    /// backend's child slot.
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.metadata(
            Metadata::new(
                B::Slot::from_view(self.content),
                self.menu.resolve::<B::Slot>(env),
            ),
            env,
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::menu::CommandExt;
    use core::any::TypeId;
    use nami::Signal;
    use waterui_core::{
        AnyView, Environment, Metadata, View,
        backend::{Backend, ChildSlot, peel_metadata},
        env::With,
        extract::State,
        metadata::MetadataKey,
        view::hook::{HookList, HookableView},
    };
    use waterui_text::text::{Text, text};

    use super::{ContextMenu, ContextMenuView, DismissContextMenu, ResolvedContextMenu};

    /// Resolving `.context_menu` attaches `Metadata<ResolvedContextMenu>`
    /// with the accessory kept as a `With` view whose extended environment
    /// is itself attached as `Metadata<Environment>`.
    #[test]
    fn accessory_environment_carries_dismiss_context_menu() {
        let env = Environment::new();
        let resolved = peel_metadata::<ResolvedContextMenu<AnyView>>(
            ContextMenuView::new(
                text("target"),
                ContextMenu::new("Reply".action(|| {})).accessory(text("like")),
            ),
            &env,
        )
        .expect("context_menu attaches ResolvedContextMenu")
        .value;

        let accessory_env = peel_metadata::<Environment>(
            resolved
                .accessory
                .expect("resolved menu keeps the accessory"),
            &env,
        )
        .expect("the accessory carries an extended environment")
        .value;

        let dismiss = accessory_env
            .extract::<DismissContextMenu>()
            .expect("the accessory environment carries DismissContextMenu");

        let before = resolved.dismiss_requests.snapshot();
        dismiss.dismiss();
        assert_ne!(resolved.dismiss_requests.snapshot(), before);
    }

    /// Records the concrete view type handed to `ChildSlot::from_view`.
    #[derive(Debug, PartialEq, Eq)]
    struct RecordedSlot(TypeId);

    impl ChildSlot for RecordedSlot {
        fn from_view<V: View>(_view: V) -> Self {
            Self(TypeId::of::<V>())
        }
    }

    struct SlotProbe;

    impl Backend for SlotProbe {
        type Output = Option<ResolvedContextMenu<RecordedSlot>>;
        type Ctx = ();
        type Slot = RecordedSlot;

        fn native<T: waterui_core::NativeView + 'static>(
            &mut self,
            _native: T,
            _env: &Environment,
            (): (),
        ) -> Self::Output {
            None
        }

        fn metadata<K: MetadataKey>(
            &mut self,
            metadata: Metadata<K, Self::Slot>,
            _env: &Environment,
            (): (),
        ) -> Self::Output {
            waterui_core::backend::cast(metadata.value).ok()
        }

        fn erased(&mut self, _view: AnyView, _env: &Environment, (): ()) -> Self::Output {
            None
        }

        fn scoped<H: HookList, V: View>(
            &mut self,
            _hooks: H,
            view: V,
            env: &Environment,
            (): (),
        ) -> Self::Output {
            view.dispatch(self, env, ())
        }

        fn hookable<T: HookableView>(
            &mut self,
            _view: T,
            _env: &Environment,
            (): (),
        ) -> Self::Output {
            None
        }
    }

    #[test]
    fn dispatch_keeps_preview_and_accessory_types_in_backend_slot() {
        let menu = ContextMenu::new("Reply".action(|| {}))
            .preview(text("preview"))
            .accessory(());
        let _: Option<&Text> = menu.get_preview();
        let _: Option<&()> = menu.get_accessory();

        let env = Environment::new();
        let resolved = ContextMenuView::new(text("target"), menu)
            .dispatch(&mut SlotProbe, &env, ())
            .expect("context_menu dispatches ResolvedContextMenu<B::Slot>");

        assert_eq!(resolved.preview, Some(RecordedSlot(TypeId::of::<Text>())));
        assert_ne!(
            resolved.preview,
            Some(RecordedSlot(TypeId::of::<AnyView>()))
        );
        let accessory_id = resolved.accessory.expect("accessory present").0;
        assert_ne!(accessory_id, TypeId::of::<AnyView>());
        assert_eq!(
            accessory_id,
            TypeId::of::<With<(), State<DismissContextMenu>>>()
        );
    }
}
