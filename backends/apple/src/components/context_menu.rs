//! The `context_menu` metadata: `Metadata<ResolvedContextMenu>` wrapped
//! around a child.
//!
//! Mirrors `WuiContextMenu`. On `AppKit` `rightMouseDown` pops up an
//! `NSMenu` whose `menuWillOpen` shows the accessory in a floating
//! non-activating panel anchored above the source's screen frame, and
//! `menuDidClose` tears it down (deferred a tick, so the closing click
//! can still land on the accessory). On `UIKit` a
//! `UIContextMenuInteraction` builds the `UIMenu` per presentation,
//! lifts `preview` as the targeted preview, and shows the accessory in
//! an overlay window one level above the menu's — hits outside the
//! accessory fall through and read as dismiss taps. The `items`
//! collection is snapshotted per presentation; `dismiss_requests`
//! counts up to a close.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::Rect;
use cocoa_ui::view;
use waterui::component::menu::ResolvedMenuItem;
use waterui::metadata::context_menu::ResolvedContextMenu;
use waterui::reactive::Computed;
use waterui::reactive::Signal;
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
    /// The rendered `preview` leaf (`UIKit` only — `AppKit` has no
    /// preview slot and drops it unresolved). `Rc` so the overlay's
    /// re-measure closure can reach it without borrowing this state.
    #[cfg(target_os = "ios")]
    preview: Option<Rc<NativeLeaf>>,
    /// The rendered `accessory` leaf; the platform overlays mount its
    /// view while the menu is open.
    accessory: Option<Rc<NativeLeaf>>,
    /// The open tracking session, so a dismiss request can cancel it —
    /// `openMenu`. `Rc` so `pop_up` runs without a state borrow held
    /// (`menuDidClose` borrows it mid-call).
    #[cfg(target_os = "macos")]
    open_menu: RefCell<Option<Rc<appkit::ContextMenu>>>,
    /// The floating accessory panel — `accessoryPanel`.
    #[cfg(target_os = "macos")]
    panel: RefCell<Option<appkit::AccessoryPanel>>,
    /// The overlay showing the accessory — `accessoryWindow`.
    #[cfg(target_os = "ios")]
    overlay: RefCell<Option<uikit::AccessoryOverlay>>,
    /// The installed interaction — `contextMenuInteraction`; filled once
    /// the handlers closing over this state exist.
    #[cfg(target_os = "ios")]
    interaction: RefCell<Option<uikit::ContextMenu>>,
    /// Main-thread proof for menu work.
    mtm: cocoa_ui::MainThreadMarker,
}

impl core::fmt::Debug for ContextMenuState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ContextMenuState").finish_non_exhaustive()
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

/// Whether the view's bounds are empty — `bounds.isEmpty`.
#[cfg(target_os = "ios")]
fn bounds_is_empty(view: &cocoa_ui::PlatformView) -> bool {
    let bounds = view::bounds(view);
    bounds.size.width <= 0.0 || bounds.size.height <= 0.0
}

