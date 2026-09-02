//! Turning a semantic tree into positioned glyphs and rules.
//!
//! Every measurement here comes from the face's OpenType `MATH` table. Nothing
//! is derived by inspecting glyph outlines, and nothing is a tuned constant:
//! where a position looks arbitrary it is the value the font supplies, and
//! where a minimum is enforced it is a `*GapMin` the table names.
//!
//! # Coordinates
//!
//! A [`MathLayout`] is expressed about its own baseline: `y` grows downward,
//! the baseline is `y = 0`, [`MathLayout::ascent`] is how far the ink reaches
//! above it and [`MathLayout::descent`] how far below, both non-negative. `x`
//! grows rightward from the fragment's left edge. Fragments compose by
//! translation, which is why every layout function can work in its own frame
//! and be placed by its caller.

use alloc::vec::Vec;

use kurbo::{Affine, BezPath};

use crate::ast::{MathClass, MathItem, MathStyle, Operator};
use crate::font::{Glyph, MathConstants, MathFont, MathFontError};
use crate::spacing::{SpacingError, SpacingTable};

/// Why a formula could not be laid out.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LayoutError {
    /// The font could not supply something the formula needs.
    #[error(transparent)]
    Font(#[from] MathFontError),
    /// The spacing table could not be read.
    #[error(transparent)]
    Spacing(#[from] SpacingError),
    /// The requested size is not a size.
    #[error("font size must be finite and positive, got {size}")]
    Size {
        /// The rejected size.
        size: f32,
    },
}

/// One thing to draw, positioned in its fragment's frame.
#[derive(Debug, Clone)]
pub enum Placed {
    /// A glyph from the math face, drawn at its baseline origin.
    Glyph {
        /// Which glyph.
        glyph: Glyph,
        /// Left edge of the glyph's advance.
        x: f32,
        /// The baseline it sits on.
        baseline: f32,
        /// The size it is drawn at, which differs from the formula's size
        /// inside scripts.
        size: f32,
    },
    /// A prebuilt outline, already scaled and already positioned.
    ///
    /// This is how a stretched glyph arrives: assembling one produces an
    /// outline rather than a glyph index, because it is several glyphs joined
    /// at their connectors. The path carries its own position, so there is no
    /// separate origin to keep in step with it.
    Outline(BezPath),
    /// A filled rectangle: a fraction bar or a radical's overbar.
    Rule {
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Horizontal extent.
        width: f32,
        /// Vertical extent.
        height: f32,
    },
}

impl Placed {
    /// The same item moved by `(dx, dy)`.
    #[must_use]
    fn translated(self, dx: f32, dy: f32) -> Self {
        match self {
            Self::Glyph {
                glyph,
                x,
                baseline,
                size,
            } => Self::Glyph {
                glyph,
                x: x + dx,
                baseline: baseline + dy,
                size,
            },
            Self::Outline(mut outline) => {
                outline.apply_affine(Affine::translate((f64::from(dx), f64::from(dy))));
                Self::Outline(outline)
            }
            Self::Rule {
                x,
                y,
                width,
                height,
            } => Self::Rule {
                x: x + dx,
                y: y + dy,
                width,
                height,
            },
        }
    }
}

/// A laid-out formula, or a piece of one.
#[derive(Debug, Clone, Default)]
pub struct MathLayout {
    /// What to draw.
    pub items: Vec<Placed>,
    /// Total advance width.
    pub width: f32,
    /// How far the ink reaches above the baseline.
    pub ascent: f32,
    /// How far the ink reaches below the baseline.
    pub descent: f32,
}

impl MathLayout {
    /// Total height, ascent plus descent.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.ascent + self.descent
    }

    /// Absorbs `other`, placing its origin at `(dx, dy)` in this frame.
    fn absorb(&mut self, other: Self, dx: f32, dy: f32) {
        self.items
            .extend(other.items.into_iter().map(|item| item.translated(dx, dy)));
        self.ascent = self.ascent.max(other.ascent - dy);
        self.descent = self.descent.max(other.descent + dy);
        self.width = self.width.max(dx + other.width);
    }
}

