//! Canvas text through Core Text: the styled text a canvas names, laid out
//! by the platform's typesetter and drawn as the glyph outlines and
//! decoration rectangles of its lines.
//!
//! The typesetter is the one the `Text` leaf's label uses: the attributed
//! string is built by the same kit routine from the same resolved fonts,
//! letter spacing and line height, and a Core Text frame breaks it at the
//! requested width. Each line's runs become glyph runs over font
//! descriptors registered with the layout; colours, backgrounds and the
//! underline and strikethrough rectangles come from the styled text's own
//! chunks. Glyphs are outlines like every glyph run a canvas draws
//! ([`super::glyphs`]), so a text layer is vector content under its
//! placement transform.
//!
//! # Safety
//!
//! The `unsafe` calls hand Core Text an attributed string, a path and
//! buffers that outlive each call, and read the lines, runs, glyphs and
//! positions it returns for the frame that holds them.

use core::ptr::{self, NonNull};
use std::rc::Rc;
use std::sync::Arc;

use cocoa_ui::MainThreadMarker;
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2_core_foundation::{
    CFAttributedString, CFDictionary, CFRange, CFRetained, CFString, CGPoint, CGRect, CGSize,
};
use cocoa_ui::objc2_core_graphics::CGPath;
use cocoa_ui::objc2_core_text::{
    CTFont, CTFontDescriptor, CTFrame, CTFramesetter, CTLine, CTRun, kCTFontAttributeName,
};
use cocoa_ui::objc2_foundation::{NSNumber, NSRange, NSString};
use kurbo::Rect;
use waterui::Str;
use waterui::graphics::color::WorkingColor;
use waterui::reactive::Signal as _;
use waterui::resolve::Resolvable as _;
use waterui::text::StyledStr;
use waterui::theme::color::Foreground;
use waterui_core::layout::{HorizontalAlignment, Size};
use waterui_graphics::draw::{
    Draw as _, Fixed, FontId, Glyph, GlyphRun, GlyphStyle, Paint, Picture,
};
use waterui_graphics::resources::Handle;
use waterui_graphics::{ResourceError, TextBackend, TextRegistration, TextRequest};

use super::resources::{Registry, TextHandle};
use crate::components::font::platform_font;

/// One primitive of a laid-out text, in the text box's coordinates.
#[derive(Clone, Debug)]
enum Primitive {
    Glyphs(GlyphRun, Paint),
    Fill(Rect, Paint),
}

/// A text laid out and ready to draw: its primitives in drawing order, the
/// colour the unstyled chunks draw in, and the fonts its runs name.
#[derive(Debug)]
pub struct LaidOutText {
    primitives: Vec<Primitive>,
    default: Paint,
    fonts: Vec<FontId>,
}

impl LaidOutText {
    /// The fonts the text's runs name.
    pub(super) fn fonts(&self) -> &[FontId] {
        &self.fonts
    }

    /// The text as a picture; `paint` replaces the colour the text draws in
    /// by default.
    pub(super) fn picture(&self, paint: Option<&Paint>) -> Picture {
        let painted = |own: &Paint| match paint {
            Some(with) if *own == self.default => with.clone(),
            _ => own.clone(),
        };
        Picture::record(|recorder| {
            for primitive in &self.primitives {
                match primitive {
                    Primitive::Glyphs(run, paint) => {
                        recorder.glyphs::<Paint>(Fixed(run.clone()), Fixed(painted(paint)));
                    }
                    Primitive::Fill(rect, paint) => {
                        recorder.fill::<Rect, Paint>(Fixed(*rect), Fixed(painted(paint)));
                    }
                }
            }
        })
    }
}

/// What one chunk of the styled text draws with.
struct ChunkStyle {
    /// The chunk's span of the plain text in UTF-16 units.
    units: core::ops::Range<usize>,
    foreground: WorkingColor,
    background: Option<WorkingColor>,
    underline: bool,
    strikethrough: bool,
}

/// The canvas's text registry: lays the text out with Core Text and
/// registers the result with the target's [`Registry`].
pub struct CoreText {
    mtm: MainThreadMarker,
    registry: Rc<Registry>,
}

impl CoreText {
    /// A registry laying text out for `registry`'s canvas.
    #[must_use]
    pub const fn new(mtm: MainThreadMarker, registry: Rc<Registry>) -> Self {
        Self { mtm, registry }
    }
}

impl core::fmt::Debug for CoreText {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CoreText").finish_non_exhaustive()
    }
}

