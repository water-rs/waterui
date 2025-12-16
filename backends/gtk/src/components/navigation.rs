//! GTK4 Navigation components implementation.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::Widget;
use nami::Signal;
use waterui_core::Environment;
use waterui_navigation::{CustomNavigationController, NavigationController, NavigationStack, NavigationView};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guards;

impl GtkComponent for NavigationView {
    /// Renders a `WaterUI` `NavigationView` as a GTK4 Box with HeaderBar.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Create the header bar
        let header_bar = gtk4::HeaderBar::new();

        // Set the title
        let title_text = self.bar.title.content().get().to_plain();
        let title_label = gtk4::Label::new(Some(&title_text));
        header_bar.set_title_widget(Some(&title_label));

        // Watch for title changes
        let guard = self.bar.title.content().watch({
            let title_label = title_label.clone();
            move |ctx| {
                let text = ctx.into_value().to_plain();
                let title_label = title_label.clone();
                glib::idle_add_local_once(move || {
                    title_label.set_text(&text);
                });
            }
        });

        // Watch for hidden state changes
        let hidden_guard = self.bar.hidden.watch({
            let header_bar = header_bar.clone();
            move |ctx| {
                let hidden = ctx.into_value();
                let header_bar = header_bar.clone();
                glib::idle_add_local_once(move || {
                    header_bar.set_visible(!hidden);
                });
            }
        });

        // Set initial hidden state
        if self.bar.hidden.get() {
            header_bar.set_visible(false);
        }

        container.append(&header_bar);

        // Render and add the content
        let content_widget = renderer.render_any(self.content, env);
        container.append(&content_widget);

        // Store watcher guards
        store_watcher_guards(&container, vec![guard, hidden_guard]);

        container.upcast()
    }
}

impl GtkComponent for NavigationStack<(), ()> {
    /// Renders a `WaterUI` `NavigationStack` as a GTK4 Stack with navigation.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let root = self.into_inner();

        // Create the main container
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Create a header bar for the stack
        let header_bar = gtk4::HeaderBar::new();
        container.append(&header_bar);

        // Create the stack for content views
        let gtk_stack = gtk4::Stack::new();
        gtk_stack.set_hexpand(true);
        gtk_stack.set_vexpand(true);
        gtk_stack.set_transition_type(gtk4::StackTransitionType::SlideLeftRight);
        gtk_stack.set_transition_duration(250);

        container.append(&gtk_stack);

        // Create the GTK navigation controller
        let controller = GtkNavigationController::new(gtk_stack.clone(), header_bar.clone());
        let navigation_controller = NavigationController::new(controller.clone());

        // Install the controller in the environment for child views
        let mut child_env = env.clone();
        child_env.insert(navigation_controller);

        // Render the root view
        let root_widget = renderer.render_any(root, &child_env);
        gtk_stack.add_named(&root_widget, Some("root"));
        gtk_stack.set_visible_child_name("root");

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
    view_stack: Vec<NavigationViewState>,
    next_id: usize,
}

struct NavigationViewState {
    id: String,
    title: String,
}

impl GtkNavigationController {
    fn new(stack: gtk4::Stack, header_bar: gtk4::HeaderBar) -> Self {
        Self {
            inner: Rc::new(RefCell::new(GtkNavigationControllerInner {
                stack,
                header_bar,
                view_stack: Vec::new(),
                next_id: 0,
            })),
        }
    }
}

impl CustomNavigationController for GtkNavigationController {
    fn push(&mut self, content: NavigationView) {
        let mut inner = self.inner.borrow_mut();

        let id = format!("view_{}", inner.next_id);
        inner.next_id += 1;

        let title = content.bar.title.content().get().to_plain();

        // Create container for the view content
        let view_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        view_container.set_hexpand(true);
        view_container.set_vexpand(true);

        // For now, we'll just create a placeholder
        // Full implementation would render the content view
        let label = gtk4::Label::new(Some(&format!("Content for: {title}")));
        view_container.append(&label);

        // Add to stack
        inner.stack.add_named(&view_container, Some(&id));
        inner.stack.set_visible_child_name(&id);

        // Update header bar title
        let title_label = gtk4::Label::new(Some(&title));
        inner.header_bar.set_title_widget(Some(&title_label));

        // Add back button if this isn't the first view
        if !inner.view_stack.is_empty() {
            let back_button = gtk4::Button::with_label("Back");
            let controller = GtkNavigationController {
                inner: self.inner.clone(),
            };
            back_button.connect_clicked(move |_| {
                let mut ctrl = controller.inner.borrow_mut();
                GtkNavigationControllerInner::pop_internal(&mut ctrl);
            });
            inner.header_bar.pack_start(&back_button);
        }

        // Track the view
        inner.view_stack.push(NavigationViewState { id, title: title.to_string() });
    }

    fn pop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        GtkNavigationControllerInner::pop_internal(&mut inner);
    }
}

impl GtkNavigationControllerInner {
    fn pop_internal(inner: &mut GtkNavigationControllerInner) {
        if inner.view_stack.len() <= 1 {
            return; // Can't pop the root view
        }

        // Remove current view
        if let Some(current) = inner.view_stack.pop() {
            if let Some(child) = inner.stack.child_by_name(&current.id) {
                inner.stack.remove(&child);
            }
        }

        // Show previous view
        if let Some(previous) = inner.view_stack.last() {
            inner.stack.set_visible_child_name(&previous.id);

            // Update title
            let title_label = gtk4::Label::new(Some(&previous.title));
            inner.header_bar.set_title_widget(Some(&title_label));
        }
    }
}
