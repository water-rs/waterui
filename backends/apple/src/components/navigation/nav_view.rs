//! A standalone `NavigationView` — a page with its own navigation chrome.
//!
//! When a `NavigationController` is in the environment a stack owns the
//! chrome and this leaf renders only the content (appear/disappear still
//! fire through window attachment). Otherwise the bar is drawn in content:
//! a `UINavigationBar` plus optional inline `UISearchBar` on iOS, a material
//! header on macOS — publishing into the window toolbar instead when the
//! window is titled.

use waterui::navigation::NavigationView;

use crate::dispatch::Dispatcher;

/// Installs the `NavigationView` handler.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<NavigationView>(platform::nav_view_leaf);
}

struct Fill;

impl waterui_core::layout::SubView for Fill {
    fn measure(
        &self,
        _proposal: waterui_core::layout::ProposalSize,
    ) -> waterui_core::layout::ViewDimensions {
        waterui_core::layout::ViewDimensions::new(waterui_core::layout::Size::new(0.0, 0.0))
    }

    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(target_os = "ios")]
mod platform {
    use core::mem;

    use cocoa_ui::uikit::{LargeTitle, NavBar, NavPage, SearchBar};
    use cocoa_ui::{Rect, view};
    use waterui::Str;
    use waterui::navigation::{NavigationController, NavigationTitleDisplayMode, NavigationView};
    use waterui_core::layout::ProposalSize;
    use waterui_core::reactive::Signal;

    use crate::components::navigation::bar::{
        BarItem, BarItemIcon, BarState, bar_color, bar_item_frame, search_prompt,
    };
    use crate::components::text::platform_color;
    use crate::contract::{KeepAlive, NativeLeaf, RenderContext};

    use super::Fill;
    use alloc::rc::Rc;
    use core::cell::RefCell;

    /// The standalone page: the content under an in-content `NavBar`; hosted
    /// pages render content only — the stack owns the chrome.
    #[allow(clippy::too_many_lines)]
    pub(super) fn nav_view_leaf(view: NavigationView, ctx: &RenderContext) -> NativeLeaf {
        let mut view = view;
        let env = ctx.env();
        let mtm = ctx.mtm();
        let content = ctx.render(mem::take(&mut view.content));
        let host = cocoa_ui::uikit::HostView::new(mtm, Rect::ZERO);
        view::add_subview(&host, content.view());
        let state = Rc::new(RefCell::new(view.state));
        let mut keep = KeepAlive::default();
        keep.keep(content);
        keep.keep(host.clone());
        keep.keep(state.clone());
        keep.keep(env.clone());

        host.set_window_handler({
            let env = env.clone();
            move |host| {
                let mut state = state.borrow_mut();
                if view::window(host).is_some() {
                    state.appeared(&env);
                } else {
                    state.disappeared(&env);
                }
            }
        });

        if env.get::<NavigationController>().is_some() {
            host.set_layout_handler(|host| {
                let bounds = view::bounds(host);
                if let Some(content) = view::subviews(host).first() {
                    view::set_frame(content, bounds);
                }
            });
            let mut leaf = NativeLeaf::new(&*host, Fill);
            leaf.keep(keep);
            return leaf;
        }

        let bar = BarState::new(&mut view.bar, ctx);
        let bar_view = NavBar::new(mtm);
        view::add_subview(&host, &bar_view);

        let search_bar = bar.search.as_ref().map(|search| {
            let field = SearchBar::new(mtm);
            field.set_change_handler({
                let text = search.text.clone();
                move |value| text.set(Str::from(value))
            });
            keep.bind(&search.text, {
                let field = field.clone();
                move |text: Str| field.set_text(text.as_ref())
            });
            keep.bind(&search_prompt(search, env), {
                let field = field.clone();
                move |prompt| field.set_placeholder(&prompt.to_plain())
            });
            view::add_subview(&host, &field);
            field
        });

        bar_view.set_page(&page(&bar, &mut keep));

        if let Some(color) = bar_color(&bar) {
            keep.bind(color, {
                let bar_view = bar_view.clone();
                move |color| {
                    bar_view.setBarTintColor(Some(&platform_color(&color)));
                }
            });
        }
        keep.bind(&bar.hidden, {
            let bar_view = bar_view.clone();
            let host = host.clone();
            move |hidden| {
                view::set_hidden(&bar_view, hidden);
                host.set_needs_layout();
            }
        });

        host.set_layout_handler({
            let hidden = bar.hidden.clone();
            move |host| {
                let bounds = view::bounds(host);
                let mut top = bounds.origin.y;
                if !hidden.snapshot() {
                    let size = bar_view
                        .sizeThatFits(cocoa_ui::geometry::Size::new(bounds.size.width, 0.0).into());
                    view::set_frame(
                        &bar_view,
                        Rect::new(bounds.origin.x, top, bounds.size.width, size.height),
                    );
                    top += size.height;
                }
                if let Some(field) = &search_bar {
                    let size = field
                        .sizeThatFits(cocoa_ui::geometry::Size::new(bounds.size.width, 0.0).into());
                    view::set_frame(
                        field,
                        Rect::new(bounds.origin.x, top, bounds.size.width, size.height),
                    );
                    top += size.height;
                }
                for sub in view::subviews(host) {
                    let bar_u: &cocoa_ui::objc2_ui_kit::UIView = &bar_view;
                    if *sub == *bar_u
                        || search_bar.as_ref().is_some_and(|field| {
                            let field_u: &cocoa_ui::objc2_ui_kit::UIView = field;
                            *sub == *field_u
                        })
                    {
                        continue;
                    }
                    view::set_frame(
                        &sub,
                        Rect::new(
                            bounds.origin.x,
                            top,
                            bounds.size.width,
                            bounds.size.height - (top - bounds.origin.y),
                        ),
                    );
                }
            }
        });

        let mut leaf = NativeLeaf::new(&*host, Fill);
        leaf.keep(keep);
        leaf
    }