/// The flush a line takes in its box: `0` leading, `1` trailing.
fn flush(alignment: HorizontalAlignment) -> f64 {
    if alignment == HorizontalAlignment::Center {
        0.5
    } else if alignment == HorizontalAlignment::Trailing {
        1.0
    } else {
        0.0
    }
}

fn chunk_styles(styled: &StyledStr, env: &waterui_core::Environment) -> Vec<ChunkStyle> {
    let default = Foreground.resolve(env).snapshot();
    let mut start = 0;
    styled
        .chunks()
        .iter()
        .map(|(text, style): &(Str, _)| {
            let end = start + text.encode_utf16().count();
            let chunk = ChunkStyle {
                units: start..end,
                foreground: style
                    .get_foreground()
                    .as_ref()
                    .map_or(default, |color| color.resolve(env).snapshot()),
                background: style
                    .get_background()
                    .as_ref()
                    .map(|color| color.resolve(env).snapshot()),
                underline: style.get_underline(),
                strikethrough: style.get_strikethrough(),
            };
            start = end;
            chunk
        })
        .collect()
}

/// Typesets `styled`: the Core Text frame of its attributed string at
/// `width`, and the box it fills. `chunks` are the styled text's chunks;
/// each is tagged so that no run of glyphs spans two of them.
fn typeset(
    mtm: MainThreadMarker,
    styled: &StyledStr,
    chunks: &[ChunkStyle],
    env: &waterui_core::Environment,
    width: Option<f32>,
) -> (CFRetained<CTFrame>, CGSize) {
    let fonts: Vec<_> = styled
        .chunks()
        .iter()
        .map(|(_, style)| {
            platform_font(
                mtm,
                &style.get_font().resolve(env).snapshot(),
                style.get_italic(),
            )
        })
        .collect();
    let resolved: Vec<_> = styled
        .chunks()
        .iter()
        .map(|(_, style)| style.get_font().resolve(env).snapshot())
        .collect();
    let runs: Vec<cocoa_ui::text::TextRun<'_>> = styled
        .chunks()
        .iter()
        .enumerate()
        .map(|(index, (text, _))| cocoa_ui::text::TextRun {
            text: text.as_str(),
            font: &fonts[index],
            foreground: None,
            background: None,
            underline: false,
            strikethrough: false,
            letter_spacing: f64::from(resolved[index].letter_spacing()),
            line_height: resolved[index].line_height().map_or(0.0, f64::from),
        })
        .collect();
    let attributed = cocoa_ui::text::build(mtm, &runs);
    // Core Text merges adjacent runs whose attributes agree. The
    // decorations and colours are not attributes of the string, so each
    // chunk is tagged to keep its glyphs in runs of its own.
    let tag = NSString::from_str("dev.waterui.canvas.chunk");
    for (index, chunk) in chunks.iter().enumerate() {
        let value = NSNumber::numberWithUnsignedInteger(index);
        let range = NSRange::new(chunk.units.start, chunk.units.len());
        // SAFETY: the string is live, the range lies inside it and the
        // value is an object.
        unsafe { attributed.addAttribute_value_range(&tag, value.as_ref(), range) };
    }
    // SAFETY: an `NSAttributedString` is a `CFAttributedString`
    // (toll-free bridged) and lives across the calls below.
    let string: &CFAttributedString =
        unsafe { &*Retained::as_ptr(&attributed).cast::<CFAttributedString>() };
    // SAFETY: the string is live.
    let setter = unsafe { CTFramesetter::with_attributed_string(string) };
    let constraint = CGSize::new(width.map_or(f64::MAX, f64::from), f64::MAX);
    // SAFETY: the setter is live; a null fit range and attributes are
    // allowed.
    let size = unsafe {
        setter.suggest_frame_size_with_constraints(
            CFRange::new(0, 0),
            None,
            constraint,
            ptr::null_mut(),
        )
    };
    let box_width = width.map_or_else(|| size.width.ceil() + 1.0, f64::from);
    // SAFETY: a null transform is the identity.
    let path = unsafe {
        CGPath::with_rect(
            CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(box_width, size.height.ceil() + 1.0),
            ),
            ptr::null(),
        )
    };
    // SAFETY: the setter and path are live; null frame attributes are
    // allowed.
    let frame = unsafe { setter.frame(CFRange::new(0, 0), &path, None) };
    (frame, size)
}

