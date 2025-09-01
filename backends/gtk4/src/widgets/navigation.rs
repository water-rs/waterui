//! GTK4 navigation widget implementations.

use gtk4::{Box as GtkBox, Button, HeaderBar, Notebook, Orientation, Widget, prelude::*};
use waterui::{Environment, View};

use crate::renderer::render;
// use waterui_navigation::{Tab, TabView};

/// Render a TabView as a GTK4 Notebook.
pub fn render_tab_view<V: View + Clone>(
    tabs: Vec<(String, V)>, // (title, content)
    env: &Environment,
) -> Widget {
    let notebook = Notebook::new();

    for (title, content) in tabs {
        let content_widget = render(content, env);
        let tab_label = gtk4::Label::new(Some(&title));

        notebook.append_page(&content_widget, Some(&tab_label));
    }

    notebook.upcast()
}

/// Render navigation buttons.
pub fn render_nav_buttons(
    buttons: Vec<String>, // button labels
    _env: &Environment,
) -> Widget {
    let button_box = GtkBox::new(Orientation::Horizontal, 8);

    for label in buttons {
        let button = Button::with_label(&label);
        // TODO: Connect button click actions
        // In a complete implementation, this would connect to navigation handlers
        // button.connect_clicked(move |_| { /* navigation logic */ });
        button_box.append(&button);
    }

    button_box.upcast()
}

/// Create a header bar with navigation elements.
pub fn render_header_bar(title: &str, start_widgets: &[Widget], end_widgets: &[Widget]) -> Widget {
    let header_bar = HeaderBar::new();

    header_bar.set_title_widget(Some(&gtk4::Label::new(Some(title))));

    for widget in start_widgets {
        header_bar.pack_start(widget);
    }

    for widget in end_widgets {
        header_bar.pack_end(widget);
    }

    header_bar.upcast()
}

/// Render a simple back button.
pub fn render_back_button() -> Widget {
    let button = Button::new();
    button.set_icon_name("go-previous-symbolic");
    button.set_tooltip_text(Some("Back"));

    // TODO: Connect navigation action
    // In a complete implementation, this would connect to the navigation system
    // button.connect_clicked(move |_| { /* go back */ });

    button.upcast()
}
