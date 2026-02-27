//! GTK4 Navigation components implementation.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use nami::Signal;
use waterui_core::Environment;
use waterui_navigation::{
    CustomNavigationController, NavigationController, NavigationStack, NavigationTransition,
    NavigationView,
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

impl GtkComponent for NavigationView {
    /// Renders a `WaterUI` `NavigationView` as a GTK4 Box with HeaderBar.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        // When this NavigationView lives inside a NavigationStack, the stack owns the
        // navigation chrome. In that case, render content only (no nested header bars).
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

        // Create the header bar
        let header_bar = gtk4::HeaderBar::new();
        header_bar.add_css_class("waterui-navigation-headerbar");
        let provider = gtk4::CssProvider::new();
        header_bar
            .style_context()
            .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);

        // Title is a view; render it and let that subtree manage its own reactivity.
        let title_widget = renderer.render_any(self.bar.title, env);
        header_bar.set_title_widget(Some(&title_widget));

        // Watch for hidden state changes
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

        // Watch for color changes
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

        // Set initial hidden state
        if self.bar.hidden.get() {
            header_bar.set_visible(false);
        }

        // Set initial color state
        provider.load_from_data(&css_for_header_bar_color(self.bar.color.get(), env));

        container.append(&header_bar);

        // Render and add the content
        let content_widget = renderer.render_any(self.content, env);
        container.append(&content_widget);

        // Store watcher guards
        store_watcher_guards(&container, vec![hidden_guard, color_guard]);

        container.upcast()
    }
}

impl GtkComponent for NavigationStack<(), ()> {
    /// Renders a `WaterUI` `NavigationStack` as a GTK4 Stack with navigation.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let transition = self.transition_style();
        let root = self.into_inner();

        // Create the main container
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Create a header bar for the stack
        let header_bar = gtk4::HeaderBar::new();
        header_bar.add_css_class("waterui-navigation-headerbar");
        let provider = gtk4::CssProvider::new();
        header_bar
            .style_context()
            .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);

        let back_button = gtk4::Button::with_label("Back");
        back_button.set_visible(false);
        header_bar.pack_start(&back_button);
        container.append(&header_bar);

        // Create the stack for content views
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

        // Install the controller in the environment for child views
        let mut child_env = env.clone();

        // Create the GTK navigation controller and install it into the subtree environment.
        let controller = GtkNavigationController::new(
            gtk_stack.clone(),
            header_bar.clone(),
            back_button.clone(),
            provider.clone(),
            &child_env,
        );
        let navigation_controller = NavigationController::new(controller.clone());

        // Insert controller into environment so child views can access it
        child_env.insert(navigation_controller.clone());
        controller.set_env(child_env.clone());

        // Back button should route through the controller so it shares the same logic as Rust-driven pops.
        back_button.connect_clicked({
            let controller = controller.clone();
            move |_| {
                let mut ctrl = controller.clone();
                ctrl.pop();
            }
        });

        // Render the root view. If it is a NavigationView, let the stack own the chrome.
        match root.downcast::<NavigationView>() {
            Ok(nav_view) => {
                let NavigationView { bar, content } = *nav_view;
                let title_widget = renderer.render_any(bar.title, &child_env);
                controller.set_root_bar_state(title_widget, bar.color, bar.hidden);
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

/// GTK-specific navigation controller implementation.
#[derive(Clone)]
struct GtkNavigationController {
    inner: Rc<RefCell<GtkNavigationControllerInner>>,
}

struct GtkNavigationControllerInner {
    stack: gtk4::Stack,
    header_bar: gtk4::HeaderBar,
    back_button: gtk4::Button,
    color_provider: gtk4::CssProvider,
    view_stack: Vec<NavigationViewState>,
    active_bar_guards: Vec<nami::watcher::BoxWatcherGuard>,
    next_id: usize,
    /// Environment for rendering child views.
    env: Environment,
}

struct NavigationViewState {
    id: String,
    title_widget: Option<gtk4::Widget>,
    bar_color: Option<nami::Computed<waterui_graphics::color::Color>>,
    bar_hidden: Option<nami::Computed<bool>>,
}

impl GtkNavigationController {
    /// Creates a new GTK navigation controller.
    ///
    fn new(
        stack: gtk4::Stack,
        header_bar: gtk4::HeaderBar,
        back_button: gtk4::Button,
        color_provider: gtk4::CssProvider,
        env: &Environment,
    ) -> Self {
        Self {
            inner: Rc::new(RefCell::new(GtkNavigationControllerInner {
                stack,
                header_bar,
                back_button,
                color_provider,
                // Track root so `pop()` can return to it.
                view_stack: vec![NavigationViewState {
                    id: "root".to_string(),
                    title_widget: None,
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

    fn set_root_bar_state(
        &self,
        title_widget: gtk4::Widget,
        bar_color: nami::Computed<waterui_graphics::color::Color>,
        bar_hidden: nami::Computed<bool>,
    ) {
        let mut inner = self.inner.borrow_mut();
        inner.view_stack[0].title_widget = Some(title_widget);
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

        // Render with a fresh renderer to avoid holding a raw pointer.
        let mut renderer = GtkRenderer::new();
        let content_widget = renderer.render_any(content.content, &inner.env);

        // Create container and add the rendered content
        let view_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        view_container.set_hexpand(true);
        view_container.set_vexpand(true);
        view_container.append(&content_widget);

        // Add to stack
        inner.stack.add_named(&view_container, Some(&id));
        inner.stack.set_visible_child_name(&id);

        // Update header bar title widget
        let title_widget = renderer.render_any(content.bar.title, &inner.env);
        // Track the view and its bar configuration for later restores.
        inner.view_stack.push(NavigationViewState {
            id,
            title_widget: Some(title_widget),
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
            return; // Can't pop the root view
        }

        // Remove current view
        if let Some(current) = self.view_stack.pop() {
            if let Some(child) = self.stack.child_by_name(&current.id) {
                self.stack.remove(&child);
            }
        }

        // Show previous view
        if let Some(previous) = self.view_stack.last() {
            self.stack.set_visible_child_name(&previous.id);
        }

        self.apply_active_bar_for_top();
    }

    fn apply_active_bar_for_top(&mut self) {
        // Drop previous active subscriptions so only the top-of-stack drives chrome.
        self.active_bar_guards.clear();

        let is_root = self.view_stack.len() <= 1;
        self.back_button.set_visible(!is_root);

        let Some(top) = self.view_stack.last() else {
            return;
        };

        // Title widget
        if let Some(title) = &top.title_widget {
            self.header_bar.set_title_widget(Some(title));
        } else {
            self.header_bar.set_title_widget(None::<&gtk4::Widget>);
        }

        // Hidden state
        if let Some(hidden) = &top.bar_hidden {
            let header_bar = self.header_bar.clone();
            let hidden_guard = hidden.watch(move |ctx: nami::watcher::Context<bool>| {
                let hidden = ctx.into_value();
                let header_bar = header_bar.clone();
                glib::idle_add_local_once(move || {
                    header_bar.set_visible(!hidden);
                });
            });
            self.header_bar.set_visible(!hidden.get());
            self.active_bar_guards.push(hidden_guard);
        } else {
            self.header_bar.set_visible(true);
        }

        // Background color
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
