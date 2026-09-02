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
//! So the measurer here is the same `parley` engine, reading the same system
//! font source `waterui-canvas` reads.

use alloc::sync::Arc;
use std::sync::Mutex;

use merman_render::environment::{
    MeasurementProfileId, TextMeasurementPolicy, TextMeasurementProfile,
    TextMeasurementProfileIdentity,
};
use merman_render::text::{TextMeasurer, TextMetrics, TextStyle};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, LayoutContext,
    StyleProperty,
};

/// The identity `merman-render` records against measurements taken here.
///
/// It is provenance, not configuration: a layout carries the name of whatever
/// measured it, so a diagram laid out with our metrics is never mistaken for one
/// laid out against a browser-compatibility profile.
const PROFILE: &str = "waterui-parley";

/// Text measurement backed by `parley` and the system font source.
///
/// # Threading
///
/// `merman-render` takes its measurer as `Arc<dyn TextMeasurer + Send + Sync>`,
/// while `parley`'s contexts are `Send` but not `Sync`. The mutex here is what
/// bridges the two. It is never contended: layout runs on the thread that owns
/// the view, one diagram at a time, and the lock is taken and released inside a
/// single `measure` call.
struct ParleyMeasurer {
    contexts: Mutex<Contexts>,
}

#[derive(Default)]
struct Contexts {
    font: FontContext,
    layout: LayoutContext<[u8; 4]>,
}

impl core::fmt::Debug for ParleyMeasurer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ParleyMeasurer")
    }
}

impl TextMeasurer for ParleyMeasurer {
    #[expect(
        clippy::significant_drop_tightening,
        reason = "the guard is already as tight as it can be: `font` and `layout` are borrowed out of it and stay borrowed for as long as the builder exists, so dropping it earlier does not compile"
    )]
    fn measure(&self, text: &str, style: &TextStyle) -> TextMetrics {
        if text.is_empty() {
            return TextMetrics {
                width: 0.0,
                height: 0.0,
                line_count: 0,
            };
        }

        // The guard is scoped to shaping alone. `Layout` is owned once it is
        // built, so line breaking and measurement need no access to the shared
        // contexts and the lock is released before them.
        let mut built = {
            let mut contexts = self
                .contexts
                .lock()
                .expect("mermaid text measurement mutex was poisoned by a panic while measuring");
            let Contexts { font, layout } = &mut *contexts;

            let mut builder = layout.ranged_builder(font, text, 1.0, true);
            builder.push_default(StyleProperty::FontSize(f32::from_f64_lossless(
                style.font_size,
            )));
            if let Some(family) = style.font_family.as_deref() {
                // Mermaid spells its font family as a CSS stack. `parley`
                // resolves one family at a time, so the first entry that names
                // a family is what we ask for and the system source falls back
                // from there.
                if let Some(first) = css_font_stack_head(family) {
                    builder.push_default(StyleProperty::FontFamily(FontFamily::named(first)));
                }
            }
            if let Some(weight) = style.font_weight.as_deref() {
                builder.push_default(StyleProperty::FontWeight(parse_weight(weight)));
            }
            if let Some(font_style) = style.font_style.as_deref() {
                builder.push_default(StyleProperty::FontStyle(parse_style(font_style)));
            }

            builder.build(text)
        };
        built.break_all_lines(None);
        built.align(Alignment::Start, AlignmentOptions::default());

        TextMetrics {
            width: f64::from(built.full_width()),
            height: f64::from(built.height()),
            line_count: built.len().max(1),
        }
    }
}

/// The first named family in a CSS font stack.
///
/// Generic families (`sans-serif` and friends) are not names `parley` can look
/// up, so they are skipped in favour of whatever concrete family follows — and
/// if the stack is nothing but generics, the system default is the right answer
/// anyway.
fn css_font_stack_head(stack: &str) -> Option<&str> {
    stack
        .split(',')
        .map(|family| family.trim().trim_matches(['"', '\'']))
        .find(|family| {
            !family.is_empty()
                && !matches!(
                    *family,
                    "serif"
                        | "sans-serif"
                        | "monospace"
                        | "cursive"
                        | "fantasy"
                        | "system-ui"
                        | "ui-serif"
                        | "ui-sans-serif"
                        | "ui-monospace"
                        | "ui-rounded"
                )
        })
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
trait FromF64Lossless {
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

/// The measurement policy a diagram is laid out under.
///
/// Installed on the render environment for every family, so a flowchart's node
/// boxes and a sequence diagram's participant boxes are sized by the same
/// engine that will draw their labels.
pub fn policy() -> TextMeasurementPolicy {
    let identity = TextMeasurementProfileIdentity::new(
        MeasurementProfileId::new(PROFILE).expect("the profile id is a valid identifier"),
        env!("CARGO_PKG_VERSION"),
    )
    .expect("the profile identity is well formed");

    TextMeasurementPolicy::uniform(TextMeasurementProfile::new(
        identity,
        Arc::new(ParleyMeasurer {
            contexts: Mutex::new(Contexts::default()),
        }),
    ))
}
