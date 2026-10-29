//! `NavigationSplitLayout` — the two/three-column adaptive container.
//!
//! iOS renders a `UISplitViewController` (double or triple column): the
//! sidebar column hosts a [`NavContentController`] inside an implicit
//! `UINavigationController`, while the middle and detail columns are
//! stack-driven `UINavigationController`s, so a column's own
//! `NavigationView` chrome lands on `navigationItem` and a collapse onto
//! compact pushes the page with the platform's back affordance.
//! macOS renders the kit [`SplitViewController`] (`NSSplitViewController`).
//! Selection bindings rebuild the downstream columns; column visibility maps
//! to per-column collapse.

use core::cell::RefCell;

use crate::dispatch::Dispatcher;
use waterui::navigation::{
    NavigationSplitLayout, split::NavigationSplitDetailBuilder as DetailBuilder,
};
use waterui::reactive::{Binding, Signal};
use waterui_backend_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_core::layout::{StretchAxis, SubView, ViewDimensions};

use crate::contract::KeepAlive;

/// Installs the `NavigationSplitLayout` handler.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<NavigationSplitLayout>(platform::split_leaf);
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

/// What a selection change must rebuild: everything downstream of it.
struct Columns {
    /// Renders views after the handler returns.
    renderer: crate::contract::Renderer,
    /// The middle column's builder, when the split has three columns.
    content: Option<DetailBuilder>,
    /// The detail column's builder.
    detail: DetailBuilder,
    /// The middle column's empty-selection placeholder.
    placeholder: AnyViewBuilder<AnyView>,
    /// Sidebar selection.
    primary: Binding<Option<Id>>,
    /// Middle-column selection, three-column splits only.
    secondary: Option<Binding<Option<Id>>>,
    /// Guard for every leaf mounted in a column.
    mounted: RefCell<KeepAlive>,
    /// The selection-watcher guards — separate from `mounted` because
    /// `bind` fires immediately while its borrow is still held, and the
    /// watcher re-borrows `mounted`.
    watchers: RefCell<KeepAlive>,
}

impl Columns {
    /// The middle column's selected content (three columns), or `None`
    /// when nothing is selected or the split has two columns.
    fn middle_destination(&self) -> Option<AnyView> {
        match (&self.content, self.primary.snapshot()) {
            (Some(content), Some(id)) => Some(AnyView::new(content.build(id))),
            _ => None,
        }
    }

    /// The detail column's selected content: the primary selection drives
    /// two-column splits; the secondary selection drives three-column
    /// splits. `None` when nothing is selected.
    fn detail_destination(&self) -> Option<AnyView> {
        self.secondary
            .as_ref()
            .map_or_else(
                || self.primary.snapshot(),
                waterui::reactive::Signal::snapshot,
            )
            .map(|id| AnyView::new(self.detail.build(id)))
    }

    /// The middle column's content, or the placeholder.
    #[cfg(target_os = "macos")]
    fn middle_view(&self) -> AnyView {
        self.middle_destination()
            .unwrap_or_else(|| self.placeholder.build())
    }

    /// The detail column's content, or the placeholder.
    #[cfg(target_os = "macos")]
    fn detail_view(&self) -> AnyView {
        self.detail_destination()
            .unwrap_or_else(|| self.placeholder.build())
    }

    /// The root of a stack-driven column: a selected destination is
    /// resolved to its `NavigationView`; the empty-selection placeholder
    /// becomes the content of a `NavigationView` with the default,
    /// untitled bar, since it is a plain view with no navigation root to
    /// resolve.
    #[cfg(target_os = "ios")]
    fn column_root(
        &self,
        destination: Option<AnyView>,
    ) -> crate::components::navigation::stack::ColumnRoot {
        use crate::components::navigation::stack::ColumnRoot;
        destination.map_or_else(
            || {
                ColumnRoot::Resolved(Box::new(waterui::navigation::NavigationView {
                    bar: waterui::navigation::Bar::default(),
                    content: self.placeholder.build(),
                    state: waterui::navigation::NavigationDestinationState::default(),
                    transition: None,
                }))
            },
            ColumnRoot::Destination,
        )
    }
}

#[cfg(target_os = "ios")]
mod platform {
    use super::{Columns, Fill};
    use alloc::rc::Rc;
    use core::cell::{Cell, RefCell};

