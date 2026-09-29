//! water-rs/hydrolysis#164 — a `peniko::ImageData` whose `data` length does
//! not match `width * height * bytes_per_pixel(format)` (the issue's example:
//! a grayscale buffer labelled `Rgba8`) used to reach wgpu
//! `Queue::write_texture` untouched and panic there with a message naming
//! neither the image nor the view; the unwind then panicked again in the
//! swapchain destructor and aborted the process. Validation now happens at
//! the first hydrolysis-owned point — the `Scene2D` handed to
//! `SceneContent::build_scene` — and the panic names the expected and actual
//! byte counts and the format.

use std::cell::RefCell;

use kurbo::Affine;
use peniko::{Blob, Brush, ImageAlphaType, ImageBrush, ImageData, ImageFormat};
use waterui::{AnyView, View};
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::{Scene2D, SceneContent, SceneInvalidator, SceneView};

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::renderer::assert_well_formed_image;

const WINDOW: u32 = 160;

/// A 10x10 `Rgba8` frame needs 400 bytes; the issue's malformed case ships
/// only the grayscale plane (100 bytes) under the `Rgba8` label.
fn malformed_image() -> ImageData {
    ImageData {
        data: Blob::from(vec![0u8; 100]),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: 10,
        height: 10,
    }
}

fn well_formed_image() -> ImageData {
    ImageData {
        data: Blob::from(vec![0u8; 400]),
        ..malformed_image()
    }
}

#[test]
#[should_panic(
    expected = "Rgba8 at 10x10 needs width*height*bytes_per_pixel = 400 bytes, but the blob holds 100 bytes"
)]
fn malformed_image_data_is_named_at_ingest() {
    assert_well_formed_image(&malformed_image());
}

#[test]
fn well_formed_image_data_passes_ingest() {
    assert_well_formed_image(&well_formed_image());
}

fn runtime_with(view: impl View) -> HeadlessRuntime {
    let view = RefCell::new(Some(AnyView::new(view)));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW,
        WINDOW,
        MinimalTestTheme::default(),
    )
}

/// Scene content that pushes a single image brush command into the scene it
/// is handed — the real `build_scene` ingest boundary from the issue.
struct ImagePane {
    draw: fn(&mut dyn Scene2D),
}

impl SceneContent for ImagePane {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, _width: f32, _height: f32) -> bool {
        (self.draw)(scene);
        false
    }

    fn set_invalidator(&mut self, _invalidator: Option<SceneInvalidator>) {}
}

/// The issue's verbatim failure: `draw_image` of a grayscale buffer labelled
/// `Rgba8`, through the retained tree's scene flush.
#[test]
#[should_panic(
    expected = "Rgba8 at 10x10 needs width*height*bytes_per_pixel = 400 bytes, but the blob holds 100 bytes"
)]
fn scene_view_rejects_malformed_image_at_draw_image() {
    let mut runtime = runtime_with(SceneView::new(ImagePane {
        draw: |scene| {
            scene.draw_image(&ImageBrush::new(malformed_image()), Affine::IDENTITY);
        },
    }));
    let _ = runtime.pump(false);
}

/// The same malformed buffer carried inside a `Brush::Image` fill: the ingest
/// check must see through the brush, not just the `draw_image` entrypoint.
#[test]
#[should_panic(
    expected = "Rgba8 at 10x10 needs width*height*bytes_per_pixel = 400 bytes, but the blob holds 100 bytes"
)]
fn scene_view_rejects_malformed_image_inside_a_fill_brush() {
    let mut runtime = runtime_with(SceneView::new(ImagePane {
        draw: |scene| {
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                &Brush::Image(ImageBrush::new(malformed_image())),
                None,
                &kurbo::BezPath::from_svg("M0,0 L10,0 L10,10 Z").expect("static path parses"),
            );
        },
    }));
    let _ = runtime.pump(false);
}

/// A correctly sized image records into the scene without complaint — the
/// check only fires on malformed input.
#[test]
fn scene_view_accepts_a_well_formed_image() {
    let mut runtime = runtime_with(SceneView::new(ImagePane {
        draw: |scene| {
            scene.draw_image(&ImageBrush::new(well_formed_image()), Affine::IDENTITY);
        },
    }));
    let _ = runtime.pump(false);
}
