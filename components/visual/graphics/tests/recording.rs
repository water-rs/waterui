//! The recording boundary's contract: constant and signal operands, engine
//! resource handles, and what a rasterizer draws for each.
//!
//! These tests draw into `cherenkov::Recorder`, mount the result on a real
//! `cherenkov_cpu` engine through [`waterui_graphics::raster::Rasterizer`]
//! and read pixels back, because the signature is only half the contract —
//! the other half is that what was recorded is what the engine draws.
#![cfg(feature = "cpu")]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use nami::{Binding, SignalExt};
use waterui_graphics::draw::kurbo::{Affine, Rect, Shape};
use waterui_graphics::draw::{
    Content, ContentChange, Draw, Glyph, GlyphRun, GlyphStyle, ImageId, LayoutSize, Recorder,
    Sampling, StaticRecorder, WorkingColor,
};
use waterui_graphics::raster::{Rasterizer, RgbaBitmap};
use waterui_graphics::{FontSource, ImageColorSpace, ImageData, Rgba8};
use waterui_graphics::{
    Picture, PictureSource, RecordingResources, Registered, SceneContent, SceneView,
};

/// sRGB red as the engine's working colour — `WorkingColor::new` names
/// Display P3 components directly, which sRGB cannot hold, so pixel
/// expectations come from converting a named sRGB colour.
fn srgb(red: f32, green: f32, blue: f32) -> WorkingColor {
    waterui_graphics::draw::Color::<waterui_graphics::draw::Srgb>::new([red, green, blue, 1.0])
        .into()
}

fn filled_square(recorder: &mut Recorder, color: WorkingColor) {
    recorder.fill(Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.05), color);
}

fn pixel(bitmap: &RgbaBitmap, x: usize, y: usize) -> [u8; 4] {
    let i = (y * bitmap.width() as usize + x) * 4;
    [
        bitmap.data()[i],
        bitmap.data()[i + 1],
        bitmap.data()[i + 2],
        bitmap.data()[i + 3],
    ]
}

#[test]
fn a_fill_is_one_command_in_the_recorded_content() {
    let content = Content::record(&LayoutSize::new(), |recorder| {
        filled_square(recorder, srgb(1.0, 0.0, 0.0));
        recorder.stroke(
            Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.05),
            waterui_graphics::draw::Stroke::new(1.0),
            srgb(0.0, 0.0, 1.0),
        );
    });
    assert_eq!(content.len(), 2);
}

#[test]
fn a_clip_scopes_the_commands_inside_it() {
    // A fill covering the whole surface, clipped to the left half: the right
    // half must come back transparent, because the clip is a scope boundary,
    // not a hint.
    let recording = Picture::record(|recorder| {
        recorder.clip(Rect::new(0.0, 0.0, 4.0, 8.0).to_path(0.05), |scene| {
            scene.fill(
                Rect::new(0.0, 0.0, 8.0, 8.0).to_path(0.05),
                srgb(1.0, 0.0, 0.0),
            );
        });
    });
    let bitmap = Rasterizer::new(8, 8)
        .expect("engine failed to start")
        .rasterize(&recording, Affine::IDENTITY)
        .expect("rasterise failed");
    let left = pixel(&bitmap, 1, 4);
    let right = pixel(&bitmap, 7, 4);
    assert!(left[0] > 250, "clipped-in pixel should be red: {left:?}");
    assert_eq!(
        right,
        [0, 0, 0, 0],
        "the clip let its body leak right of x=4"
    );
}

fn tiny_image() -> ImageData<Rgba8> {
    // 2×1: left texel red, right texel blue, premultiplied.
    let bytes = [255, 0, 0, 255, 0, 0, 255, 255];
    ImageData::<Rgba8>::new(2, 1, Vec::from(bytes))
        .expect("valid image")
        .color_space(ImageColorSpace::Srgb)
        .premultiplied()
}

struct TinyImageSource {
    bytes: Arc<[u8]>,
}

impl PictureSource for TinyImageSource {
    fn record(&self, recorder: &mut StaticRecorder, resources: &mut RecordingResources<'_>) {
        let image = resources
            .image(
                ImageData::<Rgba8>::new(2, 1, Arc::clone(&self.bytes))
                    .expect("image source")
                    .color_space(ImageColorSpace::Srgb)
                    .premultiplied(),
            )
            .expect("image registration failed");
        recorder.image(
            resources.name(&image),
            Rect::new(0.0, 0.0, 8.0, 8.0),
            Sampling::Nearest,
        );
    }
}

