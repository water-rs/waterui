//! `TabsLayout` — the adaptive tab container.
//!
//! iOS renders a `UITabBarController` (kit [`TabsController`]); macOS renders a
//! sidebar source list for `Sidebar` style and a segmented control above the
//! content otherwise. Each tab's `NavigationView` renders through the
//! navigation handler, so it draws its own standalone bar inside the pane.
//! The selection binding is two-way: native selection writes back, external
//! writes select the matching tab.

use alloc::vec::Vec;

use crate::contract::{NativeLeaf, RenderContext};
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
/// reactive `Tab` fields a spec bind still needs.
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
}

/// Renders every tab once and returns the mounted set.
fn mount_tabs(tabs: Vec<Tab<Id>>, ctx: &RenderContext) -> Vec<Mounted> {
    tabs.into_iter()
        .map(|mut tab| {
            let pane = ctx.render(waterui_backend_core::AnyView::new(tab.content.build()));
            let label_leaf = ctx.render(core::mem::replace(
                &mut tab.label,
                waterui_backend_core::AnyView::new(()),
            ));
            let label = extract_title_text(label_leaf.view());
            let (symbol, icon_leaf) = match tab.icon {
                Some(TabIcon::System(icon)) => (Some(icon.name.to_string()), None),
                Some(TabIcon::View(icon)) => (None, Some(ctx.render(icon.build()))),
                None => (None, None),
            };

            Mounted {
                id: tab.id,
                pane,
                label_leaf,
                label: label.unwrap_or_default(),
                symbol,
                icon_leaf,
                badge: tab.badge,
                enabled: tab.enabled,
                is_search: matches!(tab.role, TabRole::Search),
            }
        })
        .collect()
}

#[cfg(target_os = "ios")]
mod platform {
    use super::{Fill, Mounted, mount_tabs};
    use alloc::rc::Rc;
    use alloc::vec::Vec;

    use crate::contract::{NativeLeaf, RenderContext};
    use cocoa_ui::geometry::Rect;
    use cocoa_ui::uikit::{HostView, TabSpec, TabsController};
    use cocoa_ui::{Retained, view};
    use waterui::navigation::TabsLayout;
    use waterui::reactive::Signal;
    use waterui_core::layout::ProposalSize;

    use crate::contract::KeepAlive;

