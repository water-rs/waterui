//! Access to a face's OpenType `MATH` table.
//!
//! Everything layout needs from the font is reached through here: the layout
//! constants, glyph outlines and advances, italic correction, and the
//! construction of glyphs that grow to fit their content.
//!
//! Two things about this module are load-bearing.
//!
//! First, the paired constants are resolved by [`MathStyle`] at the point the
//! constants are built, so layout code cannot reach for the non-display gap
//! while setting a display formula. Several `MATH` constants come in exactly
//! that pair and picking the wrong one is invisible until someone compares a
//! display fraction against a reference.
//!
//! Second, growing a glyph is a *general* facility. Radicals, parentheses,
//! braces, brackets, arrows and bars all stretch through the same
//! `MathVariants` mechanism — discrete variants first, then a multi-part
//! assembly when no variant is large enough. Special-casing the radical is how
//! an implementation ends up measuring outlines to find where its bar sits.

use alloc::vec::Vec;

use kurbo::{Affine, BezPath, Point};
use ttf_parser::math::{GlyphAssembly, Variants};
use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::ast::MathStyle;

/// Why a face could not be used to set mathematics.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MathFontError {
    /// The bytes are not a font this parser understands.
    #[error("could not parse the math font face (index {index})")]
    Unparsable {
        /// The face index that was requested.
        index: u32,
    },
    /// The face parsed, but carries no `MATH` table.
    ///
    /// This is the common failure, and it is not recoverable by drawing
    /// something approximate: without the table there are no layout constants,
    /// no glyph variants and no assemblies, so there is nothing to lay out
    /// against.
    #[error(
        "font `{family}` has no OpenType MATH table, so it cannot set mathematics; \
         choose a math font such as STIX Two Math"
    )]
    NotAMathFont {
        /// The family name reported by the face, for the error message.
        family: alloc::string::String,
    },
    /// A glyph the formula needs is not in the face.
    #[error("math font has no glyph for {character:?}")]
    MissingGlyph {
        /// The character that could not be mapped.
        character: char,
    },
    /// A glyph exists but carries no outline to draw.
    #[error("math font glyph {glyph:?} has no outline")]
    MissingOutline {
        /// The glyph that could not be outlined.
        glyph: u16,
    },
    /// A stretchy glyph's assembly is malformed.
    #[error("math font cannot assemble a stretched glyph: {reason}")]
    Assembly {
        /// What about the assembly could not be honoured.
        reason: &'static str,
    },
}

/// The `MATH` constants a formula needs, in pixels, at one size and style.
///
/// Built once per style change rather than read per use, so the display and
/// non-display members of each pair are chosen exactly once.
#[derive(Debug, Clone, Copy)]
pub struct MathConstants {
    /// Height of the imaginary line operators and fraction bars centre on.
    pub axis_height: f32,
    /// Thickness of a fraction rule, and the reference thickness for bars.
    pub fraction_rule_thickness: f32,
    /// How far a numerator's baseline sits above the axis.
    pub fraction_numerator_shift_up: f32,
    /// How far a denominator's baseline sits below the axis.
    pub fraction_denominator_shift_down: f32,
    /// Smallest gap between the numerator and the rule.
    pub fraction_numerator_gap_min: f32,
    /// Smallest gap between the rule and the denominator.
    pub fraction_denominator_gap_min: f32,
    /// Smallest gap between a radical's bar and what it covers.
    pub radical_vertical_gap: f32,
    /// Thickness of a radical's bar.
    pub radical_rule_thickness: f32,
    /// How far the radical sign rises above its bar.
    pub radical_extra_ascender: f32,
    /// Space before a radical's degree.
    pub radical_kern_before_degree: f32,
    /// Space after a radical's degree.
    pub radical_kern_after_degree: f32,
    /// Where the degree sits, as a fraction of the radical's height.
    pub radical_degree_bottom_raise_percent: f32,
    /// Default upward shift of a superscript's baseline.
    pub superscript_shift_up: f32,
    /// Default downward shift of a subscript's baseline.
    pub subscript_shift_down: f32,
    /// A superscript's baseline may not sit lower than this.
    pub superscript_bottom_min: f32,
    /// A subscript's top may not rise above this.
    pub subscript_top_max: f32,
    /// Smallest gap between a superscript and a subscript on the same base.
    pub sub_superscript_gap_min: f32,
    /// Space added after a script, so the next atom does not touch it.
    pub space_after_script: f32,
}