    /// The standalone page chrome — no back affordance: there is nothing to
    /// pop without a controller in the environment.
    fn page(bar: &BarState, keep: &mut KeepAlive) -> NavPage {
        let title_view = bar.principal().map_or_else(
            || {
                if bar.title.text.is_some() {
                    None
                } else {
                    Some(cocoa_ui::view::retain_base(bar.title.leaf.view()))
                }
            },
            |item| Some(cocoa_ui::view::retain_base(item.leaf.view())),
        );
        NavPage {
            title: bar.title.text.clone().unwrap_or_default(),
            title_view,
            subtitle: bar.subtitle.text.clone(),
            leading: bar.leading_items().map(|item| button(item, keep)).collect(),
            trailing: bar
                .trailing_items()
                .map(|item| button(item, keep))
                .collect(),
            bottom: bar.bottom_items().map(|item| button(item, keep)).collect(),
            hides_back: true,
            on_back: None,
            search: None,
            large_title: match bar.display_mode {
                NavigationTitleDisplayMode::Large | NavigationTitleDisplayMode::Medium => {
                    LargeTitle::Large
                }
                NavigationTitleDisplayMode::Inline => LargeTitle::Inline,
                NavigationTitleDisplayMode::Automatic => LargeTitle::Automatic,
            },
            hidden: bar.hidden.snapshot(),
        }
    }

