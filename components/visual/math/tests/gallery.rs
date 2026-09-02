//! Renders a gallery of formulas for visual review.
//!
//! These are not pass/fail assertions about appearance — the two engines
//! rasterize differently, and no threshold on pixels can tell you whether a
//! fraction bar is in the right place. The PNGs are written out to be *looked
//! at*, by a person or by an agent with vision.
//!
//! Rendering every formula through both scene engines is the part that is
//! asserted: a formula that draws on the classic compute pipeline but not on
//! the CPU/GPU split engine would be broken on the iOS Simulator and on every
//! adapter without indirect execution, and that is exactly the failure this
//! crate's `Scene2D` output exists to avoid.
//!
//! ```text
//! WATERUI_MATH_GALLERY_DIR=/tmp/waterui_math_gallery \
//!   cargo test -p waterui-math --test gallery -- --nocapture
//! ```

use std::path::{Path, PathBuf};

use kurbo::{Affine, Rect, Shape};
use peniko::{Brush, Color, Fill};
use waterui_graphics::shared_context::SceneEngine;
use waterui_graphics::{
    GpuRuntime, OffscreenRenderConfig, OffscreenSize, Scene2D, SceneContent, SceneInvalidator,
    SceneView,
};
use waterui_math::ast::MathStyle;
use waterui_math::view::{DEFAULT_MATH_FAMILY, MathContent};

/// Paints an opaque ground under the formula.
///
/// The renderer leaves the canvas transparent, and a transparent PNG of black
/// glyphs is invisible in half the viewers someone might open it in. A review
/// image nobody can see is not a review image.
struct OnWhite {
    formula: MathContent,
}

impl SceneContent for OnWhite {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool {
        let ground = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Brush::Solid(Color::WHITE),
            None,
            &ground.to_path(0.1),
        );
        self.formula.build_scene(scene, width, height)
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.formula.set_invalidator(invalidator);
    }
}

/// Formulas chosen to exercise each construct the layout engine implements,
/// and the ones an earlier attempt got visibly wrong.
const GALLERY: &[(&str, &str)] = &[
    // Spacing: the gaps around `+` and `=` must differ, and differ from none.
    ("spacing", r"a+b=c"),
    // Fractions, including a nested one that must shrink.
    ("fraction", r"\frac{a+b}{c}"),
    ("fraction_nested", r"\frac{1}{1+\frac{1}{1+x}}"),
    // Scripts, including one on a slanted base where italic correction shows.
    ("scripts", r"x^2 + y_i - z_n^2"),
    ("scripts_nested", r"e^{x^{2}}"),
    // Radicals: a plain one, one over a fraction (the tall case), and an index.
    ("radical", r"\sqrt{x}"),
    ("radical_tall", r"\sqrt{\frac{a+b}{c+d}}"),
    ("radical_index", r"\sqrt[3]{x}"),
    // Stretchy fences around something tall.
    ("fences", r"\left(\frac{a}{b}\right)"),
    // Greek, including letters an earlier hand-written table was missing.
    ("greek", r"\alpha\beta\psi\eta\tau\xi\zeta"),
    // Upright function names next to italic variables.
    ("functions", r"\sin x + \log y"),
    // Literal text keeps its spaces and stays upright.
    ("text", r"\text{if } x > 0"),
    // Large operators with limits.
    ("sum", r"\sum_{i=1}^{n} i"),
    // A formula combining most of the above.
    ("quadratic", r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}"),
];

fn output_directory() -> PathBuf {
    std::env::var("WATERUI_MATH_GALLERY_DIR").map_or_else(
        |_| PathBuf::from("/tmp/waterui_math_gallery"),
        PathBuf::from,
    )
}

#[test]
fn renders_the_formula_gallery_on_both_scene_engines() {
    let directory = output_directory();
    std::fs::create_dir_all(&directory).expect("gallery directory must be creatable");

    let runtime = pollster::block_on(GpuRuntime::new())
        .expect("the formula gallery requires a working GPU runtime");
    let size = OffscreenSize::try_from_pixels(560, 200).expect("gallery size must be valid");

    let mut written = Vec::new();
    for (name, source) in GALLERY {
        for (engine, engine_name) in [
            (SceneEngine::Classic, "classic"),
            (SceneEngine::Hybrid, "hybrid"),
        ] {
            let content = MathContent::new(
                waterui_str::Str::from(*source),
                48.0,
                MathStyle::Display,
                DEFAULT_MATH_FAMILY,
                Brush::Solid(Color::BLACK),
            );
            let surface = SceneView::new(OnWhite { formula: content }).into_gpu_surface();
            let config = OffscreenRenderConfig::new(size)
                .format(wgpu::TextureFormat::Rgba8Unorm)
                .scene_engine(engine);
            let mut env = waterui_core::Environment::new();
            let output = pollster::block_on(surface.render_offscreen(&runtime, config, &mut env))
                .unwrap_or_else(|error| {
                    panic!("`{source}` must render on the {engine_name} engine: {error}")
                });

            let path = directory.join(format!("{name}_{engine_name}.png"));
            output
                .save_png(&path)
                .expect("gallery PNG must be writable");
            written.push(path);
        }
    }

    write_index(&directory);
    println!(
        "wrote {} formula renderings to {}",
        written.len(),
        directory.display()
    );
}

/// A companion index so whoever reviews the images knows what each one is
/// supposed to be.
fn write_index(directory: &Path) {
    let mut lines = Vec::with_capacity(GALLERY.len());
    for (name, source) in GALLERY {
        lines.push(format!("{name}: {source}"));
    }
    std::fs::write(directory.join("index.txt"), lines.join("\n"))
        .expect("gallery index must be writable");
}