/// `dismissPresentedMenu`: close whatever is tracking and drop the
/// accessory; the platform teardown paths do the same, so teardown is
/// idempotent.
fn dismiss_presented(state: &Rc<RefCell<ContextMenuState>>) {
    let state = state.borrow();
    #[cfg(target_os = "ios")]
    {
        if let Some(interaction) = &*state.interaction.borrow() {
            interaction.dismiss();
        }
        if let Some(overlay) = state.overlay.borrow_mut().take() {
            overlay.dismiss();
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(menu) = state.open_menu.borrow_mut().take() {
            menu.cancel();
        }
        if let Some(panel) = state.panel.borrow_mut().take() {
            panel.order_out();
        }
    }
}

/// `targetedPreviewFrame`: the source view's frame in window coordinates,
/// or the frame a custom preview declares — centred on the source at the
/// preview's ideal size.
#[cfg(target_os = "ios")]
fn targeted_preview_frame(state: &ContextMenuState, host: &cocoa_ui::PlatformView) -> Rect {
    let source = uikit::bounds_in_window(host);
    let Some(preview) = &state.preview else {
        return source;
    };
    let size = ideal_size(preview);
    Rect::new(
        source.origin.x + source.size.width / 2.0 - size.width / 2.0,
        source.origin.y + source.size.height / 2.0 - size.height / 2.0,
        size.width,
        size.height,
    )
}

/// `presentAccessory` (`UIKit`): the overlay window one level above the
/// context menu's, carrying the accessory re-measured each pass.
#[cfg(target_os = "ios")]
fn present_accessory(state: &Rc<RefCell<ContextMenuState>>, host: &cocoa_ui::PlatformView) {
    let state = state.borrow();
    if state.overlay.borrow().is_some() {
        return;
    }
    let Some(accessory) = &state.accessory else {
        return;
    };
    let accessory_view = view::retain_base(accessory.view());
    let preview_frame = targeted_preview_frame(&state, host);
    // The platter re-asks each layout; the leaf measures under the
    // unbounded proposal.
    let sizing = Rc::clone(accessory);
    let overlay = uikit::AccessoryOverlay::present(host, &accessory_view, preview_frame, {
        move || ideal_size(&sizing)
    });
    if let Some(overlay) = overlay {
        *state.overlay.borrow_mut() = Some(overlay);
    }
}

/// `presentAccessoryPanel` (`AppKit`): the floating panel above the
/// source for the tracking session; the accessory re-measures each
/// layout, so a view that grows re-anchors the panel.
#[cfg(target_os = "macos")]
fn present_accessory_panel(state: &Rc<RefCell<ContextMenuState>>, host: &cocoa_ui::PlatformView) {
    let state = state.borrow();
    if state.panel.borrow().is_some() {
        return;
    }
    let Some(accessory) = &state.accessory else {
        return;
    };
    let accessory_view = view::retain_base(accessory.view());
    let sizing = Rc::clone(accessory);
    let panel = appkit::AccessoryPanel::new(state.mtm, host, &accessory_view, {
        // The container re-measures the accessory every layout pass.
        move || ideal_size(&sizing)
    });
    panel.order_front();
    *state.panel.borrow_mut() = Some(panel);
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
        #[cfg(target_os = "ios")]
        let preview = metadata.value.preview.map(|view| Rc::new(ctx.render(view)));
        #[cfg(target_os = "macos")]
        drop(metadata.value.preview);
        let accessory = metadata
            .value
            .accessory
            .map(|view| Rc::new(ctx.render(view)));

        let state = Rc::new(RefCell::new(ContextMenuState {
            child: mounted,
            env: ctx.env().clone(),
            items: metadata.value.items.clone(),
            #[cfg(target_os = "ios")]
            preview,
            accessory,
            #[cfg(target_os = "macos")]
            open_menu: RefCell::new(None),
            #[cfg(target_os = "macos")]
            panel: RefCell::new(None),
            #[cfg(target_os = "ios")]
            overlay: RefCell::new(None),
            #[cfg(target_os = "ios")]
            interaction: RefCell::new(None),
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
            *state.borrow_mut().interaction.borrow_mut() = Some(uikit::ContextMenu::install(
                &host,
                uikit::ContextMenuHandlers {
                    configuration: Rc::new({
                        let state = Rc::clone(&state);
                        move |_| {
                            let state = state.borrow();
                            let nodes = menu_items::tree_nodes(&state.items.snapshot(), &state.env);
                            if nodes.is_empty() {
                                return None;
                            }
                            let menu =
                                uikit::menu(state.mtm, &cocoa_ui::menu::Command::default(), &nodes);
                            let preview = state.preview.as_ref().map(|leaf| {
                                uikit::preview_controller(
                                    state.mtm,
                                    view::retain_base(leaf.view()),
                                    ideal_size(leaf),
                                )
                            });
                            Some(uikit::ContextMenuConfiguration { menu, preview })
                        }
                    }),
                    preview: Rc::new({
                        let state = Rc::clone(&state);
                        let host = host.clone();
                        move |_| {
                            let state = state.borrow();
                            let host: &cocoa_ui::PlatformView = &host;
                            state.preview.as_ref().map_or_else(
                                || Some(uikit::targeted_preview(host)),
                                |leaf| {
                                    let bounds = view::bounds(host);
                                    // The highlight preview runs before
                                    // the provider lays the view out;
                                    // give it its ideal bounds so the
                                    // lift has something to snapshot.
                                    if bounds_is_empty(leaf.view()) {
                                        let size = ideal_size(leaf);
                                        view::set_frame(
                                            leaf.view(),
                                            Rect::new(0.0, 0.0, size.width, size.height),
                                        );
                                    }
                                    Some(uikit::targeted_preview_at(
                                        leaf.view(),
                                        host,
                                        cocoa_ui::Point::new(
                                            bounds.size.width / 2.0,
                                            bounds.size.height / 2.0,
                                        ),
                                    ))
                                },
                            )
                        }
                    }),
                    will_display: Rc::new({
                        let state = Rc::clone(&state);
                        move |view| present_accessory(&state, view)
                    }),
                    will_end: Rc::new({
                        let state = Rc::clone(&state);
                        move |_| {
                            if let Some(overlay) = state.borrow().overlay.borrow_mut().take() {
                                overlay.dismiss();
                            }
                        }
                    }),
                },
            ));
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
                    Rc::new(appkit::ContextMenu::new(
                        mtm,
                        &nodes,
                        {
                            let state = Rc::clone(&state);
                            let host_view = view::retain_base(host_view);
                            move || present_accessory_panel(&state, &host_view)
                        },
                        {
                            let state = Rc::clone(&state);
                            move || {
                                state.borrow().open_menu.borrow_mut().take();
                                // Deferred: the click that closed the menu
                                // may be addressed to the accessory — a
                                // panel ordered out in the same tick would
                                // eat it.
                                let state = Rc::clone(&state);
                                let mtm = state.borrow().mtm;
                                cocoa_ui::main_queue::enqueue_local(mtm, move |_| {
                                    if let Some(panel) = state.borrow().panel.borrow_mut().take() {
                                        panel.order_out();
                                    }
                                });
                            }
                        },
                    ))
                };
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
        leaf.keep(state);
        leaf
    });
}