struct GlyphSource {
    bytes: Arc<[u8]>,
    glyph: u32,
}

impl PictureSource for GlyphSource {
    fn record(&self, recorder: &mut StaticRecorder, resources: &mut RecordingResources<'_>) {
        let font = resources
            .font(FontSource::bytes(Arc::clone(&self.bytes)))
            .expect("font registration failed");
        recorder.glyphs(
            GlyphRun {
                font: resources.name(&font),
                size: 28.0,
                coords: Vec::new().into(),
                glyphs: vec![Glyph {
                    id: self.glyph,
                    x: 4.0,
                    y: 28.0,
                    transform: None,
                }]
                .into(),
                style: GlyphStyle::Fill,
            },
            srgb(1.0, 1.0, 1.0),
        );
    }
}

#[test]
fn image_data_rejects_zero_size_and_byte_length_mismatch() {
    assert!(ImageData::<Rgba8>::new(0, 1, Vec::from([0u8; 4])).is_err());
    assert!(ImageData::<Rgba8>::new(1, 0, Vec::from([0u8; 4])).is_err());
    // A 2×1 RGBA8 image wants exactly 8 bytes.
    assert!(ImageData::<Rgba8>::new(2, 1, Vec::from([0u8; 4])).is_err());
    assert!(ImageData::<Rgba8>::new(2, 1, Vec::from([0u8; 12])).is_err());
}

#[test]
fn an_image_draws_where_the_recording_put_it() {
    let mut rasterizer = Rasterizer::new(8, 8).expect("engine failed to start");
    let source = Arc::<[u8]>::from([255, 0, 0, 255, 0, 0, 255, 255]);
    let recording = Picture::record_with(rasterizer.resources(), TinyImageSource { bytes: source });
    let bitmap = rasterizer
        .rasterize(&recording, Affine::IDENTITY)
        .expect("rasterise failed");
    let left = pixel(&bitmap, 2, 4);
    let right = pixel(&bitmap, 6, 4);
    assert!(left[0] > 200 && left[2] < 40, "left texel: {left:?}");
    assert!(right[2] > 200 && right[0] < 40, "right texel: {right:?}");
}

/// Raw data of an installed font face: a [`FontSource`] carries font bytes,
/// and the repository commits none, so the recording tests resolve a family
/// through system font discovery — the same lookup a test runtime makes for
/// a named family. `backends/hydrolysis/test-fonts/install.py` installs the
/// fixtures; a missing family fails naming it and the script.
fn installed_font_bytes(family_name: &str) -> Arc<[u8]> {
    let mut collection = fontique::Collection::new(fontique::CollectionOptions::default());
    let family = collection.family_by_name(family_name).unwrap_or_else(|| {
        panic!(
            "font family `{family_name}` is not installed; install the test fonts \
             with `uv run backends/hydrolysis/test-fonts/install.py`"
        )
    });
    let font = family
        .default_font()
        .expect("the resolved family carries no default face");
    Arc::from(
        font.load(None)
            .expect("the resolved face failed to load")
            .as_ref(),
    )
}

#[test]
fn a_glyph_run_draws_its_glyphs() {
    let source = installed_font_bytes("Roboto");

    let face = ttf_parser::Face::parse(source.as_ref(), 0).expect("test font did not parse");
    let glyph = face.glyph_index('A').expect("the test font has no 'A'").0;

    let mut rasterizer = Rasterizer::new(32, 32).expect("engine failed to start");
    let recording = Picture::record_with(
        rasterizer.resources(),
        GlyphSource {
            bytes: source,
            glyph: u32::from(glyph),
        },
    );
    let bitmap = rasterizer
        .rasterize(&recording, Affine::IDENTITY)
        .expect("rasterise failed");
    assert!(
        bitmap.data().as_chunks::<4>().0.iter().any(|px| px[3] > 0),
        "the glyph run produced no ink"
    );
}

