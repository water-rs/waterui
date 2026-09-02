//! Both scene engines draw an image brush, and draw it the same way.
//!
//! The hybrid engine keeps images in an atlas of its own rather than carrying
//! decoded pixels along with the scene, so `draw_image` is the one command
//! whose translation between the two engines is not a rename. This renders the
//! same recording through each engine and compares the results: the image has
//! to land in the same place, the right way up, in the same colours.

use std::sync::Arc;

use kurbo::Affine;
use peniko::{Blob, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageSampler};
use waterui_graphics::shared_context::SceneEngine;
use waterui_graphics::{
    GpuRuntime, OffscreenRenderConfig, OffscreenSize, Scene2D, SceneContent, SceneView,
};
use waterui_testing::artifact_root;

/// Side of the source image, in pixels.
const IMAGE_SIDE: u32 = 8;
/// How many output pixels one source pixel covers.
const SCALE: u32 = 25;
/// Side of the rendered output, in pixels.
const OUTPUT_SIDE: u32 = IMAGE_SIDE * SCALE;
/// How far a channel may sit from what it is compared against.
///
/// The engines rasterize differently and neither promises a bit-exact texel:
/// classic lands a couple of levels off its own source colour. The margin is
/// small enough that a swapped channel, a transposed image or a blended-in
/// neighbour is still a failure — those are off by whole colours.
const CHANNEL_TOLERANCE: u8 = 4;

/// The source colour of pixel `(x, y)`.
///
/// No two pixels share a colour and the three channels run in three different
/// directions, so a transposed, mirrored or channel-swapped blit shows up as a
/// wrong colour rather than as a wrong shade.
fn source_pixel(x: u32, y: u32) -> [u8; 4] {
    let index =
        u8::try_from(y * IMAGE_SIDE + x).expect("the source image has fewer than 256 pixels");
    [index * 3, 255 - index * 3, index.wrapping_mul(7), 255]
}

/// Scene content that draws one image and nothing else.
struct ImageContent {
    brush: ImageBrush,
    transform: Affine,
}

impl SceneContent for ImageContent {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, _width: f32, _height: f32) -> bool {
        scene.draw_image(&self.brush, self.transform);
        false
    }
}

fn source_image() -> ImageBrush {
    let mut data = Vec::with_capacity((IMAGE_SIDE * IMAGE_SIDE * 4) as usize);
    for y in 0..IMAGE_SIDE {
        for x in 0..IMAGE_SIDE {
            data.extend_from_slice(&source_pixel(x, y));
        }
    }
    ImageBrush {
        image: ImageData {
            data: Blob::new(Arc::new(data)),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: IMAGE_SIDE,
            height: IMAGE_SIDE,
        },
        sampler: ImageSampler::default(),
    }
}

fn pixel_at(rgba8: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * OUTPUT_SIDE + x) * 4) as usize;
    rgba8[offset..offset + 4]
        .try_into()
        .expect("a four-byte pixel must be readable at an in-bounds offset")
}

fn channels_off_by(left: [u8; 4], right: [u8; 4]) -> u8 {
    left.iter()
        .zip(&right)
        .map(|(left, right)| left.abs_diff(*right))
        .max()
        .expect("a pixel has four channels")
}

fn render(runtime: &GpuRuntime, engine: SceneEngine) -> Vec<u8> {
    let size =
        OffscreenSize::try_from_pixels(OUTPUT_SIDE, OUTPUT_SIDE).expect("test size must be valid");
    let surface = SceneView::new(ImageContent {
        brush: source_image(),
        transform: Affine::scale(f64::from(SCALE)),
    })
    .into_gpu_surface();
    let config = OffscreenRenderConfig::new(size)
        .format(wgpu::TextureFormat::Rgba8Unorm)
        .scene_engine(engine);
    let mut env = waterui_core::Environment::new();
    let output = pollster::block_on(surface.render_offscreen(runtime, config, &mut env))
        .expect("offscreen render should succeed");

    let directory = artifact_root().join("scene_image");
    std::fs::create_dir_all(&directory).expect("artifact directory must be creatable");
    let name = match engine {
        SceneEngine::Classic => "classic",
        SceneEngine::Hybrid => "hybrid",
    };
    output
        .save_png(directory.join(format!("{name}.png")))
        .expect("snapshot PNG must be writable");
    output.rgba8
}

/// The final source pixel's row and column, in output pixels.
///
/// `vello_hybrid`'s bilinear image sampling clamps `Extend::Pad` to the last
/// texel's leading corner (`clamp(t, 0.0, size - 1.0)` in `render.wesl`) and
/// only then subtracts the half texel that turns a corner into a centre, so
/// every sample from that corner onwards resolves to one fixed half-and-half
/// blend of the last two texels instead of running on into the last one. The
/// band is excluded from the comparison rather than asserted, because
/// asserting it would pin the defect in place; see water-rs/waterui#234.
const TRAILING_BAND: u32 = SCALE;

/// An image brush draws the same picture through either engine.
///
/// The engine is forced rather than left to the adapter, so the hybrid path is
/// covered on a device that would have chosen the classic one — which is every
/// device that can run the compute pipeline.
#[test]
fn both_scene_engines_draw_an_image_brush() {
    let runtime = pollster::block_on(GpuRuntime::new())
        .expect("scene engine comparison requires a working GPU runtime");

    // The engine the device picks for itself is not what this test renders
    // with, but it is what an application on this machine gets, so it is
    // recorded next to the snapshots.
    let directory = artifact_root().join("scene_image");
    std::fs::create_dir_all(&directory).expect("artifact directory must be creatable");
    let adapter = runtime.context().adapter.get_info();
    std::fs::write(
        directory.join("adapter.txt"),
        format!(
            "adapter: {}\nbackend: {:?}\nselected engine: {:?}\n",
            adapter.name,
            adapter.backend,
            runtime.context().scene_renderer().engine(),
        ),
    )
    .expect("adapter report must be writable");

    let classic = render(&runtime, SceneEngine::Classic);
    let hybrid = render(&runtime, SceneEngine::Hybrid);

    // Every source pixel's own centre, which both engines sample to that
    // pixel's colour under either filtering quality. This is what says the
    // image is in the right place, the right way up and in the right colours
    // rather than merely identical to itself.
    let last_centre = IMAGE_SIDE - 1;
    for y in 0..IMAGE_SIDE {
        for x in 0..IMAGE_SIDE {
            if x == last_centre || y == last_centre {
                continue;
            }
            let expected = source_pixel(x, y);
            let point = (x * SCALE + SCALE / 2, y * SCALE + SCALE / 2);
            for (engine, rgba8) in [("classic", &classic), ("hybrid", &hybrid)] {
                let actual = pixel_at(rgba8, point.0, point.1);
                assert!(
                    channels_off_by(actual, expected) <= CHANNEL_TOLERANCE,
                    "the {engine} engine drew {actual:?} at {point:?}, where source pixel \
                     ({x}, {y}) puts {expected:?}"
                );
            }
        }
    }

    // And the whole picture, so a difference anywhere between the sampled
    // centres is a failure too.
    let compared = OUTPUT_SIDE - TRAILING_BAND;
    for y in 0..compared {
        for x in 0..compared {
            let classic = pixel_at(&classic, x, y);
            let hybrid = pixel_at(&hybrid, x, y);
            assert!(
                channels_off_by(classic, hybrid) <= CHANNEL_TOLERANCE,
                "the engines disagree at ({x}, {y}): classic drew {classic:?}, \
                 hybrid drew {hybrid:?}"
            );
        }
    }
}
