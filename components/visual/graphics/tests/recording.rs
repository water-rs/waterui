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

use cherenkov::kurbo::{Affine, Rect, Shape};
use cherenkov::{
    ContentChange, Draw, FontSource, Glyph, GlyphRun, GlyphStyle, ImageColorSpace, ImageData,
    Recorder, Rgba8, Sampling, WorkingColor,
};
use nami::{Binding, SignalExt};
use waterui_graphics::raster::{Rasterizer, RgbaBitmap};
use waterui_graphics::{SceneContent, SceneResources, SceneView};

/// sRGB red as the engine's working colour — `WorkingColor::new` names
/// Display P3 components directly, which sRGB cannot hold, so pixel
/// expectations come from converting a named sRGB colour.
fn srgb(red: f32, green: f32, blue: f32) -> WorkingColor {
    cherenkov::Color::<cherenkov::Srgb>::new([red, green, blue, 1.0]).into()
}

fn filled_square(color: WorkingColor) -> Recorder {
    let mut recorder = Recorder::new();
    recorder.fill(Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.05), color);
    recorder
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
    let mut recorder = filled_square(srgb(1.0, 0.0, 0.0));
    recorder.stroke(
        Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.05),
        cherenkov::Stroke::new(1.0),
        srgb(0.0, 0.0, 1.0),
    );
    let content = recorder.finish();
    assert_eq!(content.len(), 2);
}

#[test]
fn a_clip_scopes_the_commands_inside_it() {
    // A fill covering the whole surface, clipped to the left half: the right
    // half must come back transparent, because the clip is a scope boundary,
    // not a hint.
    let mut recorder = Recorder::new();
    recorder.clip(Rect::new(0.0, 0.0, 4.0, 8.0).to_path(0.05), |scene| {
        scene.fill(
            Rect::new(0.0, 0.0, 8.0, 8.0).to_path(0.05),
            srgb(1.0, 0.0, 0.0),
        );
    });
    let content = recorder.finish();
    let bitmap = Rasterizer::new(8, 8)
        .expect("engine failed to start")
        .rasterize(content, Affine::IDENTITY)
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
    let image = rasterizer
        .resources()
        .image(tiny_image())
        .expect("image registration failed");

    let mut recorder = Recorder::new();
    // The image's own rect fills its destination: left half red, right blue.
    recorder.image(image.id(), Rect::new(0.0, 0.0, 8.0, 8.0), Sampling::Nearest);
    let bitmap = rasterizer
        .rasterize(recorder.finish(), Affine::IDENTITY)
        .expect("rasterise failed");
    let left = pixel(&bitmap, 2, 4);
    let right = pixel(&bitmap, 6, 4);
    assert!(left[0] > 200 && left[2] < 40, "left texel: {left:?}");
    assert!(right[2] > 200 && right[0] < 40, "right texel: {right:?}");
}

#[test]
fn a_glyph_run_draws_its_glyphs() {
    const FONT: &[u8] = include_bytes!("../../../../testing/fonts/Roboto-Regular.ttf");

    let face = ttf_parser::Face::parse(FONT, 0).expect("test font did not parse");
    let glyph = face.glyph_index('A').expect("the test font has no 'A'").0;

    let mut rasterizer = Rasterizer::new(32, 32).expect("engine failed to start");
    let font = rasterizer
        .resources()
        .font(FontSource::bytes(Arc::<[u8]>::from(FONT)))
        .expect("font registration failed");

    let mut recorder = Recorder::new();
    recorder.glyphs(
        GlyphRun {
            font: font.id(),
            size: 28.0,
            coords: Vec::new(),
            glyphs: vec![Glyph {
                id: u32::from(glyph),
                x: 4.0,
                y: 28.0,
                transform: None,
            }],
            style: GlyphStyle::Fill,
        },
        srgb(1.0, 1.0, 1.0),
    );
    let bitmap = rasterizer
        .rasterize(recorder.finish(), Affine::IDENTITY)
        .expect("rasterise failed");
    assert!(
        bitmap.data().as_chunks::<4>().0.iter().any(|px| px[3] > 0),
        "the glyph run produced no ink"
    );
}