/// Lays formulas out against one face.
#[derive(Debug)]
pub struct Layouter<'a> {
    font: &'a MathFont<'a>,
    spacing: SpacingTable,
}

impl<'a> Layouter<'a> {
    /// Prepares a layouter over `font`.
    ///
    /// # Errors
    ///
    /// [`LayoutError::Spacing`] if the shipped spacing table is malformed,
    /// which is a defect in this crate rather than in the caller's input.
    pub fn new(font: &'a MathFont<'a>) -> Result<Self, LayoutError> {
        Ok(Self {
            font,
            spacing: SpacingTable::load()?,
        })
    }

    /// Lays `item` out at `size` pixels in `style`.
    ///
    /// # Errors
    ///
    /// [`LayoutError::Size`] if `size` is not finite and positive, or
    /// [`LayoutError::Font`] if the face cannot supply a glyph the formula
    /// needs or cannot grow one to the size the formula requires.
    pub fn layout(
        &self,
        item: &MathItem,
        size: f32,
        style: MathStyle,
    ) -> Result<MathLayout, LayoutError> {
        if !(size.is_finite() && size > 0.0) {
            return Err(LayoutError::Size { size });
        }
        self.item(item, size, style)
    }

    fn constants(&self, size: f32, style: MathStyle) -> MathConstants {
        self.font.constants(size, style)
    }

    /// The size a child in `child_style` takes, given a parent at `size` in
    /// `style`.
    ///
    /// Stepping relatively rather than absolutely is what keeps a script inside
    /// a script from being scaled twice from the top.
    fn child_size(&self, size: f32, style: MathStyle, child_style: MathStyle) -> f32 {
        let from = self.font.script_scale(style);
        let to = self.font.script_scale(child_style);
        if from > 0.0 { size * (to / from) } else { size }
    }

    fn item(
        &self,
        item: &MathItem,
        size: f32,
        style: MathStyle,
    ) -> Result<MathLayout, LayoutError> {
        match item {
            MathItem::Ident(text) => self.glyph_run(text, size, IdentStyle::Italic),
            MathItem::Number(text) | MathItem::Text(text) => {
                self.glyph_run(text, size, IdentStyle::Upright)
            }
            MathItem::Operator(operator) => {
                self.glyph_run(&operator.glyph, size, IdentStyle::Upright)
            }
            MathItem::Space(em) => Ok(MathLayout {
                width: em * size,
                ..MathLayout::default()
            }),
            MathItem::Row(items) => self.row(items, size, style),
            MathItem::Fraction {
                numerator,
                denominator,
            } => self.fraction(numerator, denominator, size, style),
            MathItem::Radical { radicand, degree } => {
                self.radical(radicand, degree.as_deref(), size, style)
            }
            MathItem::Scripts { base, sub, sup } => {
                self.scripts(base, sub.as_deref(), sup.as_deref(), size, style)
            }
            MathItem::Fenced { open, body, close } => {
                self.fenced(open.as_ref(), body, close.as_ref(), size, style)
            }
        }
    }

    /// A run of characters set as glyphs on one baseline.
    fn glyph_run(
        &self,
        text: &str,
        size: f32,
        ident_style: IdentStyle,
    ) -> Result<MathLayout, LayoutError> {
        let mut layout = MathLayout::default();
        let letters = text.chars().count();

        for character in text.chars() {
            // A single-letter identifier is italic, which is the convention
            // for a variable; a multi-letter one (`sin`, `max`) stays upright,
            // because it is a name rather than a product of variables.
            let drawn = match ident_style {
                IdentStyle::Italic if letters == 1 => math_italic(character),
                _ => character,
            };
            let glyph = self.font.glyph(drawn).or_else(|error| {
                // A face may lack the mathematical-alphanumeric codepoint while
                // having the plain letter. Falling back to the plain letter is
                // a different glyph, not a different rendering strategy, so it
                // stays inside the font layer's contract.
                if drawn == character {
                    Err(error)
                } else {
                    self.font.glyph(character)
                }
            })?;

            let (ascent, descent) = self.font.vertical_extents(glyph, size);
            layout.items.push(Placed::Glyph {
                glyph,
                x: layout.width,
                baseline: 0.0,
                size,
            });
            layout.width += self.font.advance(glyph, size);
            layout.ascent = layout.ascent.max(ascent);
            layout.descent = layout.descent.max(descent);
        }

        Ok(layout)
    }

