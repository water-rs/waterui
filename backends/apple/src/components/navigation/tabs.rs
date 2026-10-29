//! `TabsLayout` — the adaptive tab container.
//!
//! iOS renders a `UITabBarController` (kit [`TabsController`]); macOS renders a
//! sidebar source list for `Sidebar` style and a segmented control above the
//! content otherwise. Each tab's `NavigationView` renders through the
//! navigation handler, so it draws its own standalone bar inside the pane.
//! The selection binding is two-way: native selection writes back, external
//! writes select the matching tab.

use alloc::vec::Vec;

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;
use waterui::navigation::{
    TabsLayout,
    tab::{Tab, TabIcon, TabRole},
};
use waterui_core::id::Id;
use waterui_core::layout::{StretchAxis, SubView, ViewDimensions};

use super::extract_title_text;

/// Installs the `TabsLayout` handler.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<TabsLayout>(platform::tabs_leaf);
}

struct Fill;

impl SubView for Fill {
    fn measure(&self, _proposal: waterui_core::layout::ProposalSize) -> ViewDimensions {
        ViewDimensions::new(waterui_core::layout::Size::new(0.0, 0.0))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// A tab rendered once: its pane's leaf, its label's extracted text and the
/// reactive `Tab` fields a spec bind still needs. `keep` holds the tab's own
/// watchers and tasks — badge ticks, enabled flips, the deferred icon
/// raster — so they die with the tab, not the container.
struct Mounted {
    /// Identifier the selection binding stores.
    id: Id,
    /// The tab pane's rendered leaf.
    pane: NativeLeaf,
    /// The label's rendered leaf (extracted into the tab chrome).
    #[allow(dead_code)]
    label_leaf: NativeLeaf,
    /// Label text for the tab chrome.
    label: alloc::string::String,
    /// SF Symbol name, when the icon is a system icon.
    symbol: Option<alloc::string::String>,
    /// The icon's rendered view, for custom-icon platforms.
    #[allow(dead_code)]
    icon_leaf: Option<NativeLeaf>,
    /// Badge signal.
    badge: Option<waterui::reactive::Computed<i32>>,
    /// Enabled signal.
    enabled: waterui::reactive::Computed<bool>,
    /// Search-role tab.
    #[allow(dead_code)]
    is_search: bool,
    /// Per-tab guards, kept while the tab is mounted.
    #[allow(dead_code)]
    keep: core::cell::RefCell<crate::contract::KeepAlive>,
}

/// Renders one tab once: the pane, the label's extracted text and the icon's
/// leaf the platform chrome still binds.
fn mount_tab(id: Id, tab: Tab, renderer: &crate::contract::Renderer) -> Mounted {
    let (label, icon, content, badge, enabled, role) = tab.into_parts();
    let pane = renderer.render(waterui_backend_core::AnyView::new(content.build()));
    let label_leaf = renderer.render(label.build());
    let label = extract_title_text(label_leaf.view());
    let (symbol, icon_leaf) = match icon {
        Some(TabIcon::System(icon)) => (Some(icon.name().to_string()), None),
        Some(TabIcon::View(icon)) => (None, Some(renderer.render(icon.build()))),
        None => (None, None),
    };

    Mounted {
        id,
        pane,
        label_leaf,
        label: label.unwrap_or_default(),
        symbol,
        icon_leaf,
        badge,
        enabled,
        is_search: matches!(role, TabRole::Search),
        keep: core::cell::RefCell::new(crate::contract::KeepAlive::default()),
    }
}

/// Rekeys `mounted` to `snapshot`'s membership by tab id: a surviving id
/// keeps its [`Mounted`] — pane, navigation state, GPU surfaces — untouched;
/// an inserted id mounts, a removed one drops. When the selected tab left
/// the set the id to relocate the selection to is returned — the tab now at
/// its index, the new last tab when it was trailing. The caller writes the
/// binding outside the `mounted` borrow so the selection watcher's own read
/// cannot re-enter it; an empty set leaves the binding untouched.
fn reconcile_tabs(
    mounted: &mut Vec<Mounted>,
    snapshot: &waterui_core::views::AnyViewsSnapshot<Tab>,
    renderer: &crate::contract::Renderer,
    selection: &waterui::reactive::Binding<Id>,
) -> Option<Id> {
    use waterui::reactive::Signal as _;
    use waterui_core::views::ViewSnapshot as _;
    let selected = selection.snapshot();
    let old_index = mounted.iter().position(|tab| tab.id == selected);
    let mut old: alloc::collections::BTreeMap<Id, Mounted> =
        mounted.drain(..).map(|tab| (tab.id, tab)).collect();
    let mut tabs = Vec::with_capacity(snapshot.len());
    for index in 0..snapshot.len() {
        let id = snapshot
            .get_id(index)
            .expect("a collection snapshot reports an id for every index");
        tabs.push(old.remove(&id).unwrap_or_else(|| {
            mount_tab(
                id,
                snapshot
                    .get_view(index)
                    .expect("a collection snapshot reports a view for every index"),
                renderer,
            )
        }));
    }
    *mounted = tabs;
    old_index.and_then(|old_index| {
        (!mounted.is_empty() && !mounted.iter().any(|tab| tab.id == selected))
            .then(|| mounted[old_index.min(mounted.len() - 1)].id)
    })
}

#[cfg(target_os = "ios")]
mod platform {
    use super::{Fill, Mounted, reconcile_tabs};
    use alloc::collections::BTreeMap;
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use crate::contract::{NativeLeaf, RenderContext};
    use cocoa_ui::geometry::Rect;
    use cocoa_ui::objc2_ui_kit::{UITabAccessory, UITabBarMinimizeBehavior};
    use cocoa_ui::uikit::{HostView, TabContentController, TabSpec, TabsController};
    use cocoa_ui::{Retained, view};
    use waterui::Environment;
    use waterui::navigation::{TabsLayout, tab::TabBarMinimizeBehavior};
    use waterui::reactive::Signal;
    use waterui_core::id::Id;
    use waterui_core::layout::ProposalSize;
    use waterui_core::views::Views as _;

    use crate::contract::KeepAlive;

    /// Uninstalls the capsule and unmounts its subtree on drop: `UIKit`
    /// retains the accessory's `contentView` inside the controller's view
    /// hierarchy, so the mounted child must detach explicitly — the leaf
    /// owns the mount, and its watchers die with it.
    ///
    /// The host's handlers hold the mount strongly; clearing them leaves
    /// this struct the mount's only owner, so `_mounted` — declared before
    /// `host` — detaches the child while the host is still retained. The
    /// `UITabAccessory` itself arrives after first paint (see the install
    /// below), so the slot retains it once it exists.
    struct BottomAccessory {
        tabs: Retained<TabsController>,
        _mounted: Rc<crate::contract::Mounted>,
        host: Retained<HostView>,
        _accessory: Rc<core::cell::OnceCell<Retained<UITabAccessory>>>,
    }

    impl Drop for BottomAccessory {
        fn drop(&mut self) {
            self.tabs.setBottomAccessory(None);
            self.host.clear_handlers();
        }
    }

    /// Installs one tab's `TabContentController`: the spec's chrome, the
    /// declared-scroll-surface association — `UIKit` asks the selected
    /// controller for its `contentScrollViewForEdge:` whenever it tracks a
    /// scroll — a rasterized template image for a custom icon, and the
    /// badge/enabled watchers mutating the `UITabBarItem` in place, all into
    /// the tab's own keep so they die with the tab, not the container.
    fn install_controller(
        mtm: cocoa_ui::MainThreadMarker,
        tab: &Mounted,
        env: &Environment,
    ) -> Retained<TabContentController> {
        // `env` feeds only the `gpu_surface` icon raster below.
        #[cfg(not(feature = "gpu_surface"))]
        let _ = env;
        let controller = TabContentController::new(mtm, tab.pane.view());
        controller.apply_spec(&spec(tab));
        controller.set_bottom_scroll_surface(|controller| {
            crate::primary_content::declared_scroll_view(controller.view()?.as_ref())
        });

        // A declared icon view draws inside the tab item as a template image
        // — the baseline's `installIconViews`, capped at the same 25pt side.
        if let Some(icon_leaf) = &tab.icon_leaf {
            let size = icon_leaf
                .layout()
                .measure(ProposalSize {
                    width: None,
                    height: None,
                })
                .size;
            cocoa_ui::view::set_frame(
                icon_leaf.view(),
                cocoa_ui::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
            );
            if let Some(item) = controller.tabBarItem() {
                #[cfg(feature = "gpu_surface")]
                {
                    // The raster cannot run synchronously under `CAMetalLayer`:
                    // the item waits, and the task the central capture
                    // completes installs the image — the tab's keep cancels
                    // it on removal.
                    let bounds = view::bounds(icon_leaf.view());
                    if bounds.size.width > 0.0 && bounds.size.height > 0.0 {
                        let icon_view = view::retain_base(icon_leaf.view());
                        let env = env.clone();
                        tab.keep
                            .borrow_mut()
                            .keep(executor_core::spawn_local(async move {
                                let image =
                                    crate::capture_image::template_image(&icon_view, &env, 25.0)
                                        .await;
                                item.setImage(Some(&image));
                            }));
                    }
                }
                #[cfg(not(feature = "gpu_surface"))]
                if let Some(image) = cocoa_ui::bitmap::view_template_image(icon_leaf.view(), 25.0) {
                    item.setImage(Some(&image));
                }
            }
        }

        // Reactive chrome mutates the `UITabBarItem` in place — the item
        // survives as long as its controller does.
        if let Some(item) = controller.tabBarItem() {
            if let Some(badge) = &tab.badge {
                let item = item.clone();
                tab.keep.borrow_mut().bind(badge, move |value| {
                    let badge = (value > 0)
                        .then(|| objc2_foundation::NSString::from_str(&value.to_string()));
                    item.setBadgeValue(badge.as_deref());
                });
            }
            tab.keep.borrow_mut().bind(&tab.enabled, move |enabled| {
                item.setEnabled(enabled);
            });
        }
        controller
    }

    /// Rekeys the installed controllers to `mounted`'s membership: dropped
    /// ids' controllers release — unmounting their panes — new ids install,
    /// and the array is re-set in the mounted order.
    fn refresh(
        mtm: cocoa_ui::MainThreadMarker,
        env: &Environment,
        tabs: &TabsController,
        mounted: &RefCell<Vec<Mounted>>,
        controllers: &RefCell<BTreeMap<Id, Retained<TabContentController>>>,
    ) {
        let mounted = mounted.borrow();
        let mut controllers = controllers.borrow_mut();
        controllers.retain(|id, _| mounted.iter().any(|tab| tab.id == *id));
        for tab in mounted.iter() {
            controllers
                .entry(tab.id)
                .or_insert_with(|| install_controller(mtm, tab, env));
        }
        let ordered: Vec<_> = mounted
            .iter()
            .map(|tab| {
                controllers
                    .get(&tab.id)
                    .expect("a mounted tab has its controller installed")
                    .clone()
            })
            .collect();
        tabs.set_view_controllers(&ordered);
    }

    /// `UITabBarController` with one `UIViewController` per pane.
    #[allow(clippy::too_many_lines)]
    pub(super) fn tabs_leaf(mut layout: TabsLayout, ctx: &RenderContext) -> NativeLeaf {
        let mtm = ctx.mtm();
        let tabs = TabsController::new(mtm);
        let mut keep = KeepAlive::default();
        let renderer = ctx.renderer();
        let env = ctx.env().clone();
        let mounted: Rc<RefCell<Vec<Mounted>>> = Rc::new(RefCell::new(Vec::new()));
        let controllers = Rc::new(RefCell::new(
            BTreeMap::<Id, Retained<TabContentController>>::new(),
        ));
        {
            let snapshot = layout.tabs().snapshot();
            let _ = reconcile_tabs(
                &mut mounted.borrow_mut(),
                &snapshot,
                &renderer,
                layout.selection(),
            );
        }
        refresh(mtm, &env, &tabs, &mounted, &controllers);

        tabs.setTabBarMinimizeBehavior(match layout.get_minimize_behavior() {
            TabBarMinimizeBehavior::Automatic => UITabBarMinimizeBehavior::Automatic,
            TabBarMinimizeBehavior::Never => UITabBarMinimizeBehavior::Never,
            TabBarMinimizeBehavior::OnScrollDown => UITabBarMinimizeBehavior::OnScrollDown,
            TabBarMinimizeBehavior::OnScrollUp => UITabBarMinimizeBehavior::OnScrollUp,
            other => unimplemented!("unsupported TabBarMinimizeBehavior variant: {other:?}"),
        });

        // The bottom accessory is a controller-level capsule: the shared
        // view renders once into a host whose intrinsic measure forwards
        // the child's answer, then UIKit owns the capsule's placement,
        // sizing and collapse.
        if let Some(accessory) = layout.take_bottom_accessory() {
            let accessory_host = HostView::new(mtm, Rect::ZERO);
            let mounted = Rc::new(ctx.render(accessory).mount(&accessory_host));
            accessory_host.set_measure_handler({
                let mounted = Rc::clone(&mounted);
                move |_host, proposal| {
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "the layout contract is f32; measured points always fit"
                    )]
                    let measured = mounted.layout().measure(ProposalSize::new(
                        proposal.width.map(|width| width as f32),
                        proposal.height.map(|height| height as f32),
                    ));
                    cocoa_ui::geometry::Size::new(
                        f64::from(measured.size.width),
                        f64::from(measured.size.height),
                    )
                }
            });
            // `UIKit` sizes the accessory's content view by capsule
            // geometry, wider than a non-stretch child's intrinsic width:
            // measure the child under the bounds proposal and center its
            // measured frame, as `MaterialBackground` does. A stretching
            // child measures back to bounds, so this covers both.
            accessory_host.set_layout_handler({
                let mounted = Rc::clone(&mounted);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "UIKit geometry is `CGFloat`; proposals are `f32`"
                )]
                move |host| {
                    let bounds = view::bounds(host);
                    let subviews = view::subviews(host);
                    let Some(sub) = subviews.first() else {
                        return;
                    };
                    let proposal = ProposalSize::new(
                        Some(bounds.size.width as f32),
                        Some(bounds.size.height as f32),
                    );
                    let measured = mounted.layout().measure(proposal).size;
                    let width = f64::from(measured.width);
                    let height = f64::from(measured.height);
                    let frame = Rect::new(
                        (bounds.size.width - width) / 2.0,
                        (bounds.size.height - height) / 2.0,
                        width,
                        height,
                    );
                    crate::proposal::deliver(
                        sub,
                        crate::proposal::Placement::platform_hosted(proposal),
                    );
                    view::set_frame(sub, frame);
                }
            });
            // The capsule installs when the current transaction commits —
            // after the first frame, never inside the launch commit. On
            // iOS 26 an accessory installed during the scene's own
            // presentation can have its materialise transition dropped,
            // leaving the pocket flat and tinting the content's labels for
            // the rest of the process; installing into the already-live
            // tab bar is the path a now-playing bar appearing mid-run
            // takes — the same deferred install the parity twin declares.
            // The closure holds the slot weakly: `BottomAccessory` owns the
            // only strong reference, so tearing the tabs down before the
            // commit makes the upgrade fail and cancels the install.
            let accessory = Rc::new(core::cell::OnceCell::new());
            cocoa_ui::core_animation::on_commit({
                let tabs = tabs.clone();
                let accessory_host = accessory_host.clone();
                let accessory = Rc::downgrade(&accessory);
                move || {
                    let Some(accessory) = accessory.upgrade() else {
                        return;
                    };
                    let capsule = UITabAccessory::initWithContentView(mtm.alloc(), &accessory_host);
                    tabs.setBottomAccessory(Some(&capsule));
                    let _ = accessory.set(capsule);
                }
            });
            keep.keep(BottomAccessory {
                tabs: tabs.clone(),
                _mounted: mounted,
                host: accessory_host,
                _accessory: accessory,
            });
        }

        // Two-way selection.
        tabs.set_select_handler({
            let selection = layout.selection().clone();
            let mounted = mounted.clone();
            move |index| {
                // Copy the id under a short borrow — `selection.set` runs user
                // watchers that may mutate the collection and re-enter the
                // membership watch, which takes `mounted` mutably.
                let id = mounted.borrow().get(index).map(|tab| tab.id);
                if let Some(id) = id {
                    selection.set(id);
                }
            }
        });
        keep.bind(layout.selection(), {
            let tabs = tabs.clone();
            let mounted = mounted.clone();
            move |id| {
                let index = mounted.borrow().iter().position(|tab| tab.id == id);
                if let Some(index) = index {
                    tabs.select(index);
                }
            }
        });

        // Membership changes re-key the mounted set by tab id: a surviving
        // tab keeps its controller — pane, navigation state, surfaces — the
        // array is re-set in the new order.
        keep.keep(layout.tabs().watch(.., {
            let mounted = mounted.clone();
            let selection = layout.selection().clone();
            let tabs = tabs.clone();
            move |ctx, _change| {
                let snapshot = ctx.into_value();
                let relocation =
                    reconcile_tabs(&mut mounted.borrow_mut(), &snapshot, &renderer, &selection);
                refresh(mtm, &env, &tabs, &mounted, &controllers);
                if let Some(id) = relocation {
                    selection.set(id);
                }
            }
        }));

        // A `UITabBarController` lays its own content out against the screen
        // edges — its content region reaches the top chrome and the tab bar
        // owns the bottom inset — so the leaf reports `WuiSafeAreaManaging`
        // through a kit host: the window root hands it the whole window, not
        // the safe-area rect.
        let host = HostView::new(mtm, Rect::ZERO);
        host.set_manages_safe_area(true);
        let tabs_view = tabs.view().expect("tab bar view");
        view::add_subview(&host, &tabs_view);
        host.set_primary_content_handler({
            let tabs_view = tabs_view.clone();
            move |_| Some(tabs_view.clone())
        });
        host.set_layout_handler(|host| {
            let bounds = view::bounds(host);
            if let Some(sub) = view::subviews(host).first() {
                view::set_frame(sub, bounds);
            }
        });
        let mut leaf = NativeLeaf::new(&*host, Fill);
        leaf.keep(host);
        leaf.keep(tabs_view);
        leaf.keep(keep);
        leaf.keep(mounted);
        leaf
    }

    fn spec(tab: &Mounted) -> TabSpec {
        TabSpec {
            label: tab.label.clone(),
            symbol: tab.symbol.clone(),
            badge: tab.badge.as_ref().map(|badge| badge.snapshot().to_string()),
            is_search: tab.is_search,
            enabled: tab.enabled.snapshot(),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{Fill, Mounted, reconcile_tabs};
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use crate::contract::{NativeLeaf, RenderContext};
    use crate::native_layout::LayoutContext;
    use cocoa_ui::appkit::{HostView, Segment, SegmentedControl, SourceList, WindowToolbar};
    use cocoa_ui::objc2_app_kit::NSWindowStyleMask;
    use cocoa_ui::{Rect, view};
    use waterui::navigation::{TabsLayout, tab::NativeTabStyle};
    use waterui::reactive::Signal;
    use waterui_core::id::Id;
    use waterui_core::views::{ViewSnapshot as _, Views as _};

    use crate::contract::KeepAlive;

    /// The segments one mounted set's labels describe, in bar order.
    fn segments(mounted: &[Mounted]) -> Vec<Segment> {
        mounted
            .iter()
            .map(|tab| Segment {
                label: tab.label.clone(),
                symbol: tab.symbol.clone(),
                image: None,
                enabled: tab.enabled.snapshot(),
                badge: tab.badge.as_ref().map(|badge| badge.snapshot().to_string()),
                is_search: tab.is_search,
            })
            .collect()
    }

    /// macOS: `Sidebar` style gets a source-list column; everything else a
    /// segmented control strip above the selected pane.
    #[allow(clippy::too_many_lines)]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the `register_native` signature takes `TabsLayout` by value; the iOS twin consumes `bottom_accessory`, AppKit has no such slot"
    )]
    pub(super) fn tabs_leaf(layout: TabsLayout, ctx: &RenderContext) -> NativeLeaf {
        let mtm = ctx.mtm();
        // The host places each pane against the safe area itself
        // (`region_frame`): a scroll surface reaches under the window
        // toolbar, anything else stays clear of it (§7.1). So the window
        // root hands it the whole window rather than the safe-area rect.
        let host = HostView::new(mtm, Rect::ZERO);
        host.set_manages_safe_area(true);
        let mut keep = KeepAlive::default();
        let renderer = ctx.renderer();
        let mounted: Rc<RefCell<Vec<Mounted>>> = Rc::new(RefCell::new(Vec::new()));
        {
            let snapshot = layout.tabs().snapshot();
            let _ = reconcile_tabs(
                &mut mounted.borrow_mut(),
                &snapshot,
                &renderer,
                layout.selection(),
            );
        }
        keep.keep(mounted.clone());
        for tab in mounted.borrow().iter() {
            view::add_subview(&host, tab.pane.view());
        }

        let sidebar = matches!(layout.get_style(), NativeTabStyle::Sidebar);
        let chrome = if sidebar {
            let list = SourceList::new(mtm);
            list.set_segments(&segments(&mounted.borrow()));
            view::add_subview(&host, list.view());
            list.set_select_handler({
                let selection = layout.selection().clone();
                let mounted = mounted.clone();
                move |index| {
                    // Short borrow — `selection.set` runs watchers that may
                    // mutate the collection, re-entering the membership
                    // watch's `mounted` borrow_mut.
                    let id = mounted.borrow().get(index).map(|tab| tab.id);
                    if let Some(id) = id {
                        selection.set(id);
                    }
                }
            });
            Chrome::List(Rc::new(list))
        // (control path adds via add_subview below)
        } else {
            let control = SegmentedControl::new(mtm);
            control.set_segments(&segments(&mounted.borrow()));
            view::add_subview(&host, &control);
            control.set_select_handler({
                let selection = layout.selection().clone();
                let mounted = mounted.clone();
                move |index| {
                    // Short borrow — `selection.set` runs watchers that may
                    // mutate the collection, re-entering the membership
                    // watch's `mounted` borrow_mut.
                    let id = mounted.borrow().get(index).map(|tab| tab.id);
                    if let Some(id) = id {
                        selection.set(id);
                    }
                }
            });
            Chrome::Control(control)
        };
        keep.keep(chrome.clone_view());

        // Show the selected pane only; select() returns the index for bind.
        let show = {
            let mounted = mounted.clone();
            let chrome = chrome.clone();
            move |index: Option<usize>| {
                let mounted = mounted.borrow();
                for (pane_index, tab) in mounted.iter().enumerate() {
                    view::set_hidden(tab.pane.view(), index != Some(pane_index));
                }
                chrome.select(index);
                // A pane's own view wraps the navigation container inside it,
                // so hiding the pane never reaches the stack's host and its
                // chrome handler never runs: flip every chrome-aware host in
                // the pane so hidden stacks withdraw and the visible stack
                // publishes — `setNavigationChromeActive(_:)`.
                for (pane_index, tab) in mounted.iter().enumerate() {
                    flip_chrome_hosts(tab.pane.view(), index != Some(pane_index));
                }
            }
        };
        let selected = layout.selection().snapshot();
        let index = mounted.borrow().iter().position(|tab| tab.id == selected);
        show(index);

        keep.bind(layout.selection(), {
            let show = show.clone();
            let mounted = mounted.clone();
            move |id| {
                let index = mounted.borrow().iter().position(|tab| tab.id == id);
                show(index);
            }
        });

        // Membership changes re-key the mounted set by tab id: a removed
        // tab's pane detaches before its Mounted drops, an inserted tab's
        // pane mounts and joins the host, the segments rebuild in the new
        // order — a surviving tab's Mounted — pane, navigation state,
        // surfaces — is never touched.
        keep.keep(layout.tabs().watch(.., {
            let mounted = mounted.clone();
            let host = host.clone();
            let chrome = chrome.clone();
            let selection = layout.selection().clone();
            let show = show.clone();
            move |ctx, _change| {
                let snapshot = ctx.into_value();
                // Detach the panes of the ids the change removes before the
                // reconcile drops their Mounted — the leaf releases, the
                // view must leave the host.
                for tab in mounted.borrow().iter() {
                    let removed =
                        !(0..snapshot.len()).any(|index| snapshot.get_id(index) == Some(tab.id));
                    if removed {
                        tab.pane.view().removeFromSuperview();
                    }
                }
                let before: alloc::collections::BTreeSet<Id> =
                    mounted.borrow().iter().map(|tab| tab.id).collect();
                let relocation =
                    reconcile_tabs(&mut mounted.borrow_mut(), &snapshot, &renderer, &selection);
                {
                    let mounted = mounted.borrow();
                    for tab in mounted.iter() {
                        if before.contains(&tab.id) {
                            continue;
                        }
                        view::add_subview(&host, tab.pane.view());
                        view::set_hidden(tab.pane.view(), true);
                    }
                    match &chrome {
                        Chrome::List(list) => list.set_segments(&segments(&mounted)),
                        Chrome::Control(control) => control.set_segments(&segments(&mounted)),
                    }
                }
                let id = selection.snapshot();
                let index = mounted.borrow().iter().position(|tab| tab.id == id);
                show(index);
                if let Some(id) = relocation {
                    selection.set(id);
                }
            }
        }));

        // A titled window carries the segmented strip in its unified
        // titlebar instead of in-content; untitled windows keep the strip.
        let in_toolbar = Rc::new(core::cell::Cell::new(false));
        host.set_window_handler({
            let chrome = chrome.clone();
            let in_toolbar = in_toolbar.clone();
            move |host| {
                let Some(window) = view::window(host) else {
                    return;
                };
                let titled = window.styleMask().contains(NSWindowStyleMask::Titled);
                if let Chrome::Control(_) = &chrome {
                    if titled {
                        WindowToolbar::attached(&window)
                            .set_tabs(Some(cocoa_ui::view::retain_base(chrome.view())));
                    }
                    in_toolbar.set(titled);
                }
                host.set_needs_layout();
            }
        });

        host.set_layout_handler({
            move |host| {
                let host_view: &cocoa_ui::PlatformView = host;
                let context = LayoutContext::of(host_view);
                let bounds = view::bounds(host);
                let place = |content: &cocoa_ui::PlatformView, region: Rect| {
                    view::set_frame(content, context.region_frame(content, region));
                };
                if sidebar {
                    let width = 220.0_f64.min(bounds.size.width / 2.0);
                    place(
                        chrome.view(),
                        Rect::new(bounds.origin.x, bounds.origin.y, width, bounds.size.height),
                    );
                    for tab in mounted.borrow().iter() {
                        place(
                            tab.pane.view(),
                            Rect::new(
                                bounds.origin.x + width,
                                bounds.origin.y,
                                bounds.size.width - width,
                                bounds.size.height,
                            ),
                        );
                    }
                } else if in_toolbar.get() {
                    for tab in mounted.borrow().iter() {
                        place(tab.pane.view(), bounds);
                    }
                } else {
                    let height = view::fitting_size(chrome.view()).height;
                    // `HostView` is flipped: the smallest y is the top edge,
                    // so the strip docks at the safe rect's top and the
                    // panes fill what lies below it.
                    let strip_top = context.safe_rect().origin.y + 8.0;
                    let panes_top = strip_top + height + 8.0;
                    place(
                        chrome.view(),
                        Rect::new(
                            bounds.origin.x + 8.0,
                            strip_top,
                            bounds.size.width - 16.0,
                            height,
                        ),
                    );
                    for tab in mounted.borrow().iter() {
                        place(
                            tab.pane.view(),
                            Rect::new(
                                bounds.origin.x,
                                panes_top,
                                bounds.size.width,
                                bounds.origin.y + bounds.size.height - panes_top,
                            ),
                        );
                    }
                }
            }
        });

        let mut leaf = NativeLeaf::new(&*host, Fill);
        leaf.keep(keep);
        leaf.keep(show);
        leaf
    }

    /// Flips `isHidden` on every `HostView` in the pane's subtree that asked
    /// for hidden notifications, so a navigation container nested inside the
    /// pane sees the same visibility flip the pane itself just got.
    fn flip_chrome_hosts(view: &cocoa_ui::PlatformView, hidden: bool) {
        for subview in view::subviews(view) {
            if let Some(host) = subview.downcast_ref::<HostView>()
                && host.wants_hidden_events()
            {
                view::set_hidden(&subview, hidden);
            }
            flip_chrome_hosts(&subview, hidden);
        }
    }

    /// Either chrome variant behind one tiny interface.
    enum Chrome {
        /// `Sidebar` style's source list.
        List(Rc<SourceList>),
        /// The segmented strip.
        Control(cocoa_ui::Retained<SegmentedControl>),
    }

    impl Chrome {
        fn view(&self) -> &cocoa_ui::PlatformView {
            match self {
                Self::List(list) => list.view(),
                Self::Control(control) => control,
            }
        }

        fn select(&self, index: Option<usize>) {
            match self {
                Self::List(list) => list.select(index),
                Self::Control(control) => {
                    if let Some(index) = index {
                        control.select(Some(index));
                    }
                }
            }
        }

        fn clone_view(&self) -> cocoa_ui::Retained<cocoa_ui::PlatformView> {
            cocoa_ui::view::retain_base(self.view())
        }
    }

    impl Clone for Chrome {
        fn clone(&self) -> Self {
            match self {
                Self::List(list) => Self::List(list.clone()), // Rc clone
                Self::Control(control) => Self::Control(control.clone()),
            }
        }
    }
}