#[test]
fn the_same_source_registers_once_and_a_dropped_mount_registers_fresh() {
    const FONT: &[u8] = include_bytes!("../../../../testing/fonts/Roboto-Regular.ttf");

    let rasterizer = Rasterizer::new(8, 8).expect("engine failed to start");
    let (first_id, image_id) = {
        let resources = rasterizer.resources();
        let first = resources
            .font(FontSource::bytes(Arc::<[u8]>::from(FONT)))
            .expect("font registration failed");
        let second = resources
            .font(FontSource::bytes(Arc::<[u8]>::from(FONT)))
            .expect("font registration failed");
        assert_eq!(
            first.id(),
            second.id(),
            "the same font source must mint one engine registration"
        );

        // Same rule for images: identity of the upload is the data.
        let image_a = resources
            .image(tiny_image())
            .expect("image registration failed");
        let image_b = resources
            .image(tiny_image())
            .expect("image registration failed");
        assert_eq!(image_a.id(), image_b.id());
        (first.id(), image_a.id())
        // `resources` drops here with the handles — the mount is over.
    };

    // A new mount registers fresh: nothing carried the old registrations
    // forward, because carrying them is what the mount was for.
    let resources = rasterizer.resources();
    let third = resources
        .font(FontSource::bytes(Arc::<[u8]>::from(FONT)))
        .expect("font registration failed");
    assert_ne!(
        first_id,
        third.id(),
        "a detached registration must not come back"
    );
    let image_c = resources
        .image(tiny_image())
        .expect("image registration failed");
    assert_ne!(image_id, image_c.id());
}

/// Content that needs a font and an image registered against the engine it
/// will draw on — the shape of every real `SceneContent` with resources.
struct PreparedImage {
    image: Option<cherenkov::Image<Rgba8>>,
    prepared: bool,
}

impl SceneContent for PreparedImage {
    fn prepare_resources(&mut self, resources: &SceneResources) {
        self.image = Some(
            resources
                .image(tiny_image())
                .expect("image registration failed"),
        );
        self.prepared = true;
    }

    fn build_scene(&mut self, recorder: &mut Recorder, _width: f32, _height: f32) -> bool {
        let image = self.image.as_ref().expect("prepare_resources did not run");
        recorder.image(image.id(), Rect::new(0.0, 0.0, 8.0, 8.0), Sampling::Nearest);
        false
    }
}

#[test]
fn mounted_content_records_the_handles_its_prepare_registered() {
    let mut rasterizer = Rasterizer::new(8, 8).expect("engine failed to start");
    let mut content = PreparedImage {
        image: None,
        prepared: false,
    };
    // The mount hook hands content the engine's own resource table; handles
    // minted anywhere else are not this engine's.
    content.prepare_resources(&rasterizer.resources());
    assert!(content.prepared);

    let mut recorder = Recorder::new();
    assert!(!content.build_scene(&mut recorder, 8.0, 8.0));
    let bitmap = rasterizer
        .rasterize(recorder.finish(), Affine::IDENTITY)
        .expect("rasterise failed");
    let left = pixel(&bitmap, 2, 4);
    assert!(left[0] > 200, "mounted image did not draw: {left:?}");

    // A SceneView keeps the content's contract — the view is the carrier, the
    // content is the thing that was prepared.
    let view = SceneView::new(content);
    assert!(view.accessibility_label().is_none());
}

#[test]
fn a_signal_operand_updates_the_content_without_recording_again() {
    let color = Binding::container(srgb(1.0, 0.0, 0.0));
    let paint = color.computed();
    let mut recorder = Recorder::new();
    recorder.fill(Rect::new(0.0, 0.0, 4.0, 4.0).to_path(0.05), paint);
    let mut content = recorder.finish();

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
