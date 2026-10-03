//! water-rs/hydrolysis#164 — a `peniko::ImageData` whose `data` length does
//! not match `width * height * bytes_per_pixel(format)` (the issue's example:
//! a grayscale buffer labelled `Rgba8`) used to reach wgpu
//! `Queue::write_texture` untouched and panic there with a message naming
//! neither the image nor the view; the unwind then panicked again in the
//! swapchain destructor and aborted the process. Validation now happens at
//! the first owned point on each ingest path: `assert_well_formed_image`
//! guards the peniko payloads `Recording::image` lowers, and
//! `cherenkov::ImageData::new` — whose `Err` names the expected and actual
//! byte counts and the format — guards the `SceneResources::image`
//! registration a `SceneContent::build_scene` drives.

use std::cell::RefCell;
use std::sync::Arc;

use cherenkov::{Draw, ImagePattern, Paint, Rgba8, Sampling};
use kurbo::Rect;
use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
use waterui::{AnyView, View};
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::cherenkov::Recorder;
use waterui_graphics::{RecordingResources, SceneContent, SceneInvalidator, SceneView};

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

/// Scene content that records a single image op into the scene it is handed —
/// the real `build_scene` ingest boundary from the issue.
struct ImagePane {
    draw: fn(&mut Recorder, &mut RecordingResources<'_>, &mut ImageHandle),
    /// The live registration the recordings name. `Registered` is RAII — the
    /// engine unregisters the image when the last handle drops, so a pane that
    /// registers keeps the handle for the pane's life.
    image: ImageHandle,
}

/// A retained `Registered` image handle, `None` until the pane registers.
type ImageHandle = Option<waterui_graphics::Registered<cherenkov::Image<cherenkov::Rgba8>>>;

impl SceneContent for ImagePane {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> bool {
        (self.draw)(recorder, resources, &mut self.image);
        false
    }

    fn set_invalidator(&mut self, _invalidator: Option<SceneInvalidator>) {}
}

/// The bytes the issue's malformed case ships — the 100-byte grayscale plane
/// a 10x10 `Rgba8` upload needs 400 of.
fn malformed_blob() -> Arc<[u8]> {
    Arc::from(vec![0u8; 100])
}

/// The issue's verbatim failure: an image upload of a grayscale buffer
/// labelled `Rgba8`, through the retained tree's scene flush.
#[test]
#[should_panic(expected = "10x10 Rgba8 needs 400 bytes, got 100")]
fn scene_view_rejects_malformed_image_at_image_upload() {
    let mut runtime = runtime_with(SceneView::new(ImagePane {
        draw: |recorder, resources, _| {
            let image = resources
                .image(
                    cherenkov::ImageData::<Rgba8>::new(10, 10, malformed_blob())
                        .expect("hydrolysis scene ingest must name malformed Rgba8 data"),
                )
                .expect("hydrolysis scene ingest: image registration failed");
            let image = resources.name(&image);
            recorder.image(image, Rect::new(0.0, 0.0, 10.0, 10.0), Sampling::Linear);
        },
        image: None,
    }));
    let _ = runtime.pump(false);
}

/// The same malformed buffer carried inside an `ImagePattern` fill: the
/// ingest check must see through the paint, not just the `image` entrypoint.
#[test]
#[should_panic(expected = "10x10 Rgba8 needs 400 bytes, got 100")]
fn scene_view_rejects_malformed_image_inside_a_fill_paint() {
    let mut runtime = runtime_with(SceneView::new(ImagePane {
        draw: |recorder, resources, _| {
            let image = resources
                .image(
                    cherenkov::ImageData::<Rgba8>::new(10, 10, malformed_blob())
                        .expect("hydrolysis scene ingest must name malformed Rgba8 data"),
                )
                .expect("hydrolysis scene ingest: image registration failed");
            recorder.fill(
                Rect::new(0.0, 0.0, 10.0, 10.0),
                Paint::Image(ImagePattern {
                    image: resources.name(&image),
                    transform: kurbo::Affine::IDENTITY,
                    extend_x: cherenkov::Extend::Pad,
                    extend_y: cherenkov::Extend::Pad,
                    sampling: Sampling::Linear,
                }),
            );
        },
        image: None,
    }));
    let _ = runtime.pump(false);
}

/// A correctly sized image records into the scene without complaint — the
/// check only fires on malformed input.
#[test]
fn scene_view_accepts_a_well_formed_image() {
    let mut runtime = runtime_with(SceneView::new(ImagePane {
        draw: |recorder, resources, image| {
            if image.is_none() {
                *image = Some(
                    resources
                        .image(
                            cherenkov::ImageData::<Rgba8>::new(
                                10,
                                10,
                                Arc::<[u8]>::from(vec![0u8; 400]),
                            )
                            .expect("a well-formed Rgba8 image is accepted"),
                        )
                        .expect("hydrolysis scene ingest: image registration failed"),
                );
            }
            let id = resources.name(image.as_ref().expect("the pane registered its image"));
            recorder.image(id, Rect::new(0.0, 0.0, 10.0, 10.0), Sampling::Linear);
        },
        image: None,
    }));
    let _ = runtime.pump(false);
}