    use crate::contract::{NativeLeaf, RenderContext};
    use cocoa_ui::Retained;
    use cocoa_ui::geometry::Rect;
    use cocoa_ui::objc2::MainThreadOnly;
    use cocoa_ui::objc2_ui_kit::{UINavigationController, UISplitViewControllerColumn};
    use cocoa_ui::uikit::{HostView, NavContentController, SplitController};
    use cocoa_ui::view;
    use waterui::navigation::{NavigationSplitColumnVisibility, NavigationSplitLayout};
    use waterui::reactive::Signal;

    use crate::contract::KeepAlive;

    /// `UIKit` wraps the sidebar column in a nav controller whose bar
    /// reserves its height in the safe area even when it draws nothing, so
    /// the page hides the bar while it is the stack's base — a pushed page
    /// keeps it for the back affordance. (The detail and middle columns
    /// are stack-driven nav controllers whose own bar carries chrome.)
    fn hide_bar_when_base(vc: &Retained<NavContentController>) {
        let weak = cocoa_ui::objc2::rc::Weak::new(&**vc);
        vc.set_will_appear_handler(move |animated| {
            let Some(vc) = weak.load() else { return };
            let Some(nav) = vc.navigationController() else {
                return;
            };
            let is_base = nav.viewControllers().firstObject().is_some_and(|first| {
                cocoa_ui::objc2::rc::Retained::as_ptr(&first).cast::<u8>()
                    == cocoa_ui::objc2::rc::Retained::as_ptr(&vc).cast::<u8>()
            });
            nav.setNavigationBarHidden_animated(is_base, animated);
        });
    }

    struct Split {
        columns: Columns,
        nav: Retained<SplitController>,
        applying_selection: Cell<bool>,
    }

    /// The sidebar column is a navigation-content host around the sidebar
    /// leaf; middle/detail columns are rebuilt on selection changes.
    #[allow(clippy::too_many_lines)]
    pub(super) fn split_leaf(layout: NavigationSplitLayout, ctx: &RenderContext) -> NativeLeaf {
        let (
            sidebar,
            placeholder,
            primary,
            content,
            secondary,
            detail,
            visibility,
            sidebar_width,
            _style,
        ) = layout.into_parts();
        let mtm = ctx.mtm();
        let nav = SplitController::new(mtm, content.is_some());
        nav.set_collapsed_top_column(Some(if content.is_none() && primary.snapshot().is_some() {
            UISplitViewControllerColumn::Secondary
        } else {
            UISplitViewControllerColumn::Primary
        }));

        let sidebar_leaf = ctx.render(sidebar.build());
        let sidebar_vc = NavContentController::new(mtm, sidebar_leaf.view());
        hide_bar_when_base(&sidebar_vc);
        // UIKit's implicit wrapper unbalances the sidebar's appearance
        // transition when the split initially collapses onto the detail.
        let sidebar_nav = UINavigationController::initWithRootViewController(
            UINavigationController::alloc(mtm),
            &sidebar_vc,
        );
        sidebar_nav.navigationBar().setPrefersLargeTitles(true);
        nav.set_column(UISplitViewControllerColumn::Primary, &sidebar_nav);

        let columns = Columns {
            renderer: ctx.renderer(),
            content,
            detail,
            placeholder,
            primary,
            secondary,
            mounted: RefCell::new(KeepAlive::default()),
            watchers: RefCell::new(KeepAlive::default()),
        };
        let mut keep = KeepAlive::default();
        keep.keep(sidebar_leaf);
        keep.keep(sidebar_vc);
        let nav_for_binds = nav.clone();
        keep.keep(nav.clone());
        let has_middle = columns.content.is_some();
        let split = Rc::new(Split {
            columns,
            nav,
            applying_selection: Cell::new(false),
        });

        if split.columns.content.is_some() {
            split.mount_middle(&mut keep);
        }
        split.mount_detail(&mut keep);
        *split.columns.mounted.borrow_mut() = keep;

        // Selection bindings rebuild the downstream columns.
        let primary = split.columns.primary.clone();
        split.columns.watchers.borrow_mut().watch(&primary, {
            let split = split.clone();
            move |_| {
                split.applying_selection.set(true);
                let mut keep = split.columns.mounted.borrow_mut();
                if split.columns.content.is_some() {
                    split.mount_middle(&mut keep);
                } else {
                    split.mount_detail(&mut keep);
                }
                drop(keep);
                split.present_selection();
                split.applying_selection.set(false);
            }
        });
        if let Some(secondary) = &split.columns.secondary {
            split.columns.watchers.borrow_mut().bind(secondary, {
                let split = split.clone();
                move |_| {
                    let mut keep = split.columns.mounted.borrow_mut();
                    split.mount_detail(&mut keep);
                }
            });
        }

        nav_for_binds.set_collapse_handler({
            let split = Rc::downgrade(&split);
            move |column, collapsed| {
                let Some(split) = split.upgrade() else { return };
                if split.columns.content.is_none()
                    && !split.applying_selection.get()
                    && split.nav.isCollapsed()
                    && column == UISplitViewControllerColumn::Primary.0
                    && !collapsed
                    && split.columns.primary.snapshot().is_some()
                {
                    split.columns.primary.set(None);
                }
            }
        });

        split.columns.watchers.borrow_mut().bind(&visibility, {
            let nav = nav_for_binds.clone();
            move |visibility| {
                let (sidebar, content) = match visibility {
                    NavigationSplitColumnVisibility::All
                    | NavigationSplitColumnVisibility::Automatic => (false, false),
                    NavigationSplitColumnVisibility::DoubleColumn => (true, false),
                    NavigationSplitColumnVisibility::DetailOnly => (true, true),
                };
                nav.set_collapsed(UISplitViewControllerColumn::Primary, sidebar);
                if has_middle {
                    nav.set_collapsed(UISplitViewControllerColumn::Supplementary, content);
                }
            }
        });

        nav_for_binds.set_column_widths(
            UISplitViewControllerColumn::Primary,
            f64::from(sidebar_width.ideal()),
            f64::from(sidebar_width.min()),
            f64::from(sidebar_width.max()),
        );

        // A `UISplitViewController` owns its columns' bars and insets —
        // `WuiSafeAreaManaging` in the baseline — so the leaf reports it
        // through a kit host and the window root hands it the whole window,
        // not the safe-area rect.
        let host = HostView::new(mtm, Rect::ZERO);
        host.set_manages_safe_area(true);
        let split_view = nav_for_binds.view().expect("split view");
        view::add_subview(&host, &split_view);
        host.set_primary_content_handler({
            let split_view = split_view.clone();
            move |_| Some(split_view.clone())
        });
        host.set_layout_handler(|host| {
            let bounds = view::bounds(host);
            if let Some(sub) = view::subviews(host).first() {
                view::set_frame(sub, bounds);
            }
        });
        let mut leaf = NativeLeaf::new(&*host, Fill);
        leaf.keep(host);
        leaf.keep(split_view);
        leaf.keep(split);
        leaf
    }

