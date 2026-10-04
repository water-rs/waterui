//! The `context_menu` metadata: `Metadata<ResolvedContextMenu>` wrapped
//! around a child.
//!
//! Mirrors `WuiContextMenu`. On `AppKit` `rightMouseDown` pops up an
//! `NSMenu` whose top row is a custom-view item carrying the accessory —
//! `AppKit` gives a menu item's view mouse events, so interactive
//! children work inside the tracking loop. On `UIKit` the realization is
//! selected by the `accessory` attribute: a menu without one installs a
//! canonical `UIContextMenuInteraction`; a menu with one presents a
//! popover panel that mounts the preview, the accessory and the command
//! rows in its own hierarchy — the canonical interaction offers no
//! interactive accessory surface. The `items` collection is snapshotted
//! per presentation; `dismiss_requests` counts up to a close.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::Rect;
use cocoa_ui::view;
use waterui::component::menu::ResolvedMenuItem;
use waterui::metadata::context_menu::ResolvedContextMenu;
use waterui::reactive::Computed;
use waterui::reactive::Signal;
#[cfg(target_os = "ios")]
use waterui::reactive::SignalExt;
#[cfg(target_os = "ios")]
use waterui::resolve::Resolvable;
use waterui_backend_core::Environment;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::components::menu_items;
use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{self as appkit, HostView};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{self as uikit, HostView};

/// The leaf's live state.
struct ContextMenuState {
    /// The mounted content.
    child: Mounted,
    /// The environment command actions run against.
    env: Environment,
    /// The resolved items, snapshotted per menu presentation.
    items: Computed<Vec<ResolvedMenuItem>>,
    /// The rendered `preview` and its one view controller (`UIKit` only —
    /// `AppKit` has no preview slot and drops it unresolved).
    #[cfg(target_os = "ios")]
    preview: Option<IosPreview>,
    /// The rendered `accessory` leaf; mounted into the presented surface
    /// while the menu is open.
    accessory: Option<NativeLeaf>,
    /// The open tracking session, so a dismiss request can cancel it —
    /// `openMenu`. `Rc` so `pop_up` runs without a state borrow held
    /// (`menuDidClose` borrows it mid-call).
    #[cfg(target_os = "macos")]
    open_menu: RefCell<Option<Rc<appkit::ContextMenu>>>,
    /// The presented popover panel and the leaves mounted into it —
    /// `UIKit` accessory menus only.
    #[cfg(target_os = "ios")]
    presented: RefCell<Option<PanelSession>>,
    /// The installed interaction — `contextMenuInteraction`; no-accessory
    /// menus only.
    #[cfg(target_os = "ios")]
    interaction: RefCell<Option<uikit::ContextMenu>>,
    /// The recognizers that open the panel — accessory menus only.
    #[cfg(target_os = "ios")]
    triggers: RefCell<Vec<uikit::gesture::GestureAttachment>>,
    /// Main-thread proof for menu work.
    mtm: cocoa_ui::MainThreadMarker,
}

impl core::fmt::Debug for ContextMenuState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ContextMenuState").finish_non_exhaustive()
    }
}

#[cfg(target_os = "ios")]
impl Drop for ContextMenuState {
    /// The leaf is gone: any panel it presented closes, and the mounted
    /// leaves the session held detach and drop with the session — the
    /// weak `on_dismiss` finds no state to restore them into, so no
    /// teardown can run twice.
    fn drop(&mut self) {
        if let Some(session) = self.presented.borrow_mut().take() {
            session.popover.dismiss();
        }
    }
}

/// The wrapper's layout face: the content's answers verbatim.
struct ContextMenuSubView {
    /// The leaf's state.
    state: Rc<RefCell<ContextMenuState>>,
}

impl core::fmt::Debug for ContextMenuSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ContextMenuSubView").finish_non_exhaustive()
    }
}

impl SubView for ContextMenuSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.borrow().child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.borrow().child.layout().priority()
    }
}

/// A leaf's ideal size — `sizeThatFits(WuiProposalSize())`.
fn ideal_size(leaf: &NativeLeaf) -> cocoa_ui::Size {
    {
        let m = leaf.layout().measure(ProposalSize::default()).size;
        cocoa_ui::Size::new(f64::from(m.width), f64::from(m.height))
    }
}

