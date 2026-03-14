//! GTK4 Navigation components implementation.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use nami::Signal;
use waterui_controls::text_field::TextField;
use waterui_core::Environment;
use waterui_navigation::{
    CustomNavigationController, NavigationController, NavigationSplitLayout, NavigationStack,
    NavigationTransition, NavigationView,
};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::{resolved_color_to_css_rgba, store_watcher_guards};

fn css_for_header_bar_color(color: waterui_graphics::color::Color, env: &Environment) -> String {
    let resolved = color.resolve(env).get();
    format!(
        ".waterui-navigation-headerbar {{ background-color: {}; }}",
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
    search: Option<&waterui_navigation::NavigationSearch>,
) -> Option<gtk4::Widget> {
    search.map(|search| {
        renderer
            .render(
                TextField::new(&search.text).prompt(search.prompt.clone()),
                env,
            )
            .upcast()
    })
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
        header_bar.add_css_class("waterui-navigation-headerbar");
        let provider = gtk4::CssProvider::new();
        header_bar
            .style_context()
            .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);

        let leading_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let trailing_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        header_bar.pack_start(&leading_box);
        header_bar.pack_end(&trailing_box);

        let title_widget = renderer.render_any(self.bar.title, env);
        header_bar.set_title_widget(Some(&title_widget));

        if !self.bar.leading.is::<()>() {
            let leading_widget = renderer.render_any(self.bar.leading, env);
            leading_box.append(&leading_widget);
        }

        if !self.bar.trailing.is::<()>() {
            let trailing_widget = renderer.render_any(self.bar.trailing, env);
            trailing_box.append(&trailing_widget);
        }

        let hidden_guard = self.bar.hidden.watch({
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
        let provider_for_color = provider.clone();
        let color_guard = self.bar.color.watch(
            move |ctx: nami::watcher::Context<waterui_graphics::color::Color>| {
                let color = ctx.into_value();
                let css = css_for_header_bar_color(color, &env_for_color);
                let provider = provider_for_color.clone();
                glib::idle_add_local_once(move || {
                    provider.load_from_data(&css);
                });
            },
        );

        if self.bar.hidden.get() {
            header_bar.set_visible(false);
        }

        provider.load_from_data(&css_for_header_bar_color(self.bar.color.get(), env));

        container.append(&header_bar);
        if let Some(search_widget) = render_search_widget(renderer, env, self.bar.search.as_ref()) {
            search_widget.set_margin_top(6);
            search_widget.set_margin_bottom(6);
            search_widget.set_margin_start(12);
            search_widget.set_margin_end(12);
            container.append(&search_widget);
        }

        let content_widget = renderer.render_any(self.content, env);
        container.append(&content_widget);

        store_watcher_guards(&container, vec![hidden_guard, color_guard]);
        container.upcast()
    }
}

impl GtkComponent for NavigationStack<(), ()> {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let transition = self.transition_style();
        let root = self.into_inner();

        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        let bar_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.append(&bar_container);

        let header_bar = gtk4::HeaderBar::new();
        header_bar.add_css_class("waterui-navigation-headerbar");
        let provider = gtk4::CssProvider::new();
        header_bar
            .style_context()
            .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
        bar_container.append(&header_bar);

        let back_button = gtk4::Button::with_label("Back");
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
            NavigationTransition::PushPop => {
                gtk_stack.set_transition_type(gtk4::StackTransitionType::SlideLeftRight);
                gtk_stack.set_transition_duration(250);
            }
            NavigationTransition::Fade => {
                gtk_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
                gtk_stack.set_transition_duration(250);
            }
            NavigationTransition::None => {
                gtk_stack.set_transition_type(gtk4::StackTransitionType::None);
                gtk_stack.set_transition_duration(0);
            }
        }
        container.append(&gtk_stack);

        let mut child_env = env.clone();
        let controller = GtkNavigationController::new(
            gtk_stack.clone(),
            bar_container.clone(),
            header_bar.clone(),
            leading_slot.clone(),
            trailing_slot.clone(),
            search_holder.clone(),
            back_button.clone(),
            provider.clone(),
            &child_env,
        );
        let navigation_controller = NavigationController::new(controller.clone());
        child_env.insert(navigation_controller.clone());
        controller.set_env(child_env.clone());

        back_button.connect_clicked({
            let controller = controller.clone();
            move |_| {
                let mut ctrl = controller.clone();
                ctrl.pop();
            }
        });

        match root.downcast::<NavigationView>() {
            Ok(nav_view) => {
                let NavigationView { bar, content } = *nav_view;
                let title_widget = renderer.render_any(bar.title, &child_env);
                let leading_widget =
                    (!bar.leading.is::<()>()).then(|| renderer.render_any(bar.leading, &child_env));
                let trailing_widget = (!bar.trailing.is::<()>())
                    .then(|| renderer.render_any(bar.trailing, &child_env));
                let search_widget = render_search_widget(renderer, &child_env, bar.search.as_ref());
                controller.set_root_bar_state(
                    title_widget,
                    leading_widget,
                    trailing_widget,
                    search_widget,
                    bar.color,
                    bar.hidden,
                );
                let root_widget = renderer.render_any(content, &child_env);
                gtk_stack.add_named(&root_widget, Some("root"));
                gtk_stack.set_visible_child_name("root");
            }
            Err(root) => {
                let root_widget = renderer.render_any(root, &child_env);
                gtk_stack.add_named(&root_widget, Some("root"));
                gtk_stack.set_visible_child_name("root");
            }
        }

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
    back_button: gtk4::Button,
    color_provider: gtk4::CssProvider,
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
        back_button: gtk4::Button,
        color_provider: gtk4::CssProvider,
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
                back_button,
                color_provider,
                view_stack: vec![NavigationViewState {
                    id: "root".to_string(),
                    title_widget: None,
                    leading_widget: None,
                    trailing_widget: None,
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
        search_widget: Option<gtk4::Widget>,
        bar_color: nami::Computed<waterui_graphics::color::Color>,
        bar_hidden: nami::Computed<bool>,
    ) {
        let mut inner = self.inner.borrow_mut();
        inner.view_stack[0].title_widget = Some(title_widget);
        inner.view_stack[0].leading_widget = leading_widget;
        inner.view_stack[0].trailing_widget = trailing_widget;
        inner.view_stack[0].search_widget = search_widget;
        inner.view_stack[0].bar_color = Some(bar_color);
        inner.view_stack[0].bar_hidden = Some(bar_hidden);
        inner.apply_active_bar_for_top();
    }
}

impl CustomNavigationController for GtkNavigationController {
    fn push(&mut self, content: NavigationView) {
        let mut inner = self.inner.borrow_mut();
        let id = format!("view_{}", inner.next_id);
        inner.next_id += 1;

        let mut renderer = GtkRenderer::new();
        let content_widget = renderer.render_any(content.content, &inner.env);
        let view_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        view_container.set_hexpand(true);
        view_container.set_vexpand(true);
        view_container.append(&content_widget);
        inner.stack.add_named(&view_container, Some(&id));
        inner.stack.set_visible_child_name(&id);

        let title_widget = renderer.render_any(content.bar.title, &inner.env);
        let leading_widget = (!content.bar.leading.is::<()>())
            .then(|| renderer.render_any(content.bar.leading, &inner.env));
        let trailing_widget = (!content.bar.trailing.is::<()>())
            .then(|| renderer.render_any(content.bar.trailing, &inner.env));
        let search_widget =
            render_search_widget(&mut renderer, &inner.env, content.bar.search.as_ref());

        inner.view_stack.push(NavigationViewState {
            id,
            title_widget: Some(title_widget),
            leading_widget,
            trailing_widget,
            search_widget,
            bar_color: Some(content.bar.color),
            bar_hidden: Some(content.bar.hidden),
        });
        inner.apply_active_bar_for_top();
    }

    fn pop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.pop_internal();
    }
}

