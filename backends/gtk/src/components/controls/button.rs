//! GTK4 Button component implementation.

use std::cell::RefCell;

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_controls::button::{ButtonConfig, ButtonStyle};
use waterui_core::{Environment, Native};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

fn button_style_css_classes(style: ButtonStyle) -> &'static [&'static str] {
    match style {
        ButtonStyle::Automatic | ButtonStyle::Bordered => &[],
        ButtonStyle::Plain | ButtonStyle::Borderless => &["flat"],
        ButtonStyle::Link => &["flat", "link"],
        ButtonStyle::BorderedProminent => &["suggested-action"],
        _ => panic!("unsupported ButtonStyle variant on GTK backend"),
    }
}

impl GtkComponent for Native<ButtonConfig> {
    /// Renders a `WaterUI` Button component as a GTK4 Button.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        let button = gtk4::Button::new();
        let borderless = matches!(
            config.style,
            ButtonStyle::Plain | ButtonStyle::Borderless | ButtonStyle::Link
        );
        button.set_has_frame(!borderless);

        // Render the label view and set as button child
        let label_widget = renderer.render(config.label, env);
        button.set_child(Some(&label_widget));

        // Connect click handler using RefCell for interior mutability
        // since GTK's connect_clicked requires Fn, not FnMut
        let action = RefCell::new(config.action);
        let env_clone = env.clone();
        button.connect_clicked(move |_| {
            let mut action = action.borrow_mut();
            (**action)(&env_clone);
        });

        for class_name in button_style_css_classes(config.style) {
            button.add_css_class(class_name);
        }

        button.upcast()
    }
}

#[cfg(test)]
mod tests {
    use super::button_style_css_classes;
    use waterui_controls::button::ButtonStyle;

    #[test]
    fn gtk_button_style_css_classes_match_semantic_style() {
        assert_eq!(
            button_style_css_classes(ButtonStyle::Automatic),
            &[] as &[&str]
        );
        assert_eq!(
            button_style_css_classes(ButtonStyle::Bordered),
            &[] as &[&str]
        );
        assert_eq!(button_style_css_classes(ButtonStyle::Plain), &["flat"]);
        assert_eq!(button_style_css_classes(ButtonStyle::Borderless), &["flat"]);
        assert_eq!(
            button_style_css_classes(ButtonStyle::Link),
            &["flat", "link"]
        );
        assert_eq!(
            button_style_css_classes(ButtonStyle::BorderedProminent),
            &["suggested-action"]
        );
    }
}