    /// A horizontal sequence, with the gaps the spacing table calls for.
    fn row(
        &self,
        items: &[MathItem],
        size: f32,
        style: MathStyle,
    ) -> Result<MathLayout, LayoutError> {
        let mut layout = MathLayout::default();
        let mut previous: Option<MathClass> = None;

        for item in items {
            if let Some(left) = previous {
                layout.width = self
                    .spacing
                    .between(left, item.class(), style)
                    .mul_add(size, layout.width);
            }
            let child = self.item(item, size, style)?;
            let x = layout.width;
            let advance = child.width;
            layout.absorb(child, x, 0.0);
            layout.width = x + advance;
            previous = Some(item.class());
        }

        Ok(layout)
    }

    /// A fraction: numerator, rule on the axis, denominator.
    fn fraction(
        &self,
        numerator: &MathItem,
        denominator: &MathItem,
        size: f32,
        style: MathStyle,
    ) -> Result<MathLayout, LayoutError> {
        let constants = self.constants(size, style);
        let inner_style = style.fraction();
        let inner_size = self.child_size(size, style, inner_style);

        let num = self.item(numerator, inner_size, inner_style)?;
        let den = self.item(denominator, inner_size, inner_style)?;

        let thickness = constants.fraction_rule_thickness;
        let axis_y = -constants.axis_height;
        let rule_top = axis_y - thickness / 2.0;
        let rule_bottom = axis_y + thickness / 2.0;

        // Start from the font's preferred shifts, then open them up if the
        // minimum gaps are not met. Both are needed: the shifts alone let a
        // tall numerator collide with the rule.
        let numerator_baseline = (-constants.fraction_numerator_shift_up)
            .min(rule_top - constants.fraction_numerator_gap_min - num.descent);
        let denominator_baseline = constants
            .fraction_denominator_shift_down
            .max(rule_bottom + constants.fraction_denominator_gap_min + den.ascent);

        // The rule spans the wider of the two parts, and each part is centred
        // over it.
        let width = num.width.max(den.width);
        let num_x = (width - num.width) / 2.0;
        let den_x = (width - den.width) / 2.0;

        let mut layout = MathLayout::default();
        layout.absorb(num, num_x, numerator_baseline);
        layout.absorb(den, den_x, denominator_baseline);
        layout.items.push(Placed::Rule {
            x: 0.0,
            y: rule_top,
            width,
            height: thickness,
        });
        layout.width = width;
        layout.ascent = layout.ascent.max(-rule_top);
        layout.descent = layout.descent.max(rule_bottom);

        Ok(layout)
    }

    /// Sub- and superscripts attached to a base.
    fn scripts(
        &self,
        base: &MathItem,
        sub: Option<&MathItem>,
        sup: Option<&MathItem>,
        size: f32,
        style: MathStyle,
    ) -> Result<MathLayout, LayoutError> {
        let constants = self.constants(size, style);
        let script_style = style.script();
        let script_size = self.child_size(size, style, script_style);

        let base_layout = self.item(base, size, style)?;
        let base_width = base_layout.width;

        // The correction that keeps a superscript clear of a slanted base's
        // overhang. It applies to the superscript only: the subscript sits
        // under the overhang, which is the point of the shape.
        let italic = self.italic_correction(base, size);

        let mut layout = MathLayout::default();
        layout.absorb(base_layout, 0.0, 0.0);
        layout.width = base_width;

        let superscript = sup
            .map(|item| self.item(item, script_size, script_style))
            .transpose()?;
        let subscript = sub
            .map(|item| self.item(item, script_size, script_style))
            .transpose()?;

        let mut superscript_baseline = 0.0_f32;
        let mut subscript_baseline = 0.0_f32;

        if let Some(sup) = &superscript {
            // Raise until the script's own bottom clears the minimum.
            superscript_baseline = (-constants.superscript_shift_up)
                .min(-constants.superscript_bottom_min - sup.descent);
        }
        if let Some(sub) = &subscript {
            subscript_baseline = constants
                .subscript_shift_down
                .max(sub.ascent - constants.subscript_top_max);
        }
        if let (Some(sup), Some(sub)) = (&superscript, &subscript) {
            // Open the pair apart until the gap between the superscript's
            // bottom edge and the subscript's top edge meets the minimum.
            let gap = (subscript_baseline - sub.ascent) - (superscript_baseline + sup.descent);
            let shortfall = constants.sub_superscript_gap_min - gap;
            if shortfall > 0.0 {
                superscript_baseline -= shortfall / 2.0;
                subscript_baseline += shortfall / 2.0;
            }
        }

        let mut right = base_width;
        if let Some(sup) = superscript {
            let width = sup.width;
            layout.absorb(sup, base_width + italic, superscript_baseline);
            right = right.max(base_width + italic + width);
        }
        if let Some(sub) = subscript {
            let width = sub.width;
            layout.absorb(sub, base_width, subscript_baseline);
            right = right.max(base_width + width);
        }

        layout.width = right + constants.space_after_script;
        Ok(layout)
    }