impl GtkNavigationControllerInner {
    fn pop_internal(&mut self) {
        if self.view_stack.len() <= 1 {
            return;
        }

        if let Some(current) = self.view_stack.pop() {
            if let Some(child) = self.stack.child_by_name(&current.id) {
                self.stack.remove(&child);
            }
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

        if let Some(hidden) = &top.bar_hidden {
            let header_bar = self.header_bar.clone();
            let bar_container = self.bar_container.clone();
            let hidden_guard = hidden.watch(move |ctx: nami::watcher::Context<bool>| {
                let hidden = ctx.into_value();
                let header_bar = header_bar.clone();
                let bar_container = bar_container.clone();
                glib::idle_add_local_once(move || {
                    header_bar.set_visible(!hidden);
                    bar_container.set_visible(!hidden);
                });
            });
            self.header_bar.set_visible(!hidden.get());
            self.bar_container.set_visible(!hidden.get());
            self.active_bar_guards.push(hidden_guard);
        } else {
            self.header_bar.set_visible(true);
            self.bar_container.set_visible(true);
        }

        if let Some(color) = &top.bar_color {
            self.color_provider
                .load_from_data(&css_for_header_bar_color(color.get(), &self.env));
            let env = self.env.clone();
            let provider = self.color_provider.clone();
            let color_guard = color.watch(
                move |ctx: nami::watcher::Context<waterui_graphics::color::Color>| {
                    let color = ctx.into_value();
                    let css = css_for_header_bar_color(color, &env);
                    let provider = provider.clone();
                    glib::idle_add_local_once(move || {
                        provider.load_from_data(&css);
                    });
                },
            );
            self.active_bar_guards.push(color_guard);
        }
    }
}

impl GtkComponent for NavigationSplitLayout {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        paned.set_position(self.sidebar_width as i32);
        let sidebar = renderer.render_any(self.sidebar.build(), env);
        sidebar.set_hexpand(true);
        sidebar.set_vexpand(true);
        paned.set_start_child(Some(&sidebar));
        let detail = if let Some(detail) = self.detail {
            renderer.render(detail, env)
        } else {
            renderer.render_any(self.placeholder.build(), env)
        };
        detail.set_hexpand(true);
        detail.set_vexpand(true);
        paned.set_end_child(Some(&detail));
        paned.upcast()
    }
}
