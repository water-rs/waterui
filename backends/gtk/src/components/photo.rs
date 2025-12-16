//! GTK4 Photo (Image) component implementation.
//!
//! Displays images from URLs using GTK4's async image loading.

use std::cell::RefCell;
use std::rc::Rc;

use gdk_pixbuf::Pixbuf;
use gtk4::prelude::*;
use gtk4::Widget;
use waterui_core::{Environment, Native};
use waterui_media::photo::{Event, PhotoConfig};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<PhotoConfig> {
    /// Renders a `WaterUI` `Photo` as a GTK4 Picture widget.
    ///
    /// Loads images asynchronously from the provided URL.
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();
        let url = config.source;
        let on_event = Rc::new(RefCell::new(Some(config.on_event)));

        // Create a picture widget with placeholder
        let picture = gtk4::Picture::new();
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_can_shrink(true);

        // Create a spinner as loading placeholder
        let spinner = gtk4::Spinner::new();
        spinner.set_spinning(true);

        // Use a stack to show spinner while loading
        let stack = gtk4::Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        stack.add_named(&spinner, Some("loading"));
        stack.add_named(&picture, Some("image"));
        stack.set_visible_child_name("loading");

        // Load image asynchronously
        let url_string = url.to_string();
        let stack_clone = stack.clone();
        let picture_clone = picture.clone();
        let on_event_clone = on_event.clone();

        glib::spawn_future_local(async move {
            match load_image_from_url(&url_string).await {
                Ok(pixbuf) => {
                    let texture = gdk4::Texture::for_pixbuf(&pixbuf);
                    picture_clone.set_paintable(Some(&texture));
                    stack_clone.set_visible_child_name("image");

                    // Fire loaded event
                    if let Some(handler) = on_event_clone.borrow_mut().take() {
                        handler(Event::Loaded);
                    }
                }
                Err(e) => {
                    tracing::warn!("[Photo] Failed to load image: {}", e);

                    // Show error placeholder
                    let error_label = gtk4::Label::new(Some("Failed to load image"));
                    error_label.add_css_class("dim-label");
                    stack_clone.add_named(&error_label, Some("error"));
                    stack_clone.set_visible_child_name("error");

                    // Fire error event
                    if let Some(handler) = on_event_clone.borrow_mut().take() {
                        handler(Event::Error(e));
                    }
                }
            }
        });

        stack.upcast()
    }
}

/// Loads an image from a URL asynchronously.
async fn load_image_from_url(url: &str) -> Result<Pixbuf, String> {
    // Use gio's async file loading for http/https URLs
    let file = gtk4::gio::File::for_uri(url);

    // Read the file contents
    let (contents, _etag) = file
        .load_contents_future()
        .await
        .map_err(|e| format!("Failed to fetch URL: {e}"))?;

    // Create pixbuf from bytes
    let bytes = glib::Bytes::from(&contents);
    let stream = gtk4::gio::MemoryInputStream::from_bytes(&bytes);

    Pixbuf::from_stream_future(&stream)
        .await
        .map_err(|e| format!("Failed to decode image: {e}"))
}