impl MathConstants {
    fn read(constants: &ttf_parser::math::Constants<'_>, scale: f32, style: MathStyle) -> Self {
        let px = |value: ttf_parser::math::MathValue<'_>| f32::from(value.value) * scale;
        let display = style.is_display();

        Self {
            axis_height: px(constants.axis_height()),
            fraction_rule_thickness: px(constants.fraction_rule_thickness()),
            fraction_numerator_shift_up: if display {
                px(constants.fraction_numerator_display_style_shift_up())
            } else {
                px(constants.fraction_numerator_shift_up())
            },
            fraction_denominator_shift_down: if display {
                px(constants.fraction_denominator_display_style_shift_down())
            } else {
                px(constants.fraction_denominator_shift_down())
            },
            fraction_numerator_gap_min: if display {
                px(constants.fraction_num_display_style_gap_min())
            } else {
                px(constants.fraction_numerator_gap_min())
            },
            fraction_denominator_gap_min: if display {
                px(constants.fraction_denom_display_style_gap_min())
            } else {
                px(constants.fraction_denominator_gap_min())
            },
            radical_vertical_gap: if display {
                px(constants.radical_display_style_vertical_gap())
            } else {
                px(constants.radical_vertical_gap())
            },
            radical_rule_thickness: px(constants.radical_rule_thickness()),
            radical_extra_ascender: px(constants.radical_extra_ascender()),
            radical_kern_before_degree: px(constants.radical_kern_before_degree()),
            radical_kern_after_degree: px(constants.radical_kern_after_degree()),
            radical_degree_bottom_raise_percent: f32::from(
                constants.radical_degree_bottom_raise_percent(),
            ) / 100.0,
            superscript_shift_up: px(constants.superscript_shift_up()),
            subscript_shift_down: px(constants.subscript_shift_down()),
            superscript_bottom_min: px(constants.superscript_bottom_min()),
            subscript_top_max: px(constants.subscript_top_max()),
            sub_superscript_gap_min: px(constants.sub_superscript_gap_min()),
            space_after_script: px(constants.space_after_script()),
        }
    }
}

/// A glyph placed by layout: which glyph, from which face, at what size.
///
/// Positions are filled in by layout; this is the identity half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    /// Index into the face.
    pub id: GlyphId,
}

/// A glyph grown to a requested size, as an outline in pixels.
#[derive(Debug, Clone)]
pub struct StretchedGlyph {
    /// The outline, already scaled and in the y-down space layout works in,
    /// with its top-left at the origin.
    pub outline: BezPath,
    /// Total height of the outline.
    pub height: f32,
    /// Total width of the outline.
    pub width: f32,
}

/// A face that can set mathematics.
pub struct MathFont<'a> {
    face: Face<'a>,
    units_per_em: f32,
}

impl core::fmt::Debug for MathFont<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MathFont")
            .field("units_per_em", &self.units_per_em)
            .finish_non_exhaustive()
    }
}

impl<'a> MathFont<'a> {
    /// Reads a face, rejecting it if it cannot set mathematics.
    ///
    /// The `MATH` table is checked here rather than at first use, so a face
    /// without one is refused while there is still a clear place to say so.
    ///
    /// # Errors
    ///
    /// [`MathFontError::Unparsable`] if the bytes are not a font, or
    /// [`MathFontError::NotAMathFont`] if they are a font that carries no
    /// `MATH` table and so cannot set mathematics.
    pub fn new(data: &'a [u8], index: u32) -> Result<Self, MathFontError> {
        let face = Face::parse(data, index).map_err(|_| MathFontError::Unparsable { index })?;

        if face.tables().math.and_then(|math| math.constants).is_none() {
            let family = face
                .names()
                .into_iter()
                .find(|name| name.name_id == ttf_parser::name_id::FAMILY)
                .and_then(|name| name.to_string())
                .unwrap_or_else(|| alloc::string::String::from("<unnamed>"));
            return Err(MathFontError::NotAMathFont { family });
        }

        let units_per_em = f32::from(face.units_per_em());
        Ok(Self { face, units_per_em })
    }

