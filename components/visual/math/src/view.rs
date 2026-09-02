//! The `Math` view.

use alloc::string::String;

use nami::signal::IntoComputed;
use parley::FontContext;
use peniko::{Brush, Color as PenikoColor, FontData};
use waterui_core::{Computed, Environment, Signal, View};
use waterui_graphics::color::{Color, ForegroundColor, ResolvedColor};
use waterui_graphics::{Scene2D, SceneContent, SceneInvalidator, SceneView};
use waterui_layout::frame::Frame;
use waterui_str::Str;

use crate::ast::{MathItem, MathStyle};
use crate::font::MathFont;
use crate::layout::Layouter;
use crate::{latex, mathml, scene};

/// The math family used when the caller names none.
///
/// Only a face with an OpenType `MATH` table can set mathematics, so this is
/// not a stylistic default that something else could stand in for.
pub const DEFAULT_MATH_FAMILY: &str = "STIX Two Math";

/// Why a formula could not be prepared for drawing.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MathError {
    /// The source did not parse.
    #[error(transparent)]
    Latex(#[from] crate::latex::LatexError),
    /// No installed family by that name carries a `MATH` table.
    #[error(
        "no math font available: `{family}` is not installed, or carries no OpenType MATH table"
    )]
    NoFont {
        /// The family that was asked for.
        family: String,
    },
    /// The formula parsed but could not be laid out.
    #[error(transparent)]
    Layout(#[from] crate::layout::LayoutError),
}

/// A rendered mathematical formula.
///
/// The source is LaTeX. It is parsed into a semantic tree, laid out against the
/// OpenType `MATH` table of the chosen family, and drawn through `Scene2D`, so
/// it renders on every engine the backend may supply.
#[derive(Debug, Clone)]
pub struct Math {
    source: Computed<Str>,
    style: MathStyle,
    font_size: f32,
    family: Str,
    color: Option<Color>,
}

impl Math {
    /// A formula from LaTeX source.
    ///
    /// The source takes a signal, so a formula bound to state updates without
    /// its subtree being rebuilt.
    #[must_use]
    pub fn new(source: impl IntoComputed<Str>) -> Self {
        Self {
            source: source.into_computed(),
            style: MathStyle::Text,
            font_size: 18.0,
            family: Str::from_static(DEFAULT_MATH_FAMILY),
            color: None,
        }
    }

    /// Sets the formula on its own line, in display style.
    #[must_use]
    pub const fn display(mut self) -> Self {
        self.style = MathStyle::Display;
        self
    }

    /// Sets the formula inline, in text style.
    #[must_use]
    pub const fn inline(mut self) -> Self {
        self.style = MathStyle::Text;
        self
    }

    /// Sets the em size the formula is set at.
    ///
    /// # Panics
    ///
    /// Panics if `size` is not finite and positive, which is a caller bug
    /// rather than bad input.
    #[must_use]
    pub fn font_size(mut self, size: f32) -> Self {
        assert!(
            size.is_finite() && size > 0.0,
            "math font size must be finite and positive, got {size}"
        );
        self.font_size = size;
        self
    }

    /// Sets the math family. It must carry an OpenType `MATH` table.
    #[must_use]
    pub fn font_family(mut self, family: impl Into<Str>) -> Self {
        self.family = family.into();
        self
    }

    /// Overrides the colour the formula is drawn in.
    #[must_use]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = Some(color.into());
        self
    }
}

/// A formula prepared for drawing: the tree, its `MathML`, and its metrics.
#[derive(Debug, Clone)]
pub struct PreparedMath {
    /// The semantic tree, kept because it is the accessibility representation.
    pub item: MathItem,
    /// The tree as `MathML`, for the accessibility payload.
    pub mathml: String,
    /// Total advance width in pixels.
    pub width: f32,
    /// Height above the baseline.
    pub ascent: f32,
    /// Depth below the baseline.
    pub descent: f32,
}