    /// The italic correction of a base, when the base is a single glyph.
    ///
    /// A composite base has no single correction to read, and its rightmost
    /// ink is already accounted for by its own layout.
    fn italic_correction(&self, base: &MathItem, size: f32) -> f32 {
        let MathItem::Ident(text) = base else {
            return 0.0;
        };
        let mut characters = text.chars();
        let (Some(character), None) = (characters.next(), characters.next()) else {
            return 0.0;
        };
        self.font
            .glyph(math_italic(character))
            .or_else(|_| self.font.glyph(character))
            .map_or(0.0, |glyph| self.font.italic_correction(glyph, size))
    }

    /// A radical: a sign grown to its content, and a bar across the top.
    fn radical(
        &self,
        radicand: &MathItem,
        degree: Option<&MathItem>,
        size: f32,
        style: MathStyle,
    ) -> Result<MathLayout, LayoutError> {
        let constants = self.constants(size, style);
        let inner = self.item(radicand, size, style)?;

        let thickness = constants.radical_rule_thickness;
        let gap = constants.radical_vertical_gap;

        // The sign has to span the content plus the gap and the bar above it.
        let target = inner.height() + gap + thickness;
        let sign = self.font.stretch_vertical('\u{221A}', target, size)?;

        let bar_top = -(inner.ascent + gap + thickness);
        let mut layout = MathLayout::default();

        // An index, as in a cube root, sits before the sign and raised.
        let mut x = 0.0_f32;
        if let Some(degree) = degree {
            let degree_style = style.script().script();
            let degree_size = self.child_size(size, style, degree_style);
            let degree_layout = self.item(degree, degree_size, degree_style)?;
            let width = degree_layout.width;
            let raise = constants.radical_degree_bottom_raise_percent * sign.height;
            let baseline = bar_top + sign.height - raise;
            x += constants.radical_kern_before_degree;
            layout.absorb(degree_layout, x, baseline);
            x += width + constants.radical_kern_after_degree;
        }

        let mut sign_outline = sign.outline.clone();
        sign_outline.apply_affine(Affine::translate((f64::from(x), f64::from(bar_top))));
        layout.items.push(Placed::Outline(sign_outline));
        layout.ascent = layout
            .ascent
            .max(-bar_top + constants.radical_extra_ascender);
        layout.descent = layout.descent.max(bar_top + sign.height);

        let content_x = x + sign.width;
        layout.absorb(inner, content_x, 0.0);
        let content_width = layout.width - content_x;

        layout.items.push(Placed::Rule {
            x: content_x,
            y: bar_top,
            width: content_width,
            height: thickness,
        });
        layout.width = content_x + content_width;

        Ok(layout)
    }