    /// Font units to pixels at `font_size`.
    fn scale(&self, font_size: f32) -> f32 {
        font_size / self.units_per_em
    }

    fn math(&self) -> ttf_parser::math::Table<'a> {
        self.face
            .tables()
            .math
            .expect("MathFont::new refuses a face without a MATH table")
    }

    fn variants(&self) -> Option<Variants<'a>> {
        self.math().variants
    }

    /// The layout constants at this size and style.
    ///
    /// # Panics
    ///
    /// Never in practice: [`MathFont::new`] refuses a face whose `MATH` table
    /// has no constants, so a `MathFont` that exists has them.
    #[must_use]
    pub fn constants(&self, font_size: f32, style: MathStyle) -> MathConstants {
        let constants = self
            .math()
            .constants
            .expect("MathFont::new refuses a face without MATH constants");
        MathConstants::read(&constants, self.scale(font_size), style)
    }

    /// How far scripts shrink at `style`, as a multiple of the base size.
    ///
    /// # Panics
    ///
    /// Never in practice, for the same reason as [`MathFont::constants`].
    #[must_use]
    pub fn script_scale(&self, style: MathStyle) -> f32 {
        let constants = self
            .math()
            .constants
            .expect("MathFont::new refuses a face without MATH constants");
        match style {
            MathStyle::Display | MathStyle::Text => 1.0,
            MathStyle::Script => f32::from(constants.script_percent_scale_down()) / 100.0,
            MathStyle::ScriptScript => {
                f32::from(constants.script_script_percent_scale_down()) / 100.0
            }
        }
    }

    /// The glyph `character` maps to.
    ///
    /// # Errors
    ///
    /// [`MathFontError::MissingGlyph`] if the face has no glyph for it.
    pub fn glyph(&self, character: char) -> Result<Glyph, MathFontError> {
        self.face
            .glyph_index(character)
            .map(|id| Glyph { id })
            .ok_or(MathFontError::MissingGlyph { character })
    }

    /// How far the pen moves after drawing `glyph`.
    #[must_use]
    pub fn advance(&self, glyph: Glyph, font_size: f32) -> f32 {
        self.face
            .glyph_hor_advance(glyph.id)
            .map_or(0.0, |advance| f32::from(advance) * self.scale(font_size))
    }

    /// The correction that keeps a script clear of a slanted glyph's overhang.
    ///
    /// Without it a superscript on an italic letter sits visibly too far left,
    /// because the letter's advance stops before its ink does.
    #[must_use]
    pub fn italic_correction(&self, glyph: Glyph, font_size: f32) -> f32 {
        self.math()
            .glyph_info
            .and_then(|info| info.italic_corrections)
            .and_then(|corrections| corrections.get(glyph.id))
            .map_or(0.0, |value| f32::from(value.value) * self.scale(font_size))
    }

    /// How far above and below the baseline `glyph`'s ink reaches, in the
    /// y-down space layout works in.
    ///
    /// Returns `(ascent, descent)`, both non-negative.
    #[must_use]
    pub fn vertical_extents(&self, glyph: Glyph, font_size: f32) -> (f32, f32) {
        let scale = self.scale(font_size);
        self.face
            .glyph_bounding_box(glyph.id)
            .map_or((0.0, 0.0), |bbox| {
                (
                    (f32::from(bbox.y_max) * scale).max(0.0),
                    (-f32::from(bbox.y_min) * scale).max(0.0),
                )
            })
    }

    /// `glyph`'s outline in pixels, in the y-down space, relative to its
    /// baseline origin.
    ///
    /// # Errors
    ///
    /// [`MathFontError::MissingOutline`] if the glyph carries no outline, as a
    /// bitmap-only or blank glyph does.
    pub fn outline(&self, glyph: Glyph, font_size: f32) -> Result<BezPath, MathFontError> {
        let mut builder = OutlineToBezPath::default();
        self.face
            .outline_glyph(glyph.id, &mut builder)
            .ok_or(MathFontError::MissingOutline { glyph: glyph.id.0 })?;
        let mut path = builder.path;
        path.apply_affine(Affine::scale(f64::from(self.scale(font_size))));
        Ok(path)
    }

    /// Grows `character` vertically to at least `target` pixels.
    ///
    /// Discrete variants are tried in order first, since a designed variant is
    /// always better than an assembled one; only when the largest is still too
    /// small is the multi-part assembly built. This is the mechanism behind
    /// every growing glyph — radicals, fences, braces and bars alike.
    ///
    /// # Errors
    ///
    /// [`MathFontError::MissingGlyph`] or [`MathFontError::MissingOutline`] if
    /// the face cannot supply a piece, or [`MathFontError::Assembly`] if the
    /// glyph's assembly is malformed or cannot reach the requested size.
    pub fn stretch_vertical(
        &self,
        character: char,
        target: f32,
        font_size: f32,
    ) -> Result<StretchedGlyph, MathFontError> {
        let glyph = self.glyph(character)?;
        let scale = self.scale(font_size);
        let target_units = target / scale;

        let construction = self
            .variants()
            .and_then(|variants| variants.vertical_constructions.get(glyph.id));

        let Some(construction) = construction else {
            return self.as_stretched(glyph, font_size);
        };

        let mut best = None;
        for variant in construction.variants {
            best = Some(variant);
            if f32::from(variant.advance_measurement) >= target_units {
                break;
            }
        }

        let large_enough =
            best.is_some_and(|variant| f32::from(variant.advance_measurement) >= target_units);

        if !large_enough && let Some(assembly) = construction.assembly {
            let min_overlap = self
                .variants()
                .map_or(0, |variants| variants.min_connector_overlap);
            return self.assemble_vertical(assembly, min_overlap, target_units, font_size);
        }

        let variant = best.ok_or(MathFontError::Assembly {
            reason: "glyph construction has no variants",
        })?;
        self.as_stretched(
            Glyph {
                id: variant.variant_glyph,
            },
            font_size,
        )
    }

    /// One glyph, normalised so its top-left sits at the origin.
    fn as_stretched(&self, glyph: Glyph, font_size: f32) -> Result<StretchedGlyph, MathFontError> {
        let scale = self.scale(font_size);
        let bbox = self
            .face
            .glyph_bounding_box(glyph.id)
            .ok_or(MathFontError::MissingOutline { glyph: glyph.id.0 })?;
        let mut outline = self.outline(glyph, font_size)?;

        // Font space is y-up and the outline builder already flipped it, so the
        // top edge is at `-y_max`.
        let left = f32::from(bbox.x_min) * scale;
        let top = -f32::from(bbox.y_max) * scale;
        outline.apply_affine(Affine::translate((f64::from(-left), f64::from(-top))));

        Ok(StretchedGlyph {
            outline,
            height: (f32::from(bbox.y_max) - f32::from(bbox.y_min)) * scale,
            width: (f32::from(bbox.x_max) - f32::from(bbox.x_min)) * scale,
        })
    }

    /// Builds a glyph from its parts, repeating the extenders until it is long
    /// enough.
    fn assemble_vertical(
        &self,
        assembly: GlyphAssembly<'a>,
        min_connector_overlap: u16,
        target_units: f32,
        font_size: f32,
    ) -> Result<StretchedGlyph, MathFontError> {
        let parts: Vec<_> = assembly.parts.into_iter().collect();
        if parts.is_empty() {
            return Err(MathFontError::Assembly {
                reason: "assembly has no parts",
            });
        }
        if !parts.iter().any(|part| part.part_flags.extender()) {
            return Err(MathFontError::Assembly {
                reason: "assembly has no extender part",
            });
        }

        // Repeat every extender equally, growing the run until it spans the
        // target. Each pass adds one copy of each extender, which is what keeps
        // a brace's two halves symmetric.
        //
        // The bound is on the repeat count rather than on the measured length,
        // so a font whose connectors overlap by their whole advance — which
        // would never grow the run — terminates with an error instead of
        // looping.
        let mut sequence = None;
        for repeats in 1..=MAX_REPEATS {
            let candidate = expand(&parts, repeats);
            if advance_of(&candidate, min_connector_overlap) >= target_units {
                sequence = Some(candidate);
                break;
            }
            if repeats == MAX_REPEATS {
                return Err(MathFontError::Assembly {
                    reason: "assembly did not reach the requested size",
                });
            }
        }
        let Some(sequence) = sequence else {
            return Err(MathFontError::Assembly {
                reason: "assembly did not reach the requested size",
            });
        };

        let scale = self.scale(font_size);
        let mut outline = BezPath::new();
        let mut offset_units = 0.0_f32;
        let mut width = 0.0_f32;

        for (index, part) in sequence.iter().enumerate() {
            let glyph = Glyph { id: part.glyph_id };
            let mut piece = self.outline(glyph, font_size)?;
            // Parts stack downward in the y-down space.
            piece.apply_affine(Affine::translate((0.0, f64::from(offset_units * scale))));
            outline.extend(piece.iter());

            if let Some(bbox) = self.face.glyph_bounding_box(glyph.id) {
                width = width.max((f32::from(bbox.x_max) - f32::from(bbox.x_min)) * scale);
            }

            if let Some(next) = sequence.get(index + 1) {
                let overlap = overlap_between(part, next, min_connector_overlap);
                offset_units += f32::from(part.full_advance) - overlap;
            } else {
                offset_units += f32::from(part.full_advance);
            }
        }

        let height = offset_units * scale;
        Ok(StretchedGlyph {
            outline,
            height,
            width,
        })
    }
}