impl TextBackend for CoreText {
    fn register_text(
        &self,
        request: TextRequest,
    ) -> Result<Option<TextRegistration>, ResourceError> {
        let styled = request
            .text()
            .get::<StyledStr>()
            .expect("canvas text is the styled text `waterui-text` converts into");
        if styled.to_plain().is_empty() {
            return Ok(None);
        }
        let env = request.environment();
        let placement = request.placement();
        let chunks = chunk_styles(styled, env);
        let (frame, size) = typeset(self.mtm, styled, &chunks, env, placement.get_width());
        let default = Paint::Solid(Foreground.resolve(env).snapshot());
        let box_width = placement.get_width().map_or(size.width, f64::from);
        let box_height = size.height.ceil() + 1.0;

        // SAFETY: the frame is live; its lines are `CTLine`s.
        let lines = unsafe { frame.lines() };
        // SAFETY: a frame's lines are `CTLine`s.
        let lines = unsafe { lines.cast_unchecked::<CTLine>() };
        let mut origins = vec![CGPoint::new(0.0, 0.0); lines.len()];
        if let Some(first) = NonNull::new(origins.as_mut_ptr()) {
            // SAFETY: the buffer holds a point for every line of the frame.
            unsafe { frame.line_origins(CFRange::new(0, 0), first) };
        }
        let mut primitives = Vec::new();
        let mut faces: Vec<(CFRetained<CTFontDescriptor>, FontId)> = Vec::new();
        for (index, origin) in origins.iter().enumerate() {
            let line = lines.get(index).expect("the frame has the line it counted");
            // SAFETY: the line is live.
            let offset =
                unsafe { line.pen_offset_for_flush(flush(placement.alignment()), box_width) };
            let left = origin.x + offset;
            let baseline = box_height - origin.y;
            let mut glyph_runs = Vec::new();
            let mut backgrounds = Vec::new();
            let mut underlines = Vec::new();
            let mut strikes = Vec::new();
            // SAFETY: the line is live; its runs are `CTRun`s.
            let runs = unsafe { line.glyph_runs() };
            // SAFETY: a line's runs are `CTRun`s.
            let runs = unsafe { runs.cast_unchecked::<CTRun>() };
            for run in runs.iter() {
                let (glyphs, extent) = self.run(&run, left, baseline, &mut faces)?;
                // SAFETY: the run is live.
                let start = usize::try_from(unsafe { run.string_range() }.location)
                    .expect("a run starts inside its string");
                let chunk = chunks
                    .iter()
                    .find(|chunk| chunk.units.contains(&start))
                    .expect("a run starts inside a chunk");
                if let Some(color) = chunk.background {
                    backgrounds.push((extent.rect(baseline), Paint::Solid(color)));
                }
                let paint = Paint::Solid(chunk.foreground);
                if chunk.underline {
                    underlines.push((extent.underline(baseline), paint.clone()));
                }
                if chunk.strikethrough {
                    strikes.push((extent.strikethrough(baseline), paint.clone()));
                }
                glyph_runs.push((glyphs, paint));
            }
            primitives.extend(
                backgrounds
                    .into_iter()
                    .chain(underlines)
                    .map(|(rect, paint)| Primitive::Fill(rect, paint)),
            );
            primitives.extend(
                glyph_runs
                    .into_iter()
                    .map(|(run, paint)| Primitive::Glyphs(run, paint)),
            );
            primitives.extend(
                strikes
                    .into_iter()
                    .map(|(rect, paint)| Primitive::Fill(rect, paint)),
            );
        }
        let fonts = faces.into_iter().map(|(_, id)| id).collect();
        let handle = TextHandle::new(
            &self.registry,
            LaidOutText {
                primitives,
                default,
                fonts,
            },
        );
        #[expect(
            clippy::cast_possible_truncation,
            reason = "text boxes are logical points well inside f32"
        )]
        let size = Size::new(box_width as f32, size.height as f32);
        Ok(Some(TextRegistration::new(Handle::new(handle), size)))
    }
}

/// Where a run sits on its line, for the rectangles drawn with it.
struct Extent {
    left: f64,
    width: f64,
    ascent: f64,
    descent: f64,
    underline_offset: f64,
    thickness: f64,
    strike_offset: f64,
}

impl Extent {
    /// The run's highlight: from its ascent to its descent.
    fn rect(&self, baseline: f64) -> Rect {
        Rect::new(
            self.left,
            baseline - self.ascent,
            self.left + self.width,
            baseline + self.descent,
        )
    }