    /// A group between fences that grow to fit it.
    fn fenced(
        &self,
        open: Option<&Operator>,
        body: &MathItem,
        close: Option<&Operator>,
        size: f32,
        style: MathStyle,
    ) -> Result<MathLayout, LayoutError> {
        let constants = self.constants(size, style);
        let inner = self.item(body, size, style)?;
        let axis = constants.axis_height;

        // A fence is centred on the axis and must reach whichever of the
        // content's edges is further from it, so the pair stays symmetric
        // about the axis rather than about the content's own centre.
        let reach = (inner.ascent - axis).max(inner.descent + axis).max(0.0);
        let target = reach * 2.0;

        let mut layout = MathLayout::default();
        let mut x = 0.0_f32;

        if let Some(open) = open {
            x = self.place_fence(&mut layout, open, target, axis, x, size)?;
        }
        layout.absorb(inner, x, 0.0);
        x = layout.width.max(x);
        if let Some(close) = close {
            x = self.place_fence(&mut layout, close, target, axis, x, size)?;
        }

        layout.width = x;
        Ok(layout)
    }

    /// Draws one fence, grown when it is marked stretchy.
    fn place_fence(
        &self,
        layout: &mut MathLayout,
        fence: &Operator,
        target: f32,
        axis: f32,
        x: f32,
        size: f32,
    ) -> Result<f32, LayoutError> {
        let mut characters = fence.glyph.chars();
        let (Some(character), None) = (characters.next(), characters.next()) else {
            // A multi-character fence is not a stretchable glyph; set it as a
            // run so it is at least drawn correctly.
            let run = self.glyph_run(&fence.glyph, size, IdentStyle::Upright)?;
            let width = run.width;
            layout.absorb(run, x, 0.0);
            return Ok(x + width);
        };

        if !fence.stretchy {
            let run = self.glyph_run(&fence.glyph, size, IdentStyle::Upright)?;
            let width = run.width;
            layout.absorb(run, x, 0.0);
            return Ok(x + width);
        }

        let grown = self.font.stretch_vertical(character, target, size)?;
        // Centre it on the axis.
        let top = -(axis + grown.height / 2.0);
        let mut outline = grown.outline.clone();
        outline.apply_affine(Affine::translate((f64::from(x), f64::from(top))));
        layout.items.push(Placed::Outline(outline));
        layout.ascent = layout.ascent.max(-top);
        layout.descent = layout.descent.max(top + grown.height);
        Ok(x + grown.width)
    }
}

/// Whether an identifier is set slanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentStyle {
    /// Single-letter variables.
    Italic,
    /// Numbers, operators, function names and literal text.
    Upright,
}

/// The mathematical-italic codepoint for a Latin letter.
///
/// Math fonts put the slanted variable shapes in the Mathematical Alphanumeric
/// Symbols block rather than in an italic face, so `x` in a formula is
/// U+1D465, not U+0078 rendered obliquely.
fn math_italic(character: char) -> char {
    // U+1D455 is unassigned; the italic small h lives at U+210E, which is the
    // one hole in an otherwise contiguous block and the classic way to get a
    // missing glyph here.
    const ITALIC_SMALL_H: char = '\u{210E}';

    match character {
        'h' => ITALIC_SMALL_H,
        'a'..='z' => offset(character, 'a', 0x1D44E),
        'A'..='Z' => offset(character, 'A', 0x1D434),
        other => other,
    }
}

fn offset(character: char, base: char, target: u32) -> char {
    let index = character as u32 - base as u32;
    char::from_u32(target + index).unwrap_or(character)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italic_maps_latin_letters_into_the_math_block() {
        assert_eq!(math_italic('x'), '\u{1D465}');
        assert_eq!(math_italic('a'), '\u{1D44E}');
        assert_eq!(math_italic('A'), '\u{1D434}');
        assert_eq!(math_italic('Z'), '\u{1D44D}');
    }

    /// U+1D455 is a hole in the block. Mapping into it yields a codepoint no
    /// font has, so the letter would silently vanish.
    #[test]
    fn italic_small_h_avoids_the_unassigned_slot() {
        assert_eq!(math_italic('h'), '\u{210E}');
        assert_ne!(math_italic('h'), '\u{1D455}');
    }

    #[test]
    fn non_latin_characters_pass_through_unchanged() {
        assert_eq!(math_italic('α'), 'α');
        assert_eq!(math_italic('1'), '1');
        assert_eq!(math_italic('+'), '+');
    }
}