/// How many times an extender may repeat before the assembly is declared
/// unable to reach the requested size.
const MAX_REPEATS: usize = 128;

/// How far two adjacent parts may overlap without breaking either connector.
fn overlap_between(
    previous: &ttf_parser::math::GlyphPart,
    next: &ttf_parser::math::GlyphPart,
    min_connector_overlap: u16,
) -> f32 {
    f32::from(min_connector_overlap)
        .min(f32::from(previous.end_connector_length))
        .min(f32::from(next.start_connector_length))
}

/// The part run with every extender repeated `repeats` times.
fn expand(
    parts: &[ttf_parser::math::GlyphPart],
    repeats: usize,
) -> Vec<ttf_parser::math::GlyphPart> {
    let mut sequence = Vec::new();
    for part in parts {
        let copies = if part.part_flags.extender() {
            repeats
        } else {
            1
        };
        for _ in 0..copies {
            sequence.push(*part);
        }
    }
    sequence
}

/// Total length of a part run once connectors overlap.
fn advance_of(parts: &[ttf_parser::math::GlyphPart], min_connector_overlap: u16) -> f32 {
    let Some(first) = parts.first() else {
        return 0.0;
    };
    let mut total = f32::from(first.full_advance);
    for pair in parts.windows(2) {
        let overlap = overlap_between(&pair[0], &pair[1], min_connector_overlap);
        total += f32::from(pair[1].full_advance) - overlap;
    }
    total
}

/// Collects a glyph outline into a `kurbo` path, flipping to the y-down space
/// the rest of the framework works in.
#[derive(Default)]
struct OutlineToBezPath {
    path: BezPath,
}

impl OutlineBuilder for OutlineToBezPath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(flip(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(flip(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.path.quad_to(flip(x1, y1), flip(x, y));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path.curve_to(flip(x1, y1), flip(x2, y2), flip(x, y));
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

fn flip(x: f32, y: f32) -> Point {
    Point::new(f64::from(x), f64::from(-y))
}
