//! `NavigationSplitLayout` — the two/three-column adaptive container.
//!
//! iOS renders a `UISplitViewController` (double or triple column); each
//! column hosts a [`NavContentController`] wrapping the rendered subtree, so a
//! column's own `NavigationView` draws its standalone bar inside the column.
//! macOS renders the kit [`SplitViewController`] (`NSSplitViewController`).
//! Selection bindings rebuild the downstream columns; column visibility maps
//! to per-column collapse.

use core::cell::RefCell;

use crate::contract::NativeLeaf;
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
    /// The middle column's leaf: built from the primary selection (three
    /// columns) or the placeholder.
    fn middle(&self) -> NativeLeaf {
        let view = match (&self.content, self.primary.snapshot()) {
            (Some(content), Some(id)) => AnyView::new(content.build(id)),
            _ => self.placeholder.build(),
        };
        self.renderer.render(view)
    }

    /// The detail column's leaf: the primary selection drives two-column
    /// splits; the secondary selection drives three-column splits.
    fn detail(&self) -> NativeLeaf {
        let view = self
            .secondary
            .as_ref()
            .map_or_else(
                || self.primary.snapshot(),
                waterui::reactive::Signal::snapshot,
            )
            .map_or_else(
                || self.placeholder.build(),
                |id| AnyView::new(self.detail.build(id)),
            );
        self.renderer.render(view)
    }
}

#[cfg(target_os = "ios")]
mod platform {
    use super::{Columns, Fill};
    use alloc::rc::Rc;
    use core::cell::RefCell;

    use crate::contract::{NativeLeaf, RenderContext};
    use cocoa_ui::Retained;
    use cocoa_ui::geometry::Rect;
    use cocoa_ui::objc2_ui_kit::UISplitViewControllerColumn;
    use cocoa_ui::uikit::{HostView, NavContentController, SplitController};
    use cocoa_ui::view;
    use waterui::navigation::{NavigationSplitColumnVisibility, NavigationSplitLayout};

    use crate::contract::KeepAlive;

    /// `WuiSplitColumnPageController`: `UIKit` wraps each column in a nav
    /// controller whose bar reserves its height in the safe area even when
    /// it draws nothing, so the page hides the bar while it is the stack's
    /// base — a pushed page keeps it for the back affordance.
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
        mtm: cocoa_ui::MainThreadMarker,
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
        // SwiftUI collapses a NavigationSplitView onto its sidebar whatever
        // the selection — the destination's row stays highlighted and the
        // detail is where a tap goes from there.
        nav.set_collapsed_top_column(Some(UISplitViewControllerColumn::Primary));

        let sidebar_leaf = ctx.render(sidebar.build());
        let sidebar_vc = NavContentController::new(mtm, sidebar_leaf.view());
        hide_bar_when_base(&sidebar_vc);
        nav.set_column(UISplitViewControllerColumn::Primary, &sidebar_vc);

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
        let split = Rc::new(Split { columns, nav, mtm });

        if split.columns.content.is_some() {
            split.mount_middle(&mut keep);
        }
        split.mount_detail(&mut keep);
        *split.columns.mounted.borrow_mut() = keep;

        // Selection bindings rebuild the downstream columns.
        let primary = split.columns.primary.clone();
        split.columns.watchers.borrow_mut().bind(&primary, {
            let split = split.clone();
            move |_| {
                let mut keep = split.columns.mounted.borrow_mut();
                if split.columns.content.is_some() {
                    split.mount_middle(&mut keep);
                } else {
                    split.mount_detail(&mut keep);
                }
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
        fn mount_middle(&self, keep: &mut KeepAlive) {
            let leaf = self.columns.middle();
            let vc = NavContentController::new(self.mtm, leaf.view());
            hide_bar_when_base(&vc);
            self.nav
                .set_column(UISplitViewControllerColumn::Supplementary, &vc);
            keep.keep(leaf);
            keep.keep(vc);
        }

        fn mount_detail(&self, keep: &mut KeepAlive) {
            let leaf = self.columns.detail();
            let vc = NavContentController::new(self.mtm, leaf.view());
            hide_bar_when_base(&vc);
            self.nav
                .set_column(UISplitViewControllerColumn::Secondary, &vc);
            keep.keep(leaf);
            keep.keep(vc);
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{Columns, Fill};
    use alloc::rc::Rc;
    use core::cell::RefCell;

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

        let sidebar_leaf = ctx.render(sidebar.build());
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
        let nav_for_binds = nav.clone();
        let split = Rc::new(Split {
            columns,
            nav,
            sidebar: sidebar_view,
        });

        let mut keep = KeepAlive::default();
        keep.keep(sidebar_leaf);
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
            let nav = nav_for_binds.clone();
            move |visibility| {
                let (sidebar, content) = match visibility {
                    NavigationSplitColumnVisibility::All
                    | NavigationSplitColumnVisibility::Automatic => (false, false),
                    NavigationSplitColumnVisibility::DoubleColumn => (true, false),
                    NavigationSplitColumnVisibility::DetailOnly => (true, true),
                };
                nav.set_collapsed(Column::Sidebar, sidebar);
                nav.set_collapsed(Column::Supplementary, content);
            }
        });

        nav_for_binds.set_column_widths(&[
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
    /// navigation pages publish into.
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
            let nav = split.nav.clone();
            move |host: &HostView| {
                let Some(window) = view::window(host) else {
                    return;
                };
                if !window.styleMask().contains(NSWindowStyleMask::Titled) {
                    return;
                }
                // A pane hidden inside a tab container contributes no sidebar
                // chrome: claim only while the split is effectively visible.
                if view::is_hidden_in_hierarchy(host) {
                    WindowToolbar::attached(&window).set_sidebar_split(None);
                } else {
                    WindowToolbar::attached(&window).set_sidebar_split(Some(&nav));
                }
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
        /// (`set_columns` replaces the split items in place).
        fn mount(&self, keep: &mut KeepAlive) {
            // The sidebar leaf is already retained; the first column stays
            // mounted — only downstream columns rebuild.
            let middle = self.columns.middle();
            let detail = self.columns.detail();
            self.nav.set_columns(
                &self.sidebar,
                if self.columns.content.is_some() {
                    Some(middle.view())
                } else {
                    None
                },
                detail.view(),
            );
            keep.keep(middle);
            keep.keep(detail);
        }
    }
}