#[test]
fn a_held_source_registers_once_and_a_released_one_registers_fresh() {
    let font_bytes = installed_font_bytes("Roboto");

    let rasterizer = Rasterizer::new(8, 8).expect("engine failed to start");
    let resources = rasterizer.resources();
    let (first_id, image_id) = {
        let first = resources
            .font(FontSource::bytes(Arc::clone(&font_bytes)))
            .expect("font registration failed");
        let second = resources
            .font(FontSource::bytes(Arc::<[u8]>::from(&font_bytes[..])))
            .expect("font registration failed");
        assert_eq!(
            first, second,
            "the same font source must mint one engine registration while it is held"
        );

        // Same rule for images: identity of the upload is the data.
        let image_a = resources
            .image(tiny_image())
            .expect("image registration failed");
        let image_b = resources
            .image(tiny_image())
            .expect("image registration failed");
        assert_eq!(image_a, image_b);
        let mut names = resources.recording();
        (names.name(&first), names.name(&image_a))
        // Every handle, and the recording that named them, drops here; the
        // table stays.
    };

    // The table outlives the handles but never kept them: the released
    // registrations are gone, so the same sources register fresh.
    let third = resources
        .font(FontSource::bytes(Arc::clone(&font_bytes)))
        .expect("font registration failed");
    let image_c = resources
        .image(tiny_image())
        .expect("image registration failed");
    let mut names = resources.recording();
    assert_ne!(
        first_id,
        names.name(&third),
        "a released registration must not come back"
    );
    assert_ne!(image_id, names.name(&image_c));
}

/// Content that draws nothing on its first two frames and first draws an
/// image on its third, registering it in the frame that draws it.
struct LateImage {
    frame: u32,
    image: Option<Registered<ImageId>>,
}

impl SceneContent for LateImage {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> bool {
        self.frame += 1;
        if self.frame >= 3 {
            let image = self.image.get_or_insert_with(|| {
                resources
                    .image(tiny_image())
                    .expect("image registration failed")
            });
            recorder.image(
                resources.name(image),
                Rect::new(0.0, 0.0, 8.0, 8.0),
                Sampling::Nearest,
            );
        }
        false
    }

    fn rebuild_for_engine(&mut self) {
        self.image = None;
    }
}

#[test]
fn an_image_first_drawn_on_the_third_frame_reaches_the_pixels() {
    let mut rasterizer = Rasterizer::new(8, 8).expect("engine failed to start");
    let mut content = LateImage {
        frame: 0,
        image: None,
    };
    let mut frame = |content: &mut LateImage| {
        rasterizer
            .rasterize_scene(content, 1.0)
            .expect("rasterise failed")
    };

    for _ in 0..2 {
        let bitmap = frame(&mut content);
        assert!(bitmap.data().iter().all(|byte| *byte == 0));
        assert!(content.image.is_none());
    }
    let bitmap = frame(&mut content);
    let left = pixel(&bitmap, 2, 4);
    let right = pixel(&bitmap, 6, 4);
    assert!(
        left[0] > 200 && left[2] < 40,
        "the late image did not draw: {left:?}"
    );
    assert!(right[2] > 200 && right[0] < 40, "right texel: {right:?}");

    // A SceneView keeps the content's contract — the view is the carrier,
    // the content is the thing that registered.
    let view = SceneView::new(content);
    assert!(view.accessibility_label().is_none());
}

#[test]
fn a_signal_operand_updates_the_content_without_recording_again() {
    let color = Binding::container(srgb(1.0, 0.0, 0.0));
    let paint = color.computed();
    let mut content = Content::record(&LayoutSize::new(), |recorder| {
        recorder.fill(Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.05), paint);
    });

    let Some(ContentChange::Replace(_)) = content.take_change() else {
        panic!("a finished recording reports its initial replacement");
    };
    color.set(srgb(0.0, 1.0, 0.0));
    let Some(ContentChange::Update(_)) = content.take_change() else {
        panic!("a bound operand must report an incremental update, not a re-record");
    };
    assert_eq!(
        content.take_change(),
        None,
        "nothing changed after the set was consumed"
    );
}

#[test]
fn an_invalidator_fires_when_a_bound_signal_changes() {
    let flag = Rc::new(Cell::new(false));
    let asked = Rc::clone(&flag);
    let invalidator: waterui_graphics::SceneInvalidator = Rc::new(move || asked.set(true));

    let color = Binding::container(srgb(1.0, 0.0, 0.0));
    let _guard = waterui_graphics::invalidate_on_change(&invalidator, &color.computed());
    color.set(srgb(0.0, 0.0, 1.0));
    assert!(
        flag.get(),
        "a signal change did not reach the scene invalidator"
    );
}