    fn bar(&self, baseline: f64, centre_below: f64) -> Rect {
        let centre = baseline + centre_below;
        Rect::new(
            self.left,
            centre - self.thickness / 2.0,
            self.left + self.width,
            centre + self.thickness / 2.0,
        )
    }

    /// Below the baseline, at the font's underline position.
    fn underline(&self, baseline: f64) -> Rect {
        self.bar(baseline, self.underline_offset)
    }

    /// Through the middle of the lowercase letters.
    fn strikethrough(&self, baseline: f64) -> Rect {
        self.bar(baseline, -self.strike_offset)
    }
}

impl CoreText {
    /// The glyph run of `run`, on the line whose pen starts at `left` and
    /// whose baseline is `baseline` below the box's top, and its extent.
    /// Each distinct font registers once in `faces`.
    fn run(
        &self,
        run: &CTRun,
        left: f64,
        baseline: f64,
        faces: &mut Vec<(CFRetained<CTFontDescriptor>, FontId)>,
    ) -> Result<(GlyphRun, Extent), ResourceError> {
        // SAFETY: the run is live.
        let count = unsafe { run.glyph_count() };
        let length = usize::try_from(count).expect("a run has a non-negative glyph count");
        let mut ids = vec![0_u16; length];
        let mut positions = vec![CGPoint::new(0.0, 0.0); length];
        if let (Some(ids), Some(positions)) = (
            NonNull::new(ids.as_mut_ptr()),
            NonNull::new(positions.as_mut_ptr()),
        ) {
            // SAFETY: both buffers hold an entry for each of the run's
            // glyphs.
            unsafe {
                run.glyphs(CFRange::new(0, 0), ids);
                run.positions(CFRange::new(0, 0), positions);
            }
        }
        // SAFETY: the run is live.
        let attributes = unsafe { run.attributes() };
        // SAFETY: the static is a `CFString` constant Core Text exports.
        let key: &CFString = unsafe { kCTFontAttributeName };
        let font = attributes_font(&attributes, key)
            .ok_or_else(|| ResourceError::Font("a Core Text run draws with no font".into()))?;
        // SAFETY: the font is live.
        let descriptor = unsafe { font.font_descriptor() };
        let known = faces
            .iter()
            .find(|(known, _)| **known == *descriptor)
            .map(|(_, id)| *id);
        let font_id = known.unwrap_or_else(|| {
            let id = self.registry.add_font(descriptor.clone());
            faces.push((descriptor.clone(), id));
            id
        });
        // SAFETY: the font is live.
        let (size, underline_position, thickness, x_height) = unsafe {
            (
                font.size(),
                font.underline_position(),
                font.underline_thickness(),
                font.x_height(),
            )
        };
        let (mut ascent, mut descent, mut leading) = (0.0, 0.0, 0.0);
        // SAFETY: the run is live and the out-pointers are valid.
        let width = unsafe {
            run.typographic_bounds(
                CFRange::new(0, 0),
                &raw mut ascent,
                &raw mut descent,
                &raw mut leading,
            )
        };
        let origin = positions.first().map_or(0.0, |position| position.x);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "glyph positions are f32 by contract"
        )]
        let glyphs: Arc<[Glyph]> = ids
            .iter()
            .zip(&positions)
            .map(|(&id, position)| Glyph {
                id: u32::from(id),
                x: (left + position.x) as f32,
                y: (baseline - position.y) as f32,
                transform: None,
            })
            .collect();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "font sizes are f32 by contract"
        )]
        let run = GlyphRun {
            font: font_id,
            size: size as f32,
            coords: Arc::from([]),
            glyphs,
            style: GlyphStyle::Fill,
        };
        let extent = Extent {
            left: left + origin,
            width,
            ascent,
            descent,
            underline_offset: -underline_position,
            thickness,
            strike_offset: x_height / 2.0,
        };
        Ok((run, extent))
    }
}

/// The font a run's attribute dictionary names.
fn attributes_font(attributes: &CFDictionary, key: &CFString) -> Option<CFRetained<CTFont>> {
    // SAFETY: the dictionary is live; `kCTFontAttributeName` maps to a
    // `CTFont`.
    let value = unsafe { attributes.value(ptr::from_ref(key).cast()) };
    let value = NonNull::new(value.cast_mut())?;
    // SAFETY: the value is a live `CTFont` the dictionary holds; retaining
    // it extends the borrow past the dictionary.
    Some(unsafe { CFRetained::retain(value.cast::<CTFont>()) })
}
