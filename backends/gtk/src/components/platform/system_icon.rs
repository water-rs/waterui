//! GTK4 SystemIcon component implementation.

use gtk4::prelude::*;
use gtk4::{Image, Label, Widget};
use waterui_core::{Environment, Native};
use waterui_icon::SystemIcon;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<SystemIcon> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let icon = self.into_inner();
        let image = Image::new();
        image.set_pixel_size(16);

        let candidates = icon_name_candidates(icon.name.as_str());
        if let Some(display) = gdk4::Display::default() {
            let theme = gtk4::IconTheme::for_display(&display);
            for candidate in &candidates {
                if theme.has_icon(candidate) {
                    image.set_icon_name(Some(candidate));
                    image.add_css_class("image");
                    return image.upcast();
                }
            }
        }

        // Fast-fail only for truly invalid icon names; otherwise keep visual continuity.
        if icon.name.as_str().is_empty() {
            panic!("SystemIcon name must not be empty");
        }

        Label::new(Some(icon.name.as_str())).upcast()
    }
}

fn icon_name_candidates(name: &str) -> Vec<String> {
    let normalized = name.replace('.', "-");
    let mapped = match name {
        "house" => vec!["go-home-symbolic", "go-home"],
        "gear" => vec!["emblem-system-symbolic", "preferences-system"],
        "magnifyingglass" => vec!["system-search-symbolic", "system-search"],
        "person" => vec!["avatar-default-symbolic", "avatar-default"],
        "plus" => vec!["list-add-symbolic", "list-add"],
        "trash" => vec!["user-trash-symbolic", "user-trash"],
        "chevron.right" => vec!["go-next-symbolic", "go-next"],
        "chevron.left" => vec!["go-previous-symbolic", "go-previous"],
        "xmark" => vec!["window-close-symbolic", "window-close"],
        "checkmark" => vec!["object-select-symbolic", "object-select"],
        "star" => vec!["starred-symbolic", "starred"],
        "heart" => vec!["emblem-favorite-symbolic", "emblem-favorite"],
        _ => vec![],
    };

    let mut candidates = mapped
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    candidates.push(format!("{normalized}-symbolic"));
    candidates.push(normalized);
    candidates.push(name.to_owned());
    candidates
}
