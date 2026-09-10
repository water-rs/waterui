//! Diagram layout measured with the same text engine that draws the labels.
//!
//! `merman-render` sizes every node, every edge label and every participant box
//! from a [`TextMeasurer`], and ships built-in profiles that approximate a
//! browser's metrics. Those profiles are the right default for a headless SVG
//! writer, which has no font stack to ask. We do have one — the labels in a
//! rendered diagram are `WaterUI` text views shaped by `parley` — so leaving the
//! built-in profile installed would size boxes from one set of metrics and paint
//! glyphs with another, and the text would not fit the box that was reserved
//! for it.
//!
//! So the measurer here is the same `parley` engine, reading the same faces the
//! rest of the application shapes with: the one [`FontCollection`] the host
//! installed, borrowed for the duration of a measurement and never copied.

use alloc::sync::Arc;
use core::cell::RefCell;

use merman_render::environment::{
    MeasurementProfileId, TextMeasurementPolicy, TextMeasurementProfile,
    TextMeasurementProfileIdentity,
};
use merman_render::text::{TextMeasurer, TextMetrics, TextStyle};
use parley::{Alignment, AlignmentOptions, FontStyle, FontWeight, LayoutContext, StyleProperty};
use waterui_core::MainThreadBound;
use waterui_text::FontCollection;

/// The identity `merman-render` records against measurements taken here.
///
/// It is provenance, not configuration: a layout carries the name of whatever
/// measured it, so a diagram laid out with our metrics is never mistaken for one
/// laid out against a browser-compatibility profile.
const PROFILE: &str = "waterui-parley";

/// Text measurement backed by the application's shared font collection.
///
/// # Threading
///
/// `merman-render` takes its measurer as `Arc<dyn TextMeasurer + Send + Sync>`,
/// because a host may lay several diagrams out in parallel — `merman-cli` does
/// exactly that when it renders the diagrams of one Markdown file across a
/// `rayon` pool. `WaterUI` is not that host: a diagram is laid out on the thread
/// that owns the view, from the collection that thread shares with every other
/// component. [`MainThreadBound`] is what states that in the type system —
/// the measurer satisfies the bound `merman` asks for, and asserts on every
/// access that it is still on the thread it was built on. It takes no lock,
/// because there is no second thread to exclude.
struct Measurer {
    shaping: MainThreadBound<Shaping>,
}

/// The collection a measurement shapes against, and the layout arena it reuses.
///
/// The collection is borrowed, not owned: it is the one the host installed and
/// every other component reads, so a diagram's boxes are measured from the very
/// faces its labels will be painted with.
struct Shaping {
    fonts: FontCollection,
    layout: RefCell<LayoutContext<[u8; 4]>>,
}

impl Measurer {
    /// Measures against `fonts`, binding the measurer to the calling thread.
    fn new(fonts: FontCollection) -> Self {
        Self {
            shaping: MainThreadBound::new(Shaping {
                fonts,
                layout: RefCell::new(LayoutContext::new()),
            }),
        }
    }
}

impl core::fmt::Debug for Measurer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Measurer")
    }
}

impl TextMeasurer for Measurer {
    fn measure(&self, text: &str, style: &TextStyle) -> TextMetrics {
        if text.is_empty() {
            return TextMetrics {
                width: 0.0,
                height: 0.0,
                line_count: 0,
            };
        }

        // Shaping is the only part that needs the collection and the arena.
        // `Layout` is owned once it is built, so line breaking and measurement
        // happen after both borrows have ended.
        let shaping = &*self.shaping;
        let mut built = shaping.fonts.use_fonts(|fonts| {
            let mut layout = shaping.layout.borrow_mut();
            let mut builder = layout.ranged_builder(fonts, text, 1.0, true);
            builder.push_default(StyleProperty::FontSize(f32::from_f64_lossless(
                style.font_size,
            )));
            // `style.font_family` is Mermaid's own CSS stack — `"trebuchet ms",
            // verdana, arial, sans-serif` by default — and it is deliberately
            // not pushed. A diagram's labels are painted in the application's
            // font, the way its colours come from theme tokens rather than
            // Mermaid's CSS themes, so measuring against a family nothing will
            // paint with is what makes a box too small for its own text.
            if let Some(weight) = style.font_weight.as_deref() {
                builder.push_default(StyleProperty::FontWeight(parse_weight(weight)));
            }
            if let Some(font_style) = style.font_style.as_deref() {
                builder.push_default(StyleProperty::FontStyle(parse_style(font_style)));
            }

            builder.build(text)
        });
        built.break_all_lines(None);
        built.align(Alignment::Start, AlignmentOptions::default());

        TextMetrics {
            width: f64::from(built.full_width()),
            height: f64::from(built.height()),
            line_count: built.len().max(1),
        }
    }
}

