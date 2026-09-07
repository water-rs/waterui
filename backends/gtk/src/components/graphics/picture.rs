//! A `Picture` shown by a `gtk4::Picture`, rasterised on the CPU at the
//! widget's scale factor and re-rasterised when the drawing changes.

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Widget, gdk, glib};
use waterui_core::{Environment, Native, Signal};
use waterui_graphics::Picture;
use waterui_graphics::scene2d_cpu::rasterize_recording;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<Picture> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let picture = self.into_inner();
        let widget = gtk4::Picture::new();
        widget.set_can_shrink(false);
        widget.set_content_fit(gtk4::ContentFit::Contain);
        let size = picture.size();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Picture::new asserts a finite positive size; one past i32 is not a widget GTK can show"
        )]
        let (request_width, request_height) =
            (size.width.round() as i32, size.height.round() as i32);
        widget.set_size_request(request_width, request_height);

        let paint = Rc::new({
            let widget = widget.clone();
            let picture = picture.clone();
            move || {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a scale factor is a small integer, which f32 holds exactly"
                )]
                let scale = widget.scale_factor() as f32;
                let (width, height) = picture.pixel_size(scale);
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "pixel_size bounds both sides to 65535, which f32 holds exactly"
                )]
                let transform = picture.transform_to(width as f32, height as f32);
                let recording = picture.recording().get();
                let bitmap = rasterize_recording(&recording, width, height, transform);
                let stride = usize::try_from(width * 4).expect("a bitmap row fits usize");
                let bytes = glib::Bytes::from_owned(bitmap.into_data());
                let texture = gdk::MemoryTexture::new(
                    i32::try_from(width).expect("a bitmap side fits i32"),
                    i32::try_from(height).expect("a bitmap side fits i32"),
                    gdk::MemoryFormat::R8g8b8a8Premultiplied,
                    &bytes,
                    stride,
                );
                widget.set_paintable(Some(&texture));
            }
        });
        paint();
        widget.connect_scale_factor_notify({
            let paint = Rc::clone(&paint);
            move |_| paint()
        });
        let guard = picture.recording().watch({
            let paint = Rc::clone(&paint);
            move |_| {
                let paint = Rc::clone(&paint);
                glib::idle_add_local_once(move || paint());
            }
        });
        store_watcher_guard(&widget, Box::new(guard));
        widget.upcast()
    }
}