/// `dismissPresentedMenu`: close whatever is open. The platform teardown
/// paths restore the leaves, so teardown is idempotent.
///
/// Every handle is moved or cloned out before its native call: a
/// programmatic dismissal's completion and an `AppKit` `menuDidClose` may
/// reach back into the state synchronously, and they must not find a
/// borrow — shared or mutable — still held.
fn dismiss_presented(state: &Rc<RefCell<ContextMenuState>>) {
    #[cfg(target_os = "ios")]
    {
        let (popover, interaction) = {
            let state = state.borrow();
            (
                state
                    .presented
                    .borrow()
                    .as_ref()
                    .map(|session| session.popover.clone()),
                state.interaction.borrow().clone(),
            )
        };
        if let Some(popover) = popover {
            popover.dismiss();
        }
        if let Some(interaction) = interaction {
            interaction.dismiss();
        }
    }
    #[cfg(target_os = "macos")]
    {
        let menu = state.borrow().open_menu.borrow_mut().take();
        if let Some(menu) = menu {
            menu.cancel();
        }
    }
}

/// The panel's chrome colors, resolved from the menu's theme tokens.
/// Each palette maps `Foreground`, `MutedForeground`, `Error`, `Border`,
/// `SelectionContainer` and `Surface` onto the panel's row and surface
/// slots; `bind_panel_palette` pushes a new one whenever a token changes.
#[cfg(target_os = "ios")]
fn menu_panel_palette(env: &Environment) -> uikit::PanelPalette {
    use waterui::theme::color::{
        Border, Error, Foreground, MutedForeground, SelectionContainer, Surface,
    };
    uikit::PanelPalette {
        label: platform_color(&Foreground.resolve(env).snapshot()),
        muted: platform_color(&MutedForeground.resolve(env).snapshot()),
        destructive: platform_color(&Error.resolve(env).snapshot()),
        separator: platform_color(&Border.resolve(env).snapshot()),
        focus_fill: platform_color(&SelectionContainer.resolve(env).snapshot()),
        surface: platform_color(&Surface.resolve(env).snapshot()),
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color
/// object.
#[cfg(target_os = "ios")]
fn platform_color(
    color: &waterui::graphics::color::WorkingColor,
) -> cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIColor> {
    let [red, green, blue, alpha] = color.components;
    cocoa_ui::uikit::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// Repaints an open panel when any theme token it draws with changes —
/// the same per-token granularity the rest of the backend uses.
#[cfg(target_os = "ios")]
fn bind_panel_palette(
    leaf: &mut NativeLeaf,
    env: &Environment,
    state: &Rc<RefCell<ContextMenuState>>,
) {
    use waterui::theme::color::{
        Border, Error, Foreground, MutedForeground, SelectionContainer, Surface,
    };
    for token in [
        Foreground.resolve(env).computed(),
        MutedForeground.resolve(env).computed(),
        Error.resolve(env).computed(),
        Border.resolve(env).computed(),
        SelectionContainer.resolve(env).computed(),
        Surface.resolve(env).computed(),
    ] {
        leaf.bind(&token, {
            let env = env.clone();
            let state = Rc::downgrade(state);
            move |_| {
                let Some(state) = state.upgrade() else {
                    return;
                };
                let popover = state
                    .borrow()
                    .presented
                    .borrow()
                    .as_ref()
                    .map(|session| session.popover.clone());
                if let Some(popover) = popover {
                    popover.apply_palette(&menu_panel_palette(&env));
                }
            }
        });
    }
}

/// A mounted custom preview: the leaf and, for the canonical interaction
/// only, the one view controller that owns its view while presented. A
/// view may be associated with a single view controller at a time; a
/// preview that mounts into the popover panel carries no controller — the
/// panel's hierarchy owns it directly.
#[cfg(target_os = "ios")]
struct IosPreview {
    leaf: NativeLeaf,
    controller: Option<cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIViewController>>,
}

/// An open popover panel: its handle and the leaves mounted into it while
/// it presents. `Mounted` detaches on unmount, which is how the leaves
/// return to the state for the next open.
#[cfg(target_os = "ios")]
struct PanelSession {
    popover: uikit::ContextMenuPopover,
    preview_mount: Option<(
        Mounted,
        Option<cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIViewController>>,
    )>,
    accessory_mount: Option<Mounted>,
}

/// Opens the accessory menu's popover anchored to `host`, moving the
/// preview and accessory leaves into its hierarchy. No-op while a panel
/// already presents.
#[cfg(target_os = "ios")]
fn present_panel(state: &Rc<RefCell<ContextMenuState>>, host: &cocoa_ui::PlatformView) {
    if state.borrow().presented.borrow().is_some() {
        return;
    }
    let popover = uikit::ContextMenuPopover::new(
        state.borrow().mtm,
        &menu_panel_palette(&state.borrow().env),
    );
    let mount_target = popover.mount_target();
    let nodes = {
        let state = state.borrow();
        menu_items::tree_nodes(&state.items.snapshot(), &state.env)
    };
    popover.set_commands(&nodes);
    // The panel is reachable from the state through the session; the
    // callback reaches back weakly so the state can out-live neither — a
    // dead state means the leaf was torn down mid-presentation.
    let on_dismiss: Rc<dyn Fn()> = {
        let state = Rc::downgrade(state);
        Rc::new(move || {
            if let Some(state) = state.upgrade() {
                teardown_panel(&state);
            }
        })
    };
    let mut state = state.borrow_mut();
    let preview_mount = state.preview.take().map(|IosPreview { leaf, controller }| {
        let mounted = leaf.mount(&mount_target);
        popover.set_slot(uikit::PanelSlot::Preview, mounted.view());
        (mounted, controller)
    });
    let accessory_mount = state.accessory.take().map(|leaf| {
        let mounted = leaf.mount(&mount_target);
        popover.set_slot(uikit::PanelSlot::Accessory, mounted.view());
        mounted
    });
    let presented = popover.present(host, on_dismiss);
    if presented {
        *state.presented.borrow_mut() = Some(PanelSession {
            popover,
            preview_mount,
            accessory_mount,
        });
    } else {
        // No presentation could be made: the leaves come home unused.
        restore_session(
            &mut state,
            PanelSession {
                popover,
                preview_mount,
                accessory_mount,
            },
        );
    }
}

/// The presentation ended — outside tap, a picked command, `dismiss` or
/// Escape — through `presentationControllerDidDismiss`. The mounted
/// leaves return to the state and the panel releases.
#[cfg(target_os = "ios")]
fn teardown_panel(state: &Rc<RefCell<ContextMenuState>>) {
    let session = state.borrow().presented.borrow_mut().take();
    let Some(session) = session else {
        return;
    };
    restore_session(&mut state.borrow_mut(), session);
}

/// Moves a session's leaves back into the state's slots.
#[cfg(target_os = "ios")]
fn restore_session(state: &mut ContextMenuState, session: PanelSession) {
    let PanelSession {
        popover: _,
        preview_mount,
        accessory_mount,
    } = session;
    if let Some((mounted, controller)) = preview_mount {
        state.preview = Some(IosPreview {
            leaf: mounted.unmount(),
            controller,
        });
    }
    if let Some(mounted) = accessory_mount {
        state.accessory = Some(mounted.unmount());
    }
}

/// `setAccessory` (`AppKit`): inserts the accessory leaf as the menu's
/// top row, a custom-view item sized to its ideal measure. The row's
/// size is fixed for the tracking session — `AppKit` does not resize
/// item views mid-track.
#[cfg(target_os = "macos")]
fn set_accessory_item(state: &Rc<RefCell<ContextMenuState>>, menu: &appkit::ContextMenu) {
    let state = state.borrow();
    let Some(accessory) = &state.accessory else {
        return;
    };
    let accessory_view = view::retain_base(accessory.view());
    menu.set_accessory(&accessory_view, ideal_size(accessory));
}

/// Installs the `context_menu` handler on the dispatcher.
#[expect(
    clippy::too_many_lines,
    reason = "the handler's wiring is sequential; splitting it would obscure the mount order"
)]
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<ResolvedContextMenu>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        // `preview` is a `UIKit` primitive; on `AppKit` the `AnyView`
        // drops unresolved.
        let accessory = metadata.value.accessory.map(|view| ctx.render(view));
        #[cfg(target_os = "ios")]
        let preview = metadata.value.preview.map(|view| {
            let leaf = ctx.render(view);
            // An accessory-bearing menu mounts the leaf into its own
            // panel; the canonical interaction alone takes a controller,
            // and only one controller may own the view at a time.
            let controller = if accessory.is_some() {
                None
            } else {
                Some(uikit::preview_controller(
                    mtm,
                    view::retain_base(leaf.view()),
                    ideal_size(&leaf),
                ))
            };
            IosPreview { leaf, controller }
        });
        #[cfg(target_os = "macos")]
        drop(metadata.value.preview);

        let state = Rc::new(RefCell::new(ContextMenuState {
            child: mounted,
            env: ctx.env().clone(),
            items: metadata.value.items.clone(),
            #[cfg(target_os = "ios")]
            preview,
            accessory,
            #[cfg(target_os = "macos")]
            open_menu: RefCell::new(None),
            #[cfg(target_os = "ios")]
            presented: RefCell::new(None),
            #[cfg(target_os = "ios")]
            interaction: RefCell::new(None),
            #[cfg(target_os = "ios")]
            triggers: RefCell::new(Vec::new()),
            mtm,
        }));

        // The content always fills the wrapper.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                let state = state.borrow();
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });

        // `setPlacementProposal` forwards to the content.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            move |selected| {
                let state = state.borrow();
                proposal::deliver(state.child.view(), selected);
            }
        });

        #[cfg(target_os = "ios")]
        {
            view::set_user_interaction_enabled(&host, true);
            let has_accessory = state.borrow().accessory.is_some();
            if has_accessory {
                // An accessory-bearing menu presents the popover panel: a
                // long press — or a secondary click on pointer devices —
                // opens it, like the canonical interaction. The closures
                // reach state and host weakly: the recognizer targets are
                // owned by `state.triggers`, so a strong capture would be
                // the state retaining itself.
                let present = {
                    let state = Rc::downgrade(&state);
                    let host = objc2::rc::Weak::new(&*host);
                    Rc::new(move || {
                        if let (Some(state), Some(host)) = (state.upgrade(), host.load()) {
                            present_panel(&state, &host);
                        }
                    })
                };
                state.borrow_mut().triggers.borrow_mut().extend([
                    uikit::gesture::long_press(
                        &host,
                        0.5,
                        cocoa_ui::gesture::ButtonMask::PRIMARY,
                        {
                            let present = Rc::clone(&present);
                            move |gesture| {
                                if matches!(gesture, cocoa_ui::gesture::GestureState::Began) {
                                    present();
                                }
                            }
                        },
                    ),
                    uikit::gesture::tap(
                        &host,
                        1,
                        cocoa_ui::gesture::ButtonMask::SECONDARY,
                        move |gesture| {
                            if matches!(gesture, cocoa_ui::gesture::GestureState::Ended) {
                                present();
                            }
                        },
                    ),
                ]);
            } else {
                *state.borrow_mut().interaction.borrow_mut() = Some(uikit::ContextMenu::install(
                    &host,
                    uikit::ContextMenuHandlers {
                        configuration: Rc::new({
                            let state = Rc::clone(&state);
                            move |_| {
                                let state = state.borrow();
                                let nodes =
                                    menu_items::tree_nodes(&state.items.snapshot(), &state.env);
                                if nodes.is_empty() {
                                    return None;
                                }
                                let menu = uikit::menu(
                                    state.mtm,
                                    &cocoa_ui::menu::Command::default(),
                                    &nodes,
                                );
                                let preview = state.preview.as_ref().and_then(|preview| {
                                    preview.controller.as_ref().map(|controller| {
                                        controller.setPreferredContentSize(
                                            ideal_size(&preview.leaf).into(),
                                        );
                                        controller.clone()
                                    })
                                });
                                Some(uikit::ContextMenuConfiguration { menu, preview })
                            }
                        }),
                        preview: Rc::new({
                            let host = host.clone();
                            move |_| {
                                // `UITargetedPreview` requires its view to
                                // be in a window, so the highlight always
                                // lifts the source; a custom preview's
                                // card is the preview controller's
                                // content, not this view.
                                Some(uikit::targeted_preview(&host))
                            }
                        }),
                    },
                ));
            }
        }

        #[cfg(target_os = "macos")]
        host.set_right_mouse_handler({
            let state = Rc::clone(&state);
            move |host_view, event| {
                let menu = {
                    let mtm = state.borrow().mtm;
                    let nodes = {
                        let state = state.borrow();
                        menu_items::tree_nodes(&state.items.snapshot(), &state.env)
                    };
                    Rc::new(appkit::ContextMenu::new(mtm, &nodes, || {}, {
                        let state = Rc::clone(&state);
                        move || {
                            state.borrow().open_menu.borrow_mut().take();
                        }
                    }))
                };
                set_accessory_item(&state, &menu);
                state
                    .borrow()
                    .open_menu
                    .borrow_mut()
                    .replace(Rc::clone(&menu));
                // `popUpContextMenu` runs the tracking loop; `menuDidClose`
                // fires before it returns.
                menu.pop_up(host_view, event);
            }
        });

        let mut leaf = NativeLeaf::new(
            &*host,
            ContextMenuSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.watch(&metadata.value.dismiss_requests, {
            let state = Rc::clone(&state);
            move |_| dismiss_presented(&state)
        });
        #[cfg(target_os = "ios")]
        bind_panel_palette(&mut leaf, &ctx.env().clone(), &state);
        leaf.keep(state);
        leaf
    });
}