/// Parses a CSS font weight the way Mermaid's style strings spell it.
///
/// An unrecognised spelling is `normal`, matching CSS: a weight is a
/// presentation hint, and refusing to draw a diagram because a stylesheet said
/// `font-weight: bolder` would be the wrong trade.
fn parse_weight(weight: &str) -> FontWeight {
    match weight.trim() {
        "bold" => FontWeight::BOLD,
        "lighter" => FontWeight::LIGHT,
        "bolder" => FontWeight::EXTRA_BOLD,
        numeric => numeric
            .parse::<f32>()
            .map_or(FontWeight::NORMAL, FontWeight::new),
    }
}

/// Parses a CSS font style, defaulting to upright for the same reason as
/// [`parse_weight`].
fn parse_style(style: &str) -> FontStyle {
    match style.trim() {
        "italic" => FontStyle::Italic,
        "oblique" => FontStyle::Oblique(None),
        _ => FontStyle::Normal,
    }
}

/// Lossless `f64` -> `f32` for the font sizes Mermaid deals in.
pub trait FromF64Lossless {
    fn from_f64_lossless(value: f64) -> Self;
}

impl FromF64Lossless for f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "font sizes are small positive magnitudes; f32 represents every one Mermaid emits"
    )]
    fn from_f64_lossless(value: f64) -> Self {
        value as Self
    }
}

/// The text style a diagram's labels are painted with.
///
/// A function of the diagram's font size alone, and deliberately not of a
/// label's prominence. `merman` measured every label through [`policy`] at that
/// size, and the box it reserved holds exactly the text that measurement
/// described. A paint layer that shrank a title by an eighth and set it bold
/// was reserving one metric and drawing another: the bold advance stayed inside
/// the smaller box on this machine's faces and overflowed it by a quarter of a
/// pixel on CI's, which is how `Ingest` came to be painted wider than the box
/// reserved for it. Prominence is a colour, never a size.
pub fn label_style(font_size: f32) -> TextStyle {
    TextStyle {
        font_family: None,
        font_size: f64::from(font_size),
        font_weight: None,
        font_style: None,
    }
}

/// The measurement policy a diagram laid out against `fonts` is measured under.
///
/// Installed on the render environment for every family, so a flowchart's node
/// boxes and a sequence diagram's participant boxes are sized by the same
/// collection that will draw their labels.
pub fn policy(fonts: FontCollection) -> TextMeasurementPolicy {
    let identity = TextMeasurementProfileIdentity::new(
        MeasurementProfileId::new(PROFILE).expect("the profile id is a valid identifier"),
        env!("CARGO_PKG_VERSION"),
    )
    .expect("the profile identity is well formed");

    TextMeasurementPolicy::uniform(TextMeasurementProfile::new(
        identity,
        Arc::new(Measurer::new(fonts)),
    ))
}

/// Measures `text` against `fonts` exactly as a diagram's layout does.
///
/// This is the layout half of the agreement the module exists to keep, exposed
/// so a test can hold the box a diagram reserved next to the text that will be
/// painted into it.
#[cfg(test)]
pub fn measure(fonts: FontCollection, text: &str, style: &TextStyle) -> TextMetrics {
    Measurer::new(fonts).measure(text, style)
}