impl PreparedMath {
    /// Total height.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// Parses and measures `source` against `font`.
///
/// # Errors
///
/// Returns [`MathError`] when the source does not parse or cannot be laid out.
pub fn prepare(
    source: &str,
    font: &FontData,
    font_size: f32,
    style: MathStyle,
) -> Result<PreparedMath, MathError> {
    let item = latex::parse(source)?;
    let math_font = MathFont::new(font.data.data(), font.index)
        .map_err(|error| MathError::Layout(crate::layout::LayoutError::Font(error)))?;
    let layouter = Layouter::new(&math_font)?;
    let layout = layouter.layout(&item, font_size, style)?;
    let mathml = mathml::to_mathml(&item, style).unwrap_or_else(|_| String::new());

    Ok(PreparedMath {
        item,
        mathml,
        width: layout.width,
        ascent: layout.ascent,
        descent: layout.descent,
    })
}

/// Finds an installed family that can set mathematics.
///
/// # Errors
///
/// Returns [`MathError::NoFont`] when no installed face by that name carries a
/// `MATH` table. There is deliberately no substitute: a face without the table
/// has no layout constants, so drawing with it would not be the same formula
/// set differently, it would be a formula with no geometry.
pub fn resolve_font(fonts: &mut FontContext, family: &str) -> Result<FontData, MathError> {
    let no_font = || MathError::NoFont {
        family: String::from(family),
    };

    let info = fonts
        .collection
        .family_by_name(family)
        .ok_or_else(no_font)?;
    for font in info.fonts() {
        let Some(blob) = fonts.source_cache.get(font.source()) else {
            continue;
        };
        let data = FontData::new(blob, font.index());
        if MathFont::new(data.data.data(), data.index).is_ok() {
            return Ok(data);
        }
    }
    Err(no_font())
}

/// Scene content that draws one formula.
///
/// [`Math`] wraps this, and a backend that wants to draw a formula into a scene
/// it already owns can use it directly rather than going through a view.
pub struct MathContent {
    source: Computed<Str>,
    style: MathStyle,
    font_size: f32,
    family: Str,
    brush: Brush,
    fonts: Option<FontContext>,
    font: Option<FontData>,
    invalidator: Option<SceneInvalidator>,
}

impl MathContent {
    /// Scene content drawing `source` at `font_size` in `style`.
    ///
    /// The family must carry an OpenType `MATH` table;
    /// [`DEFAULT_MATH_FAMILY`] is the one this crate asks for by default.
    #[must_use]
    pub fn new(
        source: impl IntoComputed<Str>,
        font_size: f32,
        style: MathStyle,
        family: impl Into<Str>,
        brush: Brush,
    ) -> Self {
        Self {
            source: source.into_computed(),
            style,
            font_size,
            family: family.into(),
            brush,
            fonts: None,
            font: None,
            invalidator: None,
        }
    }
}

impl core::fmt::Debug for MathContent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MathContent")
            .field("family", &self.family)
            .field("font_size", &self.font_size)
            .field("style", &self.style)
            .finish_non_exhaustive()
    }
}

impl SceneContent for MathContent {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool {
        if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
            return false;
        }

        // The font collection is discovered once and kept for the life of this
        // surface, which is where per-surface resources belong.
        if self.font.is_none() {
            let fonts = self.fonts.get_or_insert_with(FontContext::new);
            match resolve_font(fonts, self.family.as_str()) {
                Ok(font) => self.font = Some(font),
                Err(error) => {
                    tracing::error!(%error, "math formula has no usable font");
                    return false;
                }
            }
        }
        let Some(font) = self.font.clone() else {
            return false;
        };

        let source = self.source.get();
        let prepared = match prepare(source.as_str(), &font, self.font_size, self.style) {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::error!(%error, formula = %source, "could not render math formula");
                return false;
            }
        };

        let math_font = match MathFont::new(font.data.data(), font.index) {
            Ok(math_font) => math_font,
            Err(error) => {
                tracing::error!(%error, "math font became unusable");
                return false;
            }
        };
        let Ok(layouter) = Layouter::new(&math_font) else {
            return false;
        };
        let Ok(layout) = layouter.layout(&prepared.item, self.font_size, self.style) else {
            return false;
        };

        scene::draw(&layout, scene, &font, &self.brush, 0.0, prepared.ascent);
        false
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.invalidator = invalidator;
    }
}

impl View for Math {
    fn body(self, env: &Environment) -> impl View {
        let color = self
            .color
            .clone()
            .map(|color| color.resolve(env))
            .or_else(|| {
                env.query::<ForegroundColor, Computed<ResolvedColor>>()
                    .cloned()
            });

        let brush = color.map_or_else(
            || Brush::Solid(PenikoColor::BLACK),
            |signal| Brush::Solid(to_peniko(&signal.get())),
        );

        let content = MathContent::new(self.source, self.font_size, self.style, self.family, brush);

        Frame::new(SceneView::new(content))
    }
}

fn to_peniko(color: &ResolvedColor) -> PenikoColor {
    let srgb = color.to_srgb();
    let channel = |value: f32| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=255 before the cast"
        )]
        let byte = (value * 255.0).clamp(0.0, 255.0).round() as u8;
        byte
    };
    PenikoColor::from_rgba8(
        channel(srgb.red),
        channel(srgb.green),
        channel(srgb.blue),
        channel(color.opacity),
    )
}
