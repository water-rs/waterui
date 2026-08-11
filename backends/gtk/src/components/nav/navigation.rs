//! GTK4 Navigation components implementation.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use nami::{Signal, SignalExt};
use waterui_controls::text_field::TextField;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_core::{AnyView, Environment};
use waterui_navigation::{
    Bar, CustomNavigationController, NativeNavigationTransition, NavigationController,
    NavigationSplitLayout, NavigationStack, NavigationToolbarPlacement, NavigationTransaction,
    NavigationView, navigation_back_label, resolve_navigation_root,
};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::{ScopedCss, resolved_color_to_css_rgba, store_watcher_guards};

fn css_for_header_bar_color(color: &waterui_graphics::color::Color, env: &Environment) -> String {
    let resolved = color.resolve(env).get();
    format!(
        "background-color: {};",
        resolved_color_to_css_rgba(resolved)
    )
}

fn clear_box_children(container: &gtk4::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn render_search_widget(
    renderer: &mut GtkRenderer,
    env: &Environment,
    search: &waterui_navigation::NavigationSearch,
) -> gtk4::Widget {
    renderer
        .render(
            TextField::new(&search.text).prompt(search.prompt.clone()),
            env,
        )
        .upcast()
}

struct RenderedNavigationBar {
    title: gtk4::Widget,
    leading: Option<gtk4::Widget>,
    trailing: Option<gtk4::Widget>,
    bottom: Option<gtk4::Widget>,
    search: Option<gtk4::Widget>,
    color: Option<nami::Computed<waterui_graphics::color::Color>>,
    hidden: nami::Computed<bool>,
}

fn optional_box_widget(container: gtk4::Box) -> Option<gtk4::Widget> {
    container.first_child().map(|_| container.upcast())
}

fn render_navigation_bar(
    bar: Bar,
    env: &Environment,
    renderer: &mut GtkRenderer,
) -> RenderedNavigationBar {
    let title_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    title_box.append(&renderer.render_any(bar.title, env));
    if !bar.subtitle.is::<()>() {
        title_box.append(&renderer.render_any(bar.subtitle, env));
    }

    let principal = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let leading = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let trailing = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    bottom.set_halign(gtk4::Align::Center);
    for item in bar.toolbar.items {
        let widget = renderer.render_any(item.content, env);
        match item.placement {
            NavigationToolbarPlacement::Principal => principal.append(&widget),
            NavigationToolbarPlacement::Cancellation
            | NavigationToolbarPlacement::TopBarLeading => leading.append(&widget),
            NavigationToolbarPlacement::BottomBar | NavigationToolbarPlacement::Status => {
                bottom.append(&widget);
            }
            NavigationToolbarPlacement::PrimaryAction
            | NavigationToolbarPlacement::SecondaryAction
            | NavigationToolbarPlacement::Confirmation
            | NavigationToolbarPlacement::TopBarTrailing => trailing.append(&widget),
        }
    }
    let title = if principal.first_child().is_some() {
        principal.upcast()
    } else {
        title_box.upcast()
    };
    RenderedNavigationBar {
        title,
        leading: optional_box_widget(leading),
        trailing: optional_box_widget(trailing),
        bottom: optional_box_widget(bottom),
        search: bar
            .search
            .as_ref()
            .map(|search| render_search_widget(renderer, env, search)),
        color: bar.color,
        hidden: bar.hidden,
    }
}

impl GtkComponent for NavigationView {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        if env.get::<NavigationController>().is_some() {
            let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            container.set_hexpand(true);
            container.set_vexpand(true);
            let content_widget = renderer.render_any(self.content, env);
            container.append(&content_widget);
            return container.upcast();
        }

        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        let header_bar = gtk4::HeaderBar::new();
        let scoped_css = ScopedCss::attach(
            &header_bar,
            "waterui-navigation-headerbar",
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let leading_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let trailing_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        header_bar.pack_start(&leading_box);
        header_bar.pack_end(&trailing_box);

        let bar = render_navigation_bar(self.bar, env, renderer);
        header_bar.set_title_widget(Some(&bar.title));
        if let Some(leading) = &bar.leading {
            leading_box.append(leading);
        }
        if let Some(trailing) = &bar.trailing {
            trailing_box.append(trailing);
        }

        let hidden_guard = bar.hidden.watch({
            let header_bar = header_bar.clone();
            move |ctx: nami::watcher::Context<bool>| {
                let hidden = ctx.into_value();
                let header_bar = header_bar.clone();
                glib::idle_add_local_once(move || {
                    header_bar.set_visible(!hidden);
                });
            }
        });

        let env_for_color = env.clone();
        let scoped_css_for_color = scoped_css.clone();
        let color_guard = bar.color.as_ref().map(|color| {
            color.watch(
                move |ctx: nami::watcher::Context<waterui_graphics::color::Color>| {
                    let color = ctx.into_value();
                    let declarations = css_for_header_bar_color(&color, &env_for_color);
                    let scoped_css = scoped_css_for_color.clone();
                    glib::idle_add_local_once(move || {
                        scoped_css.set_declarations(&declarations);
                    });
                },
            )
        });

        if bar.hidden.get() {
            header_bar.set_visible(false);
        }
        if let Some(color) = &bar.color {
            scoped_css.set_declarations(&css_for_header_bar_color(&color.get(), env));
        }

        container.append(&header_bar);
        if let Some(search_widget) = bar.search {
            search_widget.set_margin_top(6);
            search_widget.set_margin_bottom(6);
            search_widget.set_margin_start(12);
            search_widget.set_margin_end(12);
            container.append(&search_widget);
        }

        let content_widget = renderer.render_any(self.content, env);
        container.append(&content_widget);
        if let Some(bottom) = bar.bottom {
            container.append(&bottom);
        }

        let mut guards = vec![hidden_guard];
        guards.extend(color_guard);
        store_watcher_guards(&container, guards);
        container.upcast()
    }
}

impl GtkComponent for NavigationStack<(), ()> {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let transition = self.transition_style().native();
        let root = self.into_inner();

        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        let bar_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.append(&bar_container);

        let header_bar = gtk4::HeaderBar::new();
        let scoped_css = ScopedCss::attach(
            &header_bar,
            "waterui-navigation-headerbar",
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        bar_container.append(&header_bar);

        let back_button = gtk4::Button::with_label(
            navigation_back_label().resolve(env).accessibility_label().get().to_plain().as_str(),
        );
        back_button.set_visible(false);
        let leading_slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let trailing_slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let search_holder = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        leading_slot.append(&back_button);
        header_bar.pack_start(&leading_slot);
        header_bar.pack_end(&trailing_slot);
        bar_container.append(&search_holder);

        let gtk_stack = gtk4::Stack::new();
        gtk_stack.set_hexpand(true);
        gtk_stack.set_vexpand(true);
        match transition {
            NativeNavigationTransition::Automatic | NativeNavigationTransition::Zoom(_) => {
                gtk_stack.set_transition_type(gtk4::StackTransitionType::SlideLeftRight);
                gtk_stack.set_transition_duration(250);
            }
            NativeNavigationTransition::Fade => {
                gtk_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
                gtk_stack.set_transition_duration(250);
            }
            NativeNavigationTransition::None | NativeNavigationTransition::Custom => {
                gtk_stack.set_transition_type(gtk4::StackTransitionType::None);
                gtk_stack.set_transition_duration(0);
            }
        }
        container.append(&gtk_stack);
        let bottom_holder = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.append(&bottom_holder);

        let mut child_env = env.clone();
        let controller = GtkNavigationController::new(
            gtk_stack.clone(),
            bar_container,
            header_bar,
            leading_slot,
            trailing_slot,
            search_holder,
            bottom_holder,
            back_button.clone(),
            scoped_css,
            &child_env,
        );
        let navigation_controller = NavigationController::new(controller.clone());
        child_env.insert(navigation_controller.clone());
        controller.set_env(child_env.clone());

        back_button.connect_clicked({
            let navigation_controller = navigation_controller;
            move |_| {
                navigation_controller.request_pop(1);
            }
        });

        let NavigationView { bar, content, .. } = resolve_navigation_root(root, &child_env);
        let bar = render_navigation_bar(bar, &child_env, renderer);
        controller.set_root_bar_state(
            bar.title,
            bar.leading,
            bar.trailing,
            bar.bottom,
            bar.search,
            bar.color,
            bar.hidden,
        );
        let root_widget = renderer.render_any(content, &child_env);
        gtk_stack.add_named(&root_widget, Some("root"));
        gtk_stack.set_visible_child_name("root");

        container.upcast()
    }
}

#[derive(Clone)]
struct GtkNavigationController {
    inner: Rc<RefCell<GtkNavigationControllerInner>>,
}

struct GtkNavigationControllerInner {
    stack: gtk4::Stack,
    bar_container: gtk4::Box,
    header_bar: gtk4::HeaderBar,
    leading_slot: gtk4::Box,
    trailing_slot: gtk4::Box,
    search_holder: gtk4::Box,
    bottom_holder: gtk4::Box,
    back_button: gtk4::Button,
    color_css: ScopedCss,
    view_stack: Vec<NavigationViewState>,
    active_bar_guards: Vec<nami::watcher::BoxWatcherGuard>,
    next_id: usize,
    env: Environment,
}

struct NavigationViewState {
    id: String,
    title_widget: Option<gtk4::Widget>,
    leading_widget: Option<gtk4::Widget>,
    trailing_widget: Option<gtk4::Widget>,
    bottom_widget: Option<gtk4::Widget>,
    search_widget: Option<gtk4::Widget>,
    bar_color: Option<nami::Computed<waterui_graphics::color::Color>>,
    bar_hidden: Option<nami::Computed<bool>>,
}

impl GtkNavigationController {
    #[allow(clippy::too_many_arguments)]
    fn new(
        stack: gtk4::Stack,
        bar_container: gtk4::Box,
        header_bar: gtk4::HeaderBar,
        leading_slot: gtk4::Box,
        trailing_slot: gtk4::Box,
        search_holder: gtk4::Box,
        bottom_holder: gtk4::Box,
        back_button: gtk4::Button,
        color_css: ScopedCss,
        env: &Environment,
    ) -> Self {
        Self {
            inner: Rc::new(RefCell::new(GtkNavigationControllerInner {
                stack,
                bar_container,
                header_bar,
                leading_slot,
                trailing_slot,
                search_holder,
                bottom_holder,
                back_button,
                color_css,
                view_stack: vec![NavigationViewState {
                    id: "root".to_string(),
                    title_widget: None,
                    leading_widget: None,
                    trailing_widget: None,
                    bottom_widget: None,
                    search_widget: None,
                    bar_color: None,
                    bar_hidden: None,
                }],
                active_bar_guards: Vec::new(),
                next_id: 0,
                env: env.clone(),
            })),
        }
    }

    fn set_env(&self, env: Environment) {
        self.inner.borrow_mut().env = env;
    }

    #[allow(clippy::too_many_arguments)]
    fn set_root_bar_state(
        &self,
        title_widget: gtk4::Widget,
        leading_widget: Option<gtk4::Widget>,
        trailing_widget: Option<gtk4::Widget>,
        bottom_widget: Option<gtk4::Widget>,
        search_widget: Option<gtk4::Widget>,
        bar_color: Option<nami::Computed<waterui_graphics::color::Color>>,
        bar_hidden: nami::Computed<bool>,
    ) {
        let mut inner = self.inner.borrow_mut();
        inner.view_stack[0].title_widget = Some(title_widget);
        inner.view_stack[0].leading_widget = leading_widget;
        inner.view_stack[0].trailing_widget = trailing_widget;
        inner.view_stack[0].bottom_widget = bottom_widget;
        inner.view_stack[0].search_widget = search_widget;
        inner.view_stack[0].bar_color = bar_color;
        inner.view_stack[0].bar_hidden = Some(bar_hidden);
        inner.apply_active_bar_for_top();
    }
}

impl CustomNavigationController for GtkNavigationController {
    fn apply(&mut self, transaction: NavigationTransaction) {
        let NavigationTransaction {
            retained_prefix,
            removed,
            inserted,
            ..
        } = transaction;
        let mut inner = self.inner.borrow_mut();
        assert_eq!(
            retained_prefix + removed + 1,
            inner.view_stack.len(),
            "GTK navigation transaction must replace the current suffix"
        );
        for _ in 0..removed {
            inner.pop_internal();
        }
        for content in inserted {
            inner.push(content.build());
        }
    }
}

impl GtkNavigationControllerInner {
    fn push(&mut self, content: NavigationView) {
        let id = format!("view_{}", self.next_id);
        self.next_id += 1;

        let mut renderer = GtkRenderer::new();
        let content_widget = renderer.render_any(content.content, &self.env);
        let view_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        view_container.set_hexpand(true);
        view_container.set_vexpand(true);
        view_container.append(&content_widget);
        self.stack.add_named(&view_container, Some(&id));
        self.stack.set_visible_child_name(&id);

        let bar = render_navigation_bar(content.bar, &self.env, &mut renderer);

        self.view_stack.push(NavigationViewState {
            id,
            title_widget: Some(bar.title),
            leading_widget: bar.leading,
            trailing_widget: bar.trailing,
            bottom_widget: bar.bottom,
            search_widget: bar.search,
            bar_color: bar.color,
            bar_hidden: Some(bar.hidden),
        });
        self.apply_active_bar_for_top();
    }
    fn pop_internal(&mut self) {
        if self.view_stack.len() <= 1 {
            return;
        }

        if let Some(current) = self.view_stack.pop()
            && let Some(child) = self.stack.child_by_name(&current.id)
        {
            self.stack.remove(&child);
        }

        if let Some(previous) = self.view_stack.last() {
            self.stack.set_visible_child_name(&previous.id);
        }

        self.apply_active_bar_for_top();
    }

    fn apply_active_bar_for_top(&mut self) {
        self.active_bar_guards.clear();

        let is_root = self.view_stack.len() <= 1;
        self.back_button.set_visible(!is_root);

        let Some(top) = self.view_stack.last() else {
            return;
        };

        clear_box_children(&self.leading_slot);
        clear_box_children(&self.trailing_slot);
        clear_box_children(&self.search_holder);
        clear_box_children(&self.bottom_holder);
        self.leading_slot.append(&self.back_button);

        if let Some(title) = &top.title_widget {
            self.header_bar.set_title_widget(Some(title));
        } else {
            self.header_bar.set_title_widget(None::<&gtk4::Widget>);
        }
        if let Some(leading) = &top.leading_widget {
            self.leading_slot.append(leading);
        }
        if let Some(trailing) = &top.trailing_widget {
            self.trailing_slot.append(trailing);
        }
        if let Some(search) = &top.search_widget {
            search.set_margin_top(6);
            search.set_margin_bottom(6);
            search.set_margin_start(12);
            search.set_margin_end(12);
            self.search_holder.append(search);
        }
        if let Some(bottom) = &top.bottom_widget {
            self.bottom_holder.append(bottom);
        }

        if let Some(hidden) = &top.bar_hidden {
            let header_bar = self.header_bar.clone();
            let bar_container = self.bar_container.clone();
            let bottom_holder = self.bottom_holder.clone();
            let hidden_guard = hidden.watch(move |ctx: nami::watcher::Context<bool>| {
                let hidden = ctx.into_value();
                let header_bar = header_bar.clone();
                let bar_container = bar_container.clone();
                let bottom_holder = bottom_holder.clone();
                glib::idle_add_local_once(move || {
                    header_bar.set_visible(!hidden);
                    bar_container.set_visible(!hidden);
                    bottom_holder.set_visible(!hidden);
                });
            });
            self.header_bar.set_visible(!hidden.get());
            self.bar_container.set_visible(!hidden.get());
            self.bottom_holder.set_visible(!hidden.get());
            self.active_bar_guards.push(hidden_guard);
        } else {
            self.header_bar.set_visible(true);
            self.bar_container.set_visible(true);
            self.bottom_holder.set_visible(true);
        }

        if let Some(color) = &top.bar_color {
            self.color_css
                .set_declarations(&css_for_header_bar_color(&color.get(), &self.env));
            let env = self.env.clone();
            let scoped_css = self.color_css.clone();
            let color_guard = color.watch(
                move |ctx: nami::watcher::Context<waterui_graphics::color::Color>| {
                    let color = ctx.into_value();
                    let declarations = css_for_header_bar_color(&color, &env);
                    let scoped_css = scoped_css.clone();
                    glib::idle_add_local_once(move || {
                        scoped_css.set_declarations(&declarations);
                    });
                },
            );
            self.active_bar_guards.push(color_guard);
        } else {
            self.color_css.clear();
        }
    }
}

fn cached_split_host_switcher(
    host: gtk4::Box,
    env: Environment,
    placeholder: AnyViewBuilder<AnyView>,
    destination: impl Fn(Id) -> AnyView + 'static,
) -> Rc<dyn Fn(Option<Id>)> {
    let widgets = Rc::new(RefCell::new(BTreeMap::<Option<Id>, Widget>::new()));
    let destination = Rc::new(destination);
    Rc::new(move |selected| {
        let host = host.clone();
        let env = env.clone();
        let placeholder = placeholder.clone();
        let destination = Rc::clone(&destination);
        let widgets = Rc::clone(&widgets);
        glib::idle_add_local_once(move || {
            // Resolve the cached widget before building, so the shared borrow is
            // released before the miss path takes a mutable one.
            let existing = widgets.borrow().get(&selected).cloned();
            let widget = existing.unwrap_or_else(|| {
                let view =
                    selected.map_or_else(|| placeholder.build(), |selected| destination(selected));
                let mut renderer = GtkRenderer::new();
                let widget = renderer.render_any(view, &env);
                widget.set_hexpand(true);
                widget.set_vexpand(true);
                widgets.borrow_mut().insert(selected, widget.clone());
                widget
            });
            clear_box_children(&host);
            host.append(&widget);
        });
    })
}

impl GtkComponent for NavigationSplitLayout {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "validated logical column widths are converted to GTK coordinates"
    )]
    #[allow(
        clippy::too_many_lines,
        reason = "one cohesive widget-construction pass; splitting it would scatter GTK setup order"
    )]
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let (
            primary,
            placeholder,
            primary_selection,
            content,
            secondary_selection,
            detail,
            visibility,
            column_width,
            style,
        ) = self.into_parts();
        let outer = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        let preferred_width = match style {
            waterui_navigation::NativeNavigationSplitStyle::Automatic
            | waterui_navigation::NativeNavigationSplitStyle::Balanced => column_width.ideal(),
            waterui_navigation::NativeNavigationSplitStyle::ProminentDetail => column_width.min(),
        };
        outer.set_position(preferred_width.round() as i32);
        let primary_widget = renderer.render_any(primary.build(), env);
        primary_widget.set_hexpand(true);
        primary_widget.set_vexpand(true);
        outer.set_start_child(Some(&primary_widget));

        let content_host = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_host.set_hexpand(true);
        content_host.set_vexpand(true);
        let detail_host = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        detail_host.set_hexpand(true);
        detail_host.set_vexpand(true);
        let is_three_column = content.is_some();
        if is_three_column {
            let inner = gtk4::Paned::new(gtk4::Orientation::Horizontal);
            inner.set_position(preferred_width.round() as i32);
            inner.set_start_child(Some(&content_host));
            inner.set_end_child(Some(&detail_host));
            outer.set_end_child(Some(&inner));
        } else {
            outer.set_end_child(Some(&detail_host));
        }

        let env = env.clone();
        let mut guards: Vec<nami::watcher::BoxWatcherGuard> = Vec::new();
        if let Some(content) = content {
            let switch_content = cached_split_host_switcher(
                content_host.clone(),
                env.clone(),
                placeholder.clone(),
                move |selected| AnyView::new(content.build(selected)),
            );
            switch_content(primary_selection.get());
            guards.push(primary_selection.computed().watch({
                let switch_content = Rc::clone(&switch_content);
                move |ctx: nami::watcher::Context<Option<Id>>| {
                    switch_content(ctx.into_value());
                }
            }));
        }

        let detail_selection = secondary_selection.unwrap_or_else(|| primary_selection.clone());
        let switch_detail =
            cached_split_host_switcher(detail_host.clone(), env, placeholder, move |selected| {
                AnyView::new(detail.build(selected))
            });
        switch_detail(detail_selection.get());
        guards.push(detail_selection.computed().watch({
            let switch_detail = Rc::clone(&switch_detail);
            move |ctx: nami::watcher::Context<Option<Id>>| {
                switch_detail(ctx.into_value());
            }
        }));

        let apply_visibility = {
            let primary = primary_widget;
            let content = content_host;
            let detail = detail_host;
            move |value: waterui_navigation::NavigationSplitColumnVisibility| {
                let (show_primary, show_content) = match value {
                    waterui_navigation::NavigationSplitColumnVisibility::Automatic
                    | waterui_navigation::NavigationSplitColumnVisibility::All => {
                        (true, is_three_column)
                    }
                    waterui_navigation::NavigationSplitColumnVisibility::DoubleColumn => {
                        (!is_three_column, is_three_column)
                    }
                    waterui_navigation::NavigationSplitColumnVisibility::DetailOnly => {
                        (false, false)
                    }
                };
                primary.set_visible(show_primary);
                content.set_visible(show_content);
                detail.set_visible(true);
            }
        };
        apply_visibility(visibility.get());
        guards.push(visibility.watch(
            move |ctx: nami::watcher::Context<
                waterui_navigation::NavigationSplitColumnVisibility,
            >| {
                let value = ctx.into_value();
                glib::idle_add_local_once({
                    let apply_visibility = apply_visibility.clone();
                    move || apply_visibility(value)
                });
            },
        ));
        store_watcher_guards(&outer, guards);
        outer.upcast()
    }
}