    impl Split {
        fn present_selection(&self) {
            if self.columns.content.is_some() {
                return;
            }
            let column = if self.columns.primary.snapshot().is_some() {
                UISplitViewControllerColumn::Secondary
            } else {
                UISplitViewControllerColumn::Primary
            };
            self.nav.set_collapsed_top_column(Some(column));
            if self.nav.isCollapsed() {
                self.nav.show_column(column);
            }
        }

        /// Middle/detail columns render through the stack driver: a real
        /// `UINavigationController` per column, so the page's chrome —
        /// large title, the `searchable` field, toolbar items — lands on
        /// `navigationItem`, and a collapse pushes the top page onto the
        /// compact stack with the platform's own back affordance.
        fn mount_middle(&self, keep: &mut KeepAlive) {
            let ctx = self.columns.renderer.context();
            let (leaf, column_nav) = crate::components::navigation::stack::column_leaf(
                self.columns.column_root(self.columns.middle_destination()),
                &ctx,
            );
            self.nav
                .set_column(UISplitViewControllerColumn::Supplementary, &column_nav);
            keep.keep(leaf);
            keep.keep(column_nav);
        }

        fn mount_detail(&self, keep: &mut KeepAlive) {
            let ctx = self.columns.renderer.context();
            let (leaf, column_nav) = crate::components::navigation::stack::column_leaf(
                self.columns.column_root(self.columns.detail_destination()),
                &ctx,
            );
            self.nav
                .set_column(UISplitViewControllerColumn::Secondary, &column_nav);
            keep.keep(leaf);
            keep.keep(column_nav);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{Columns, Fill};
    use alloc::rc::Rc;
    use core::cell::{Cell, RefCell};

    use crate::contract::{NativeLeaf, RenderContext};
    use cocoa_ui::appkit::{Column, ColumnWidth, HostView, SplitViewController, WindowToolbar};
    use cocoa_ui::objc2_app_kit::NSWindowStyleMask;
    use cocoa_ui::{MainThreadMarker, Rect, Retained, view};
    use waterui::navigation::{NavigationSplitColumnVisibility, NavigationSplitLayout};

    use crate::contract::KeepAlive;

    struct Split {
        columns: Columns,
        nav: Retained<SplitViewController>,
        /// The sidebar column's view, reinstalled on every `set_columns`.
        sidebar: Retained<cocoa_ui::PlatformView>,
        /// Whether the split's host view is effectively visible. An
        /// expanded `allowsFullHeightLayout` sidebar item claims titlebar
        /// space even while its view is hidden, so a hidden pane's sidebar
        /// item is collapsed for as long as the pane stays hidden.
        chrome_active: Cell<bool>,
        /// The sidebar collapse state the app or user wants — what
        /// `apply_sidebar` restores when the pane shows again.
        sidebar_collapsed: Cell<bool>,
    }

    /// `NSSplitViewController` with the sidebar leaf and rebuilt
    /// supplementary/detail columns.
    pub(super) fn split_leaf(layout: NavigationSplitLayout, ctx: &RenderContext) -> NativeLeaf {
        let (
            sidebar,
            placeholder,
            primary,
            content,
            secondary,
            detail,
            visibility,
            sidebar_width,
            _style,
        ) = layout.into_parts();
        let mtm = ctx.mtm();
        let nav = SplitViewController::new(mtm);

        let sidebar_env = ctx
            .env()
            .extending(waterui::component::list::SidebarContent);
        let sidebar_leaf = ctx.with_env(&sidebar_env).render(sidebar.build());
        let columns = Columns {
            renderer: ctx.renderer(),
            content,
            detail,
            placeholder,
            primary,
            secondary,
            mounted: RefCell::new(KeepAlive::default()),
            watchers: RefCell::new(KeepAlive::default()),
        };
        let sidebar_view = cocoa_ui::view::retain_base(sidebar_leaf.view());
        let split = Rc::new(Split {
            columns,
            nav,
            sidebar: sidebar_view,
            chrome_active: Cell::new(true),
            sidebar_collapsed: Cell::new(false),
        });

        let mut keep = KeepAlive::default();
        keep.keep(sidebar_leaf);
        keep.keep(sidebar_env);
        split.mount(&mut keep);
        *split.columns.mounted.borrow_mut() = keep;

        let primary = split.columns.primary.clone();
        split.columns.watchers.borrow_mut().bind(&primary, {
            let split = split.clone();
            move |_| {
                let mut keep = split.columns.mounted.borrow_mut();
                split.mount(&mut keep);
            }
        });
        if let Some(secondary) = &split.columns.secondary {
            split.columns.watchers.borrow_mut().bind(secondary, {
                let split = split.clone();
                move |_| {
                    let mut keep = split.columns.mounted.borrow_mut();
                    split.mount(&mut keep);
                }
            });
        }

        split.columns.watchers.borrow_mut().bind(&visibility, {
            let split = split.clone();
            move |visibility| {
                let (sidebar, content) = match visibility {
                    NavigationSplitColumnVisibility::All
                    | NavigationSplitColumnVisibility::Automatic => (false, false),
                    NavigationSplitColumnVisibility::DoubleColumn => (true, false),
                    NavigationSplitColumnVisibility::DetailOnly => (true, true),
                };
                split.sidebar_collapsed.set(sidebar);
                split.nav.set_collapsed(Column::Supplementary, content);
                split.apply_sidebar();
            }
        });

        // The platform's collapse reports are the user's own toggles, so
        // they become the wanted state — but only while the pane's chrome
        // is live. While the pane is hidden the reported collapse is this
        // leaf's own forced one, which must not overwrite it. Captured
        // weakly: `Split` owns `nav`, and `nav` owns the handler.
        split.nav.set_collapse_handler({
            let split = Rc::downgrade(&split);
            move |column, collapsed| {
                let Some(split) = split.upgrade() else { return };
                if column == Column::Sidebar && split.chrome_active.get() {
                    split.sidebar_collapsed.set(collapsed);
                }
            }
        });

        split.nav.set_column_widths(&[
            ColumnWidth {
                preferred: Some(f64::from(sidebar_width.ideal())),
                minimum: Some(f64::from(sidebar_width.min())),
                maximum: Some(f64::from(sidebar_width.max())),
            },
            ColumnWidth {
                preferred: None,
                minimum: None,
                maximum: None,
            },
            ColumnWidth {
                preferred: None,
                minimum: None,
                maximum: None,
            },
        ]);

        host_leaf(mtm, split)
    }

    /// Wraps the split controller's view in a host view: the controller
    /// fills the host's bounds, and on a titled window the host registers it
    /// with the window's toolbar as the sidebar toggle. The coordinator is
    /// keyed per window, so attaching here shares the instance the
    /// navigation pages publish into. While the host is hidden inside a
    /// container the sidebar item collapses — an expanded one claims
    /// titlebar space even unseen — and the offer withdraws with it.
    fn host_leaf(mtm: MainThreadMarker, split: Rc<Split>) -> NativeLeaf {
        let host = HostView::new(mtm, Rect::ZERO);
        // An `NSSplitViewController` owns its columns' insets — the sidebar
        // runs the window's full height behind the unified toolbar — so the
        // leaf manages its own safe area and the root hands it the whole
        // window, not the safe-area rect (as the iOS leaf already does).
        host.set_manages_safe_area(true);
        let split_view = split.nav.view();
        host.set_primary_content_handler({
            let split_view = split_view.clone();
            move |_| Some(split_view.clone())
        });
        view::add_subview(&host, &split_view);
        host.set_layout_handler(|host| {
            let bounds = view::bounds(host);
            if let Some(sub) = view::subviews(host).first() {
                view::set_frame(sub, bounds);
            }
        });
        let offer_sidebar = {
            let split = split.clone();
            move |host: &HostView| {
                // A pane hidden inside a tab container contributes no
                // sidebar chrome — and must release the sidebar's claim on
                // the titlebar too: an expanded `allowsFullHeightLayout`
                // item reserves that space even while its view is hidden,
                // so the item collapses while the pane is hidden and the
                // wanted collapse state comes back when it shows again.
                let active = !view::is_hidden_in_hierarchy(host);
                split.chrome_active.set(active);
                split.apply_sidebar();
                let Some(window) = view::window(host) else {
                    return;
                };
                if !window.styleMask().contains(NSWindowStyleMask::Titled) {
                    return;
                }
                // The apply runs first so the toolbar's collapse-following
                // toggle label reads the restored state, not the forced
                // collapse the hidden pane carried.
                WindowToolbar::attached(&window).set_sidebar_split(if active {
                    Some(&split.nav)
                } else {
                    None
                });
            }
        };
        host.set_window_handler({
            let offer = offer_sidebar.clone();
            move |host| offer(host)
        });
        host.set_hidden_handler({
            let offer = offer_sidebar.clone();
            move |host, _hidden| offer(host)
        });
        let mut leaf = NativeLeaf::new(&*host, Fill);
        leaf.keep(split);
        leaf.keep(host);
        leaf
    }

    impl Split {
        /// Rebuilds the columns from the current selections and installs them
        /// (`set_columns` replaces the split items in place). The
        /// replacements arrive expanded, so the sidebar state is re-imposed
        /// afterwards: a selection change while the pane is hidden must not
        /// bring back an expanded sidebar, and a visible pane keeps its
        /// wanted collapse across the rebuild.
        fn mount(&self, keep: &mut KeepAlive) {
            // The sidebar leaf is already retained; the first column stays
            // mounted — only downstream columns rebuild.
            let middle = self.columns.renderer.render(self.columns.middle_view());
            let detail = self.columns.renderer.render(self.columns.detail_view());
            self.nav.set_columns(
                &self.sidebar,
                if self.columns.content.is_some() {
                    Some(middle.view())
                } else {
                    None
                },
                detail.view(),
            );
            self.apply_sidebar();
            keep.keep(middle);
            keep.keep(detail);
        }

        /// Writes the effective sidebar collapse: forced collapsed while
        /// the pane is hidden — an expanded full-height sidebar item
        /// claims titlebar space even with its view hidden — else the
        /// wanted `sidebar_collapsed`.
        fn apply_sidebar(&self) {
            self.nav.set_collapsed(
                Column::Sidebar,
                !self.chrome_active.get() || self.sidebar_collapsed.get(),
            );
        }
    }
}