    /// A bar item: the rendered content's own control provides the action;
    /// a symbol icon becomes a `UIBarButtonItem` image; a view icon or a bare
    /// view is hosted as the item's custom view.
    fn button(
        item: &BarItem,
        keep: &mut KeepAlive,
    ) -> cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIBarButtonItem> {
        let mtm = cocoa_ui::MainThreadMarker::new().expect("main thread");
        let action = cocoa_ui::uikit::first_button(item.leaf.view()).map(|button| {
            alloc::rc::Rc::new(move || {
                button.sendActionsForControlEvents(
                    cocoa_ui::objc2_ui_kit::UIControlEvents::PrimaryActionTriggered,
                );
            }) as alloc::rc::Rc<dyn Fn()>
        });
        let button = match &item.icon {
            Some(BarItemIcon::System(symbol)) => {
                cocoa_ui::uikit::bar_item(mtm, Some(symbol), None, action)
            }
            Some(BarItemIcon::View(icon)) => {
                // The declared icon draws inside the bar's chrome as a
                // template image the bar tints; a view that cannot render
                // stays hosted as the item's custom view.
                let size = icon
                    .layout()
                    .measure(ProposalSize {
                        width: None,
                        height: None,
                    })
                    .size;
                cocoa_ui::view::set_frame(
                    icon.view(),
                    cocoa_ui::geometry::Rect::new(
                        0.0,
                        0.0,
                        f64::from(size.width),
                        f64::from(size.height),
                    ),
                );
                if let Some(image) = cocoa_ui::bitmap::view_template_image(icon.view(), 24.0) {
                    cocoa_ui::uikit::image_bar_item(mtm, Some(&image), action)
                } else {
                    cocoa_ui::view::set_frame(item.leaf.view(), bar_item_frame(item));
                    cocoa_ui::uikit::bar_item(mtm, None, Some(item.leaf.view()), action)
                }
            }
            None => {
                // A hosted item must arrive with a real frame: the bar wraps
                // it as the item's customView and never lays it out itself.
                cocoa_ui::view::set_frame(item.leaf.view(), bar_item_frame(item));
                cocoa_ui::uikit::bar_item(mtm, None, Some(item.leaf.view()), None)
            }
        };
        if let Some(title) = &item.title {
            keep.bind(title, {
                let button = button.clone();
                move |title| {
                    cocoa_ui::uikit::bar_item_accessibility_label(&button, &title.to_plain());
                }
            });
        }
        button
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use core::mem;

    use cocoa_ui::appkit::{
        HostView, HostedItem, HostedSearch, Label, SearchField, ToolbarChild, ToolbarContent,
        WindowToolbar, first_button, header_material_view, symbol_image,
    };
    use cocoa_ui::objc2_app_kit::NSWindowStyleMask;
    use cocoa_ui::text::WrapWidth;
    use cocoa_ui::{Rect, Retained, view};
    use waterui::Str;
    use waterui::navigation::{NavigationController, NavigationToolbarPlacement, NavigationView};
    use waterui::reactive::Signal;
    use waterui_core::layout::ProposalSize;

    use crate::components::navigation::bar::{
        BarItem, BarItemIcon, BarState, bar_color, bar_item_frame, search_prompt,
    };
    use crate::components::text::platform_color;
    use crate::contract::{KeepAlive, NativeLeaf, RenderContext};

    use super::Fill;
    use alloc::rc::Rc;
    use core::cell::RefCell;

    /// The standalone page on macOS: a material header under non-titled
    /// windows; the window toolbar when the window is titled.
    #[allow(clippy::too_many_lines)]
    pub(super) fn nav_view_leaf(view: NavigationView, ctx: &RenderContext) -> NativeLeaf {
        let mut view = view;
        let env = ctx.env();
        let mtm = ctx.mtm();
        let content = ctx.render(mem::take(&mut view.content));
        let host = HostView::new(mtm, Rect::ZERO);
        view::add_subview(&host, content.view());
        let state = Rc::new(RefCell::new(view.state));
        let mut keep = KeepAlive::default();
        keep.keep(content);
        keep.keep(host.clone());
        keep.keep(state);
        keep.keep(env.clone());

        if env.get::<NavigationController>().is_some() {
            host.set_layout_handler(|host| {
                let bounds = view::bounds(host);
                if let Some(content) = view::subviews(host).first() {
                    view::set_frame(content, bounds);
                }
            });
            let mut leaf = NativeLeaf::new(&*host, Fill);
            leaf.keep(keep);
            return leaf;
        }

        let bar = Rc::new(BarState::new(&mut view.bar, ctx));

        // In-content chrome: a material header strip with the title and the
        // hosted item views; replaced by the window toolbar on titled
        // windows.
        let header = header_material_view(mtm);
        view::add_subview(&host, &header);
        let title = Label::new(mtm);
        if let Some(text) = &bar.title.text {
            title.set_text(text);
        }
        header.addSubview(&title);
        for item in &bar.items {
            header.addSubview(item.leaf.view());
        }
        let search_field = bar.search.as_ref().map(|search| {
            let field = SearchField::new(mtm);
            field.set_change_handler({
                let text = search.text.clone();
                move |field| text.set(Str::from(field.text()))
            });
            keep.bind(&search.text, {
                let field = field.clone();
                move |text: Str| field.set_text(text.as_ref())
            });
            keep.bind(&search_prompt(search, env), {
                let field = field.clone();
                move |prompt| field.set_placeholder(prompt.to_plain().as_ref())
            });
            header.addSubview(&field);
            field
        });
        if let Some(color) = bar_color(&bar) {
            keep.bind(color, {
                let header = header.clone();
                move |color| {
                    cocoa_ui::appkit::set_material_background(
                        &header,
                        Some(&platform_color(&color)),
                    );
                }
            });
        }
        keep.bind(&bar.hidden, {
            let host = host.clone();
            let header = header.clone();
            move |hidden| {
                view::set_hidden(&header, hidden);
                host.set_needs_layout();
            }
        });

        // A second copy for the toolbar publisher: the layout handler below
        // captures the field for the in-content header.
        let field_for_toolbar = search_field.clone();
        host.set_layout_handler({
            let bar = bar.clone();
            let header = header.clone();
            move |host| {
                let title = &title;
                let search_field = &search_field;
                let bounds = view::bounds(host);
                // The in-content bar reserves space only while it is shown:
                // a hidden bar and one promoted into the window toolbar get
                // the same zero-height layout.
                let bar_height = if view::is_hidden(&header) { 0.0 } else { 52.0 };
                if bar_height > 0.0 {
                    let y = bounds.origin.y + bounds.size.height - bar_height;
                    view::set_frame(
                        &header,
                        Rect::new(bounds.origin.x, y, bounds.size.width, bar_height),
                    );
                    let title_size = title.measure(WrapWidth::Free).size;
                    view::set_frame(
                        title,
                        Rect::new(
                            bounds.origin.x + (bounds.size.width - title_size.width) / 2.0,
                            y + (bar_height - title_size.height) / 2.0,
                            title_size.width,
                            title_size.height,
                        ),
                    );
                    let mut left = bounds.origin.x + 12.0;
                    let mut right = bounds.origin.x + bounds.size.width - 12.0;
                    for item in &bar.items {
                        let size = bar_item_frame(item).size;
                        let leading = matches!(
                            item.placement,
                            NavigationToolbarPlacement::Cancellation
                                | NavigationToolbarPlacement::TopBarLeading
                        );
                        let x = if leading {
                            let x = left;
                            left += size.width + 8.0;
                            x
                        } else {
                            right -= size.width;
                            right
                        };
                        if !leading {
                            right -= 8.0;
                        }
                        view::set_frame(
                            item.leaf.view(),
                            Rect::new(
                                x,
                                y + (bar_height - size.height) / 2.0,
                                size.width,
                                size.height,
                            ),
                        );
                    }
                    if let Some(field) = &search_field {
                        let width = 220.0_f64.min(bounds.size.width / 3.0);
                        view::set_frame(
                            field,
                            Rect::new(right - width, y + (bar_height - 28.0) / 2.0, width, 28.0),
                        );
                    }
                }
                for sub in view::subviews(host) {
                    if **sub == ***header {
                        continue;
                    }
                    view::set_frame(
                        &sub,
                        Rect::new(
                            bounds.origin.x,
                            bounds.origin.y,
                            bounds.size.width,
                            bounds.size.height - bar_height,
                        ),
                    );
                }
            }
        });

        // Titled windows: publish the chrome into the window toolbar and hide
        // the in-content header — while the bar is visible and the view is
        // effectively shown; a hidden bar or a pane hidden inside a container
        // withdraws.
        let toolbar = Rc::new(RefCell::new(
            Option::<cocoa_ui::Retained<WindowToolbar>>::None,
        ));
        let publish_bar = {
            let bar = bar.clone();
            let host_weak = host.clone();
            let toolbar = toolbar.clone();
            let search_field = field_for_toolbar;
            move |host: &HostView| {
                let Some(window) = view::window(host) else {
                    return;
                };
                let titled = window.styleMask().contains(NSWindowStyleMask::Titled);
                let mut slot = toolbar.borrow_mut();
                if !titled {
                    if let Some(attached) = slot.take() {
                        attached.clear_content(Rc::as_ptr(&bar) as usize);
                        view::set_hidden(&header, false);
                        host_weak.set_needs_layout();
                    }
                    return;
                }
                if slot.is_none() {
                    *slot = Some(WindowToolbar::attached(&window));
                }
                let owner = Rc::as_ptr(&bar) as usize;
                if bar.hidden.snapshot() || view::is_hidden_in_hierarchy(host) {
                    slot.as_ref().expect("attached").clear_content(owner);
                    return;
                }
                slot.as_ref()
                    .expect("attached")
                    .set_content(toolbar_content(&bar, search_field.as_ref()), owner);
                view::set_hidden(&header, true);
                host_weak.set_needs_layout();
            }
        };
        host.set_window_handler({
            let publish = publish_bar.clone();
            move |host| publish(host)
        });
        host.set_hidden_handler({
            let publish = publish_bar.clone();
            move |host, _hidden| publish(host)
        });
        keep.watch(&bar.hidden, {
            let publish = publish_bar;
            let host = host.clone();
            move |_| publish(host.as_ref())
        });
        keep.keep(toolbar);

        let mut leaf = NativeLeaf::new(&*host, Fill);
        leaf.keep(keep);
        leaf.keep(bar);
        leaf
    }

    /// The toolbar contribution of the standalone bar.
    fn toolbar_content(
        bar: &Rc<BarState>,
        search_field: Option<&Retained<SearchField>>,
    ) -> ToolbarContent {
        ToolbarContent {
            shows_back: false,
            on_back: None,
            title: bar.title.text.clone(),
            title_item: if bar.title.text.is_some() {
                None
            } else {
                let size = bar.title.leaf.layout().measure(ProposalSize {
                    width: None,
                    height: None,
                });
                Some(HostedItem {
                    view: cocoa_ui::view::retain_base(bar.title.leaf.view()),
                    size: cocoa_ui::Size::new(
                        f64::from(size.size.width),
                        f64::from(size.size.height),
                    ),
                })
            },
            leading: bar.leading().map(child),
            trailing: bar.trailing().map(child),
            status: bar.status().map(|item| HostedItem {
                view: cocoa_ui::view::retain_base(item.leaf.view()),
                size: bar_item_frame(item).size,
            }),
            // The field is shared with the in-content header, which the
            // toolbar hides — rehosting it here keeps one field, its text
            // and its focus.
            search: search_field.map(|field| HostedSearch {
                field: field.clone(),
                source_id: Rc::as_ptr(bar) as usize,
            }),
        }
    }

    /// A semantic item as a toolbar child: the icon in a capsule, the action
    /// forwarded to the content's own button.
    fn child(item: &BarItem) -> ToolbarChild {
        let button = first_button(item.leaf.view());
        let action = button.as_ref().map(|button| {
            let button = button.clone();
            alloc::rc::Rc::new(move || {
                // SAFETY: `activate` on a control the same leaf owns
                // is the documented way to fire its action.
                unsafe { cocoa_ui::appkit::activate(button.control()) }
            }) as alloc::rc::Rc<dyn Fn()>
        });
        let bordered = button.as_ref().is_none_or(|button| !button.is_borderless());
        ToolbarChild {
            view: HostedItem {
                view: cocoa_ui::view::retain_base(item.leaf.view()),
                size: bar_item_frame(item).size,
            },
            icon: match &item.icon {
                Some(BarItemIcon::System(symbol)) => symbol_image(symbol),
                Some(BarItemIcon::View(icon)) => {
                    let size = icon
                        .layout()
                        .measure(ProposalSize {
                            width: None,
                            height: None,
                        })
                        .size;
                    cocoa_ui::view::set_frame(
                        icon.view(),
                        Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
                    );
                    cocoa_ui::bitmap::view_template_image(icon.view(), 18.0)
                }
                None => None,
            },
            label: item
                .title
                .as_ref()
                .map_or_else(String::new, |title| title.snapshot().to_plain().to_string()),
            bordered,
            action,
        }
    }
}