    /// `UITabBarController` with one `UIViewController` per pane.
    #[allow(clippy::too_many_lines)]
    pub(super) fn tabs_leaf(mut layout: TabsLayout, ctx: &RenderContext) -> NativeLeaf {
        let mtm = ctx.mtm();
        let tabs = TabsController::new(mtm);
        let mut keep = KeepAlive::default();
        let mounted = Rc::new(mount_tabs(core::mem::take(&mut layout.tabs), ctx));

        let specs: Vec<TabSpec> = mounted.iter().map(spec).collect();
        tabs.set_tabs(
            &specs,
            &mounted
                .iter()
                .map(|tab| Retained::from(tab.pane.view()))
                .collect::<Vec<_>>(),
        );

        // A declared icon view draws inside the tab item as a template image
        // — the baseline's `installIconViews`, capped at the same 25pt side.
        for (index, tab) in mounted.iter().enumerate() {
            let Some(icon_leaf) = &tab.icon_leaf else {
                continue;
            };
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
            if let Some(image) = cocoa_ui::bitmap::view_template_image(icon_leaf.view(), 25.0) {
                tabs.tab_item(index).setImage(Some(&image));
            }
        }

        // Reactive chrome mutates each `UITabBarItem` in place — rebuilding
        // the controllers re-parents pane views UIKit still owns.
        for (index, tab) in mounted.iter().enumerate() {
            if let Some(badge) = &tab.badge {
                keep.bind(badge, {
                    let item = tabs.tab_item(index);
                    move |value| {
                        let badge = (value > 0)
                            .then(|| objc2_foundation::NSString::from_str(&value.to_string()));
                        item.setBadgeValue(badge.as_deref());
                    }
                });
            }
            keep.bind(&tab.enabled, {
                let item = tabs.tab_item(index);
                move |enabled| {
                    item.setEnabled(enabled);
                }
            });
        }

        // Two-way selection.
        tabs.set_select_handler({
            let selection = layout.selection.clone();
            let mounted = mounted.clone();
            move |index| {
                if let Some(tab) = mounted.get(index) {
                    selection.set(tab.id);
                }
            }
        });
        keep.bind(&layout.selection, {
            let tabs = tabs.clone();
            let mounted = mounted.clone();
            move |id| {
                if let Some(index) = mounted.iter().position(|tab| tab.id == id) {
                    tabs.select(index);
                }
            }
        });

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
    use super::{Fill, mount_tabs};
    use alloc::rc::Rc;
    use alloc::vec::Vec;

    use crate::contract::{NativeLeaf, RenderContext};
    use cocoa_ui::appkit::{HostView, Segment, SegmentedControl, SourceList, WindowToolbar};
    use cocoa_ui::objc2_app_kit::NSWindowStyleMask;
    use cocoa_ui::{Rect, view};
    use waterui::navigation::{TabsLayout, tab::NativeTabStyle};
    use waterui::reactive::Signal;

    use crate::contract::KeepAlive;

    /// macOS: `Sidebar` style gets a source-list column; everything else a
    /// segmented control strip above the selected pane.
    #[allow(clippy::too_many_lines)]
    pub(super) fn tabs_leaf(mut layout: TabsLayout, ctx: &RenderContext) -> NativeLeaf {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mut keep = KeepAlive::default();
        let mounted = Rc::new(mount_tabs(core::mem::take(&mut layout.tabs), ctx));
        keep.keep(mounted.clone());
        for tab in mounted.iter() {
            view::add_subview(&host, tab.pane.view());
        }

        let sidebar = matches!(layout.style, NativeTabStyle::Sidebar);
        let chrome = if sidebar {
            let list = SourceList::new(mtm);
            list.set_segments(
                &mounted
                    .iter()
                    .map(|tab| Segment {
                        label: tab.label.clone(),
                        symbol: tab.symbol.clone(),
                        image: None,
                        enabled: tab.enabled.snapshot(),
                        badge: tab.badge.as_ref().map(|badge| badge.snapshot().to_string()),
                    })
                    .collect::<Vec<_>>(),
            );
            view::add_subview(&host, list.view());
            list.set_select_handler({
                let selection = layout.selection.clone();
                let mounted = mounted.clone();
                move |index| {
                    if let Some(tab) = mounted.get(index) {
                        selection.set(tab.id);
                    }
                }
            });
            Chrome::List(Rc::new(list))
        // (control path adds via add_subview below)
        } else {
            let control = SegmentedControl::new(mtm);
            control.set_segments(
                &mounted
                    .iter()
                    .map(|tab| Segment {
                        label: tab.label.clone(),
                        symbol: tab.symbol.clone(),
                        image: None,
                        enabled: tab.enabled.snapshot(),
                        badge: tab.badge.as_ref().map(|badge| badge.snapshot().to_string()),
                    })
                    .collect::<Vec<_>>(),
            );
            view::add_subview(&host, &control);
            control.set_select_handler({
                let selection = layout.selection.clone();
                let mounted = mounted.clone();
                move |index| {
                    if let Some(tab) = mounted.get(index) {
                        selection.set(tab.id);
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
        let selected = layout.selection.snapshot();
        show(mounted.iter().position(|tab| tab.id == selected));

        keep.bind(&layout.selection, {
            let show = show.clone();
            let mounted = mounted.clone();
            move |id| {
                show(mounted.iter().position(|tab| tab.id == id));
            }
        });

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
                let bounds = cocoa_ui::view::bounds(host);
                if sidebar {
                    let width = 220.0_f64.min(bounds.size.width / 2.0);
                    view::set_frame(
                        chrome.view(),
                        Rect::new(bounds.origin.x, bounds.origin.y, width, bounds.size.height),
                    );
                    for tab in mounted.iter() {
                        view::set_frame(
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
                    for tab in mounted.iter() {
                        view::set_frame(tab.pane.view(), bounds);
                    }
                } else {
                    let height = 28.0_f64;
                    view::set_frame(
                        chrome.view(),
                        Rect::new(
                            bounds.origin.x + 8.0,
                            bounds.origin.y + bounds.size.height - height - 8.0,
                            bounds.size.width - 16.0,
                            height,
                        ),
                    );
                    for tab in mounted.iter() {
                        view::set_frame(
                            tab.pane.view(),
                            Rect::new(
                                bounds.origin.x,
                                bounds.origin.y,
                                bounds.size.width,
                                bounds.size.height - height - 8.0,
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
