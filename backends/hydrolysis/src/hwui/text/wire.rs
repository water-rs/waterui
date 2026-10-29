//! The text wire: how a shaping request and its reply cross the JNI
//! boundary to the Kotlin `HwuiTextProvider`.
//!
//! A request is the text as a `String`; its style runs packed as
//! [`SPAN_WORDS`] `int`s each over UTF-16 offsets, colours as `ColorLong`s
//! in two words; the distinct family lists the runs index as a `String[]`,
//! each list's names joined by [`FAMILY_SEPARATOR`]; the locale as a BCP 47
//! tag; and the wrap width, line limit and [`paragraph`] flags. A reply is
//! one `float[]`: the layout width, height, ink top and ink bottom, then
//! [`LINE_WORDS`] floats per line — advance, line height, baseline, ink
//! left, ink right — where a span without ink carries `+inf, -inf`.

use std::ops::Range;
use waterui_graphics::draw::kurbo::Rect;

use crate::hwui::HwuiError;

use super::index::Utf16Index;
use crate::text::types::LineMetrics;

/// `int`s per packed style run.
pub const SPAN_WORDS: usize = 12;

codes! {
    /// Span field indices.
    span: usize => "TextWire" {
        /// The run's first UTF-16 offset.
        START = 0;
        /// The run's end UTF-16 offset.
        END = 1;
        /// The run's family list index, or [`DEFAULT_FAMILY`](super::DEFAULT_FAMILY).
        FAMILY = 2;
        /// The font size in pixels, as float bits.
        SIZE = 3;
        /// The weight, 1 to 1000.
        WEIGHT = 4;
        /// The [`run_flags`](super::run_flags).
        FLAGS = 5;
        /// The foreground `ColorLong`, low word then high word.
        FOREGROUND = 6;
        /// The background `ColorLong`, low word then high word.
        BACKGROUND = 8;
        /// The line height in pixels, as float bits.
        LINE_HEIGHT = 10;
        /// The letter spacing in ems, as float bits.
        LETTER_SPACING = 11;
    }
}

codes! {
    /// A span's flag bits.
    run_flags: i32 => "TextWire" {
        /// Italic.
        ITALIC = 1;
        /// Underlined.
        UNDERLINE = 1 << 1;
        /// Struck through.
        STRIKETHROUGH = 1 << 2;
        /// The foreground words are set.
        HAS_FOREGROUND = 1 << 3;
        /// The background words are set.
        HAS_BACKGROUND = 1 << 4;
        /// The line height word is set.
        HAS_LINE_HEIGHT = 1 << 5;
    }
}

codes! {
    /// A request's paragraph flags.
    paragraph: i32 => "TextWire" {
        /// The bits holding the alignment code.
        ALIGN_MASK = 0b11;
        /// Lines start at the paragraph's start edge.
        ALIGN_START = 0;
        /// Lines are centred.
        ALIGN_CENTER = 1;
        /// Lines end at the paragraph's end edge.
        ALIGN_END = 2;
        /// The paragraph runs right to left.
        RIGHT_TO_LEFT = 1 << 2;
        /// The last allowed line ends in an ellipsis when text remains.
        ELLIPSIS = 1 << 3;
        /// A family neither registered nor known to the system is an error,
        /// not skipped.
        STRICT_FAMILIES = 1 << 4;
    }
}

/// The family index of a run that names none: the platform default.
pub const DEFAULT_FAMILY: i32 = -1;
/// The wrap width of a request that does not wrap.
pub const UNBOUNDED: f32 = -1.0;
/// The line limit of a request without one.
pub const NO_LINE_LIMIT: i32 = -1;
/// What joins the names of one family list.
pub const FAMILY_SEPARATOR: char = '\u{1f}';

/// Floats before the first line of a reply.
pub const REPLY_HEADER: usize = 4;
/// Floats per line of a reply.
pub const LINE_WORDS: usize = 5;

/// One style run of a shaping request, over a byte range of the text.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextRun {
    /// The byte range the run covers, on character boundaries.
    pub range: Range<usize>,
    /// The CSS `font-family` list, each family registered with the provider
    /// or known to the system; `None` is the platform default.
    pub family: Option<String>,
    /// The font size in pixels.
    pub size: f32,
    /// The weight, 1 to 1000.
    pub weight: u16,
    /// Italic.
    pub italic: bool,
    /// Underlined.
    pub underline: bool,
    /// Struck through.
    pub strikethrough: bool,
    /// The foreground `ColorLong`; `None` draws with the paint's colour.
    pub foreground: Option<u64>,
    /// The background `ColorLong`.
    pub background: Option<u64>,
    /// An explicit line height in pixels.
    pub line_height: Option<f32>,
    /// Extra letter spacing in pixels.
    pub letter_spacing: f32,
}

impl TextRun {
    /// The byte range the run covers, on character boundaries.
    #[must_use]
    pub const fn range(&self) -> &Range<usize> {
        &self.range
    }

    /// Sets the `range`.
    pub const fn set_range(&mut self, range: Range<usize>) {
        self.range = range;
    }

    /// The CSS `font-family` list, each family registered with the provider
    /// or known to the system; `None` is the platform default.
    #[must_use]
    pub fn family(&self) -> Option<&str> {
        self.family.as_deref()
    }

    /// Sets the `family`.
    pub fn set_family(&mut self, family: Option<String>) {
        self.family = family;
    }

    /// The font size in pixels.
    #[must_use]
    pub const fn size(&self) -> f32 {
        self.size
    }

    /// Sets the `size`.
    pub const fn set_size(&mut self, size: f32) {
        self.size = size;
    }

    /// The weight, 1 to 1000.
    #[must_use]
    pub const fn weight(&self) -> u16 {
        self.weight
    }

    /// Sets the `weight`.
    pub const fn set_weight(&mut self, weight: u16) {
        self.weight = weight;
    }

    /// Italic.
    #[must_use]
    pub const fn italic(&self) -> bool {
        self.italic
    }

    /// Sets the `italic`.
    pub const fn set_italic(&mut self, italic: bool) {
        self.italic = italic;
    }

    /// Underlined.
    #[must_use]
    pub const fn underline(&self) -> bool {
        self.underline
    }

    /// Sets the `underline`.
    pub const fn set_underline(&mut self, underline: bool) {
        self.underline = underline;
    }

    /// Struck through.
    #[must_use]
    pub const fn strikethrough(&self) -> bool {
        self.strikethrough
    }

    /// Sets the `strikethrough`.
    pub const fn set_strikethrough(&mut self, strikethrough: bool) {
        self.strikethrough = strikethrough;
    }

    /// The foreground `ColorLong`; `None` draws with the paint's colour.
    #[must_use]
    pub const fn foreground(&self) -> Option<u64> {
        self.foreground
    }

    /// Sets the `foreground`.
    pub const fn set_foreground(&mut self, foreground: Option<u64>) {
        self.foreground = foreground;
    }

    /// The background `ColorLong`.
    #[must_use]
    pub const fn background(&self) -> Option<u64> {
        self.background
    }

    /// Sets the `background`.
    pub const fn set_background(&mut self, background: Option<u64>) {
        self.background = background;
    }

    /// An explicit line height in pixels.
    #[must_use]
    pub const fn line_height(&self) -> Option<f32> {
        self.line_height
    }

    /// Sets the `line_height`.
    pub const fn set_line_height(&mut self, line_height: Option<f32>) {
        self.line_height = line_height;
    }

    /// Extra letter spacing in pixels.
    #[must_use]
    pub const fn letter_spacing(&self) -> f32 {
        self.letter_spacing
    }

    /// Sets the `letter_spacing`.
    pub const fn set_letter_spacing(&mut self, letter_spacing: f32) {
        self.letter_spacing = letter_spacing;
    }
}

impl TextRun {
    /// A run in the platform's default family at `size` pixels and `weight`
    /// (1 to 1000), without decorations or explicit colours or spacing.
    #[must_use]
    pub const fn new(range: Range<usize>, size: f32, weight: u16) -> Self {
        Self {
            range,
            family: None,
            size,
            weight,
            italic: false,
            underline: false,
            strikethrough: false,
            foreground: None,
            background: None,
            line_height: None,
            letter_spacing: 0.0,
        }
    }
}

/// Where lines sit inside the wrap width, relative to the paragraph
/// direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextAlignment {
    /// The paragraph's start edge.
    Start,
    /// Centred.
    Center,
    /// The paragraph's end edge.
    End,
}

impl TextAlignment {
    const fn code(self) -> i32 {
        match self {
            Self::Start => paragraph::ALIGN_START,
            Self::Center => paragraph::ALIGN_CENTER,
            Self::End => paragraph::ALIGN_END,
        }
    }
}

/// A layout to shape: the text, its runs, the paragraph's base style, and
/// the paragraph.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ShapeRequest<'a> {
    /// The text.
    pub text: &'a str,
    /// The style runs, each over a byte range of `text`.
    pub runs: &'a [TextRun],
    /// The paragraph's base paint, sent as row zero of every packed request:
    /// a zero-length run whose size, weight, family and line height set the
    /// styled text's defaults — required even when `text` is empty.
    pub default_style: TextRun,
    /// The BCP 47 locale shaping and line breaking follow; empty is the
    /// root locale.
    pub locale: &'a str,
    /// The wrap width; `None` lays every paragraph out on one line.
    pub max_width: Option<f32>,
    /// The most lines laid out; `None` lays out every line.
    pub max_lines: Option<usize>,
    /// Whether the last allowed line ends in an ellipsis when text remains;
    /// needs [`ShapeRequest::max_lines`].
    pub ellipsis: bool,
    /// The line alignment.
    pub alignment: TextAlignment,
    /// Whether the paragraph runs right to left.
    pub right_to_left: bool,
    /// Whether a family neither registered nor known to the system is an
    /// error (the seam's strict resolution) rather than skipped.
    pub strict_families: bool,
}

impl<'a> ShapeRequest<'a> {
    /// The text.
    #[must_use]
    pub const fn text(&self) -> &'a str {
        self.text
    }

    /// The style runs, each over a byte range of `text`.
    #[must_use]
    pub const fn runs(&self) -> &'a [TextRun] {
        self.runs
    }

    /// The paragraph's base paint, sent as row zero of every packed request:
    /// a zero-length run whose size, weight, family and line height set the
    /// styled text's defaults — required even when `text` is empty.
    #[must_use]
    pub const fn default_style(&self) -> &TextRun {
        &self.default_style
    }

    /// The BCP 47 locale shaping and line breaking follow; empty is the
    /// root locale.
    #[must_use]
    pub const fn locale(&self) -> &'a str {
        self.locale
    }

    /// Sets the `locale`.
    pub const fn set_locale(&mut self, locale: &'a str) {
        self.locale = locale;
    }

    /// The wrap width; `None` lays every paragraph out on one line.
    #[must_use]
    pub const fn max_width(&self) -> Option<f32> {
        self.max_width
    }

    /// Sets the `max_width`.
    pub const fn set_max_width(&mut self, max_width: Option<f32>) {
        self.max_width = max_width;
    }

    /// The most lines laid out; `None` lays out every line.
    #[must_use]
    pub const fn max_lines(&self) -> Option<usize> {
        self.max_lines
    }

    /// Sets the `max_lines`.
    pub const fn set_max_lines(&mut self, max_lines: Option<usize>) {
        self.max_lines = max_lines;
    }

    /// Whether the last allowed line ends in an ellipsis when text remains;
    /// needs [`ShapeRequest::max_lines`].
    #[must_use]
    pub const fn ellipsis(&self) -> bool {
        self.ellipsis
    }

    /// Sets the `ellipsis`.
    pub const fn set_ellipsis(&mut self, ellipsis: bool) {
        self.ellipsis = ellipsis;
    }

    /// The line alignment.
    #[must_use]
    pub const fn alignment(&self) -> TextAlignment {
        self.alignment
    }

    /// Sets the `alignment`.
    pub const fn set_alignment(&mut self, alignment: TextAlignment) {
        self.alignment = alignment;
    }

    /// Whether the paragraph runs right to left.
    #[must_use]
    pub const fn right_to_left(&self) -> bool {
        self.right_to_left
    }

    /// Sets the `right_to_left`.
    pub const fn set_right_to_left(&mut self, right_to_left: bool) {
        self.right_to_left = right_to_left;
    }

    /// Whether a family neither registered nor known to the system is an
    /// error (the seam's strict resolution) rather than skipped.
    #[must_use]
    pub const fn strict_families(&self) -> bool {
        self.strict_families
    }

    /// Sets the `strict_families`.
    pub const fn set_strict_families(&mut self, strict_families: bool) {
        self.strict_families = strict_families;
    }
}

impl<'a> ShapeRequest<'a> {
    /// `text` with `runs` over the `default_style` base paint, unwrapped
    /// and unlimited, start-aligned, left to right, in the root locale,
    /// skipping unknown families.
    #[must_use]
    pub const fn new(text: &'a str, runs: &'a [TextRun], default_style: TextRun) -> Self {
        Self {
            text,
            runs,
            default_style,
            locale: "",
            max_width: None,
            max_lines: None,
            ellipsis: false,
            alignment: TextAlignment::Start,
            right_to_left: false,
            strict_families: false,
        }
    }
}

/// A [`ShapeRequest`] packed for the provider's `shape` call.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PackedRequest {
    /// The number of runs: row zero is always the default style's base
    /// paint, the rest the text's style runs.
    pub span_count: i32,
    /// The runs, `SPAN_WORDS` per run, offsets in UTF-16 units.
    pub spans: Vec<i32>,
    /// The distinct family lists the runs index, names joined by
    /// [`FAMILY_SEPARATOR`].
    pub families: Vec<String>,
    /// The BCP 47 locale.
    pub locale: String,
    /// The wrap width, [`UNBOUNDED`] when unbounded.
    pub max_width: f32,
    /// The line limit, [`NO_LINE_LIMIT`] without one.
    pub max_lines: i32,
    /// The [`paragraph`] flags.
    pub paragraph: i32,
}

impl PackedRequest {
    /// The number of runs: row zero is always the default style's base
    /// paint, the rest the text's style runs.
    #[must_use]
    pub const fn span_count(&self) -> i32 {
        self.span_count
    }

    /// The runs, `SPAN_WORDS` per run, offsets in UTF-16 units.
    #[must_use]
    pub const fn spans(&self) -> &[i32] {
        self.spans.as_slice()
    }

    /// The distinct family lists the runs index, names joined by
    /// [`FAMILY_SEPARATOR`].
    #[must_use]
    pub const fn families(&self) -> &[String] {
        self.families.as_slice()
    }

    /// The BCP 47 locale.
    #[must_use]
    pub const fn locale(&self) -> &str {
        self.locale.as_str()
    }

    /// The wrap width, [`UNBOUNDED`] when unbounded.
    #[must_use]
    pub const fn max_width(&self) -> f32 {
        self.max_width
    }

    /// The line limit, [`NO_LINE_LIMIT`] without one.
    #[must_use]
    pub const fn max_lines(&self) -> i32 {
        self.max_lines
    }

    /// The [`paragraph`] flags.
    #[must_use]
    pub const fn paragraph(&self) -> i32 {
        self.paragraph
    }
}

const fn refused(reason: String) -> HwuiError {
    HwuiError::Text { reason }
}

const fn int_bits(word: u32) -> i32 {
    i32::from_ne_bytes(word.to_ne_bytes())
}

/// A `ColorLong` as its low and high words.
const fn color_words(color: u64) -> [i32; 2] {
    let bytes = color.to_le_bytes();
    [
        i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    ]
}

impl ShapeRequest<'_> {
    /// Packs the request, converting run offsets with `index` (the index of
    /// [`ShapeRequest::text`]).
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] for a run off the text's character boundaries, a
    /// non-positive or non-finite size or line height, a weight outside
    /// 1 to 1000, a malformed family list, a negative or non-finite wrap
    /// width, a zero or oversized line limit, or an ellipsis without one.
    pub fn pack(&self, index: &Utf16Index) -> Result<PackedRequest, HwuiError> {
        if self.default_style.range != (0..0) {
            return Err(refused(format!(
                "the default style must cover no text, got {:?}",
                self.default_style.range
            )));
        }
        let span_count = i32::try_from(self.runs.len() + 1).map_err(|_| {
            refused(format!(
                "{} style runs are more than an int counts",
                self.runs.len() + 1
            ))
        })?;
        let max_width = match self.max_width {
            None => UNBOUNDED,
            Some(width) if width.is_finite() && width >= 0.0 => width,
            Some(width) => return Err(refused(format!("wrap width {width}"))),
        };
        let max_lines = match self.max_lines {
            None => NO_LINE_LIMIT,
            Some(lines) => i32::try_from(lines)
                .ok()
                .filter(|&lines| lines > 0)
                .ok_or_else(|| refused(format!("a limit of {lines} lines")))?,
        };
        if self.ellipsis && self.max_lines.is_none() {
            return Err(refused(
                "an ellipsis without a line limit truncates nothing".to_owned(),
            ));
        }
        let mut spans = Vec::with_capacity((self.runs.len() + 1) * SPAN_WORDS);
        let mut families: Vec<String> = Vec::new();
        for run in std::iter::once(&self.default_style).chain(self.runs.iter()) {
            spans.extend_from_slice(&pack_run(run, index, &mut families)?);
        }
        let mut flags = self.alignment.code();
        debug_assert_eq!(
            flags & !paragraph::ALIGN_MASK,
            0,
            "an alignment code fits its bits"
        );
        for (on, flag) in [
            (self.right_to_left, paragraph::RIGHT_TO_LEFT),
            (self.ellipsis, paragraph::ELLIPSIS),
            (self.strict_families, paragraph::STRICT_FAMILIES),
        ] {
            if on {
                flags |= flag;
            }
        }
        Ok(PackedRequest {
            span_count,
            spans,
            families,
            locale: self.locale.to_owned(),
            max_width,
            max_lines,
            paragraph: flags,
        })
    }
}

/// The CSS `font-family` list `css` as its family names joined by
/// [`FAMILY_SEPARATOR`]: quoted names verbatim, unquoted ones with their
/// whitespace collapsed, and the CSS `system-ui` and `ui-*` generics as the
/// Android families they name.
fn family_names(css: &str) -> Result<String, HwuiError> {
    let malformed = |what: String| refused(format!("font family list {css:?}: {what}"));
    let mut names = String::new();
    let mut chars = css.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut name = String::new();
        let name = if let Some(quote) = chars.next_if(|&c| c == '"' || c == '\'') {
            loop {
                match chars.next() {
                    Some(c) if c == quote => break,
                    Some('\\') => {
                        name.push(
                            chars
                                .next()
                                .ok_or_else(|| malformed("ends inside an escape".to_owned()))?,
                        );
                    }
                    Some(c) => name.push(c),
                    None => return Err(malformed("an unterminated quote".to_owned())),
                }
            }
            while chars.next_if(|c| c.is_whitespace()).is_some() {}
            name
        } else {
            while let Some(c) = chars.next_if(|&c| c != ',') {
                name.push(c);
            }
            if let Some(quote) = name.chars().find(|&c| c == '"' || c == '\'') {
                return Err(malformed(format!(
                    "{quote:?} inside the unquoted family {:?}",
                    name.trim()
                )));
            }
            let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
            match name.as_str() {
                "system-ui" | "ui-sans-serif" => "sans-serif".to_owned(),
                "ui-serif" => "serif".to_owned(),
                "ui-monospace" => "monospace".to_owned(),
                _ => name,
            }
        };
        if name.is_empty() {
            return Err(malformed("an empty family name".to_owned()));
        }
        if name.chars().any(char::is_control) {
            return Err(malformed(format!(
                "family {name:?} has a control character"
            )));
        }
        if !names.is_empty() {
            names.push(FAMILY_SEPARATOR);
        }
        names.push_str(&name);
        match chars.next() {
            None => return Ok(names),
            Some(',') => {}
            Some(c) => {
                return Err(malformed(format!("{c:?} after family {name:?}")));
            }
        }
    }
}

fn pack_run(
    run: &TextRun,
    index: &Utf16Index,
    families: &mut Vec<String>,
) -> Result<[i32; SPAN_WORDS], HwuiError> {
    let Range { start, end } = run.range;
    if start > end || !index.is_boundary(start) || !index.is_boundary(end) {
        return Err(refused(format!(
            "style run {start}..{end} is not a range of character boundaries in a {}-byte text",
            index.len()
        )));
    }
    if !(run.size.is_finite() && run.size > 0.0) {
        return Err(refused(format!(
            "style run {start}..{end} has font size {}",
            run.size
        )));
    }
    if !(1..=1000).contains(&run.weight) {
        return Err(refused(format!(
            "style run {start}..{end} has weight {}",
            run.weight
        )));
    }
    let letter_spacing = run.letter_spacing / run.size;
    if !letter_spacing.is_finite() {
        return Err(refused(format!(
            "style run {start}..{end} has letter spacing {}",
            run.letter_spacing
        )));
    }
    let family = match &run.family {
        None => DEFAULT_FAMILY,
        Some(css) => {
            let names = family_names(css)?;
            let at = families
                .iter()
                .position(|known| *known == names)
                .unwrap_or_else(|| {
                    families.push(names);
                    families.len() - 1
                });
            i32::try_from(at)
                .map_err(|_| refused(format!("{at} family lists are more than an int counts")))?
        }
    };
    let mut flags = 0;
    for (on, flag) in [
        (run.italic, run_flags::ITALIC),
        (run.underline, run_flags::UNDERLINE),
        (run.strikethrough, run_flags::STRIKETHROUGH),
        (run.foreground.is_some(), run_flags::HAS_FOREGROUND),
        (run.background.is_some(), run_flags::HAS_BACKGROUND),
        (run.line_height.is_some(), run_flags::HAS_LINE_HEIGHT),
    ] {
        if on {
            flags |= flag;
        }
    }
    let line_height = match run.line_height {
        None => 0.0,
        Some(height) if height.is_finite() && height > 0.0 => height,
        Some(height) => {
            return Err(refused(format!(
                "style run {start}..{end} has line height {height}"
            )));
        }
    };
    let mut packed = [0; SPAN_WORDS];
    packed[span::START] = index.utf16(start);
    packed[span::END] = index.utf16(end);
    packed[span::FAMILY] = family;
    packed[span::SIZE] = int_bits(run.size.to_bits());
    packed[span::WEIGHT] = i32::from(run.weight);
    packed[span::FLAGS] = flags;
    packed[span::FOREGROUND..span::FOREGROUND + 2]
        .copy_from_slice(&color_words(run.foreground.unwrap_or(0)));
    packed[span::BACKGROUND..span::BACKGROUND + 2]
        .copy_from_slice(&color_words(run.background.unwrap_or(0)));
    packed[span::LINE_HEIGHT] = int_bits(line_height.to_bits());
    packed[span::LETTER_SPACING] = int_bits(letter_spacing.to_bits());
    Ok(packed)
}

/// A decoded shaping reply.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedMetrics {
    /// The layout width.
    pub width: f32,
    /// The layout height.
    pub height: f32,
    /// The vertical ink extent of every line together; `None` without ink.
    pub vertical_ink: Option<(f32, f32)>,
    /// Every line's metrics.
    pub lines: Box<[LineMetrics]>,
    /// Every line's horizontal ink extent; `None` for a line without ink.
    pub ink: Box<[Option<(f32, f32)>]>,
}

impl ShapedMetrics {
    /// The metrics of a layout with no lines.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            vertical_ink: None,
            lines: Box::new([]),
            ink: Box::new([]),
        }
    }

    /// The layout box widened to its ink: what its node draws.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        let (left, right) = self
            .ink
            .iter()
            .flatten()
            .fold((0.0f32, self.width), |(left, right), &(l, r)| {
                (left.min(l), right.max(r))
            });
        let (top, bottom) = self
            .vertical_ink
            .map_or((0.0, self.height), |(top, bottom)| {
                (top.min(0.0), bottom.max(self.height))
            });
        Rect::new(
            f64::from(left),
            f64::from(top),
            f64::from(right),
            f64::from(bottom),
        )
    }
}

/// An ink span off the wire, `None` for the empty sentinel; `what` names a
/// malformed one.
fn ink_span(
    low: f32,
    high: f32,
    what: impl FnOnce() -> String,
) -> Result<Option<(f32, f32)>, HwuiError> {
    if low == f32::INFINITY && high == f32::NEG_INFINITY {
        Ok(None)
    } else if low.is_finite() && high.is_finite() && low <= high {
        Ok(Some((low, high)))
    } else {
        Err(refused(format!("{} has ink extent {low}..{high}", what())))
    }
}

/// Reads the provider's packed reply.
///
/// # Errors
///
/// [`HwuiError::Text`] when the reply has no header, a partial line, a
/// non-finite metric, or a one-sided or inverted ink extent.
pub fn decode_reply(reply: &[f32]) -> Result<ShapedMetrics, HwuiError> {
    let Some((&[width, height, top, bottom], lines)) = reply.split_first_chunk::<REPLY_HEADER>()
    else {
        return Err(refused(format!(
            "the shaping reply has {} floats, short of its {REPLY_HEADER}-float header",
            reply.len()
        )));
    };
    for (name, value) in [("width", width), ("height", height)] {
        if !(value.is_finite() && value >= 0.0) {
            return Err(refused(format!("the shaping reply has {name} {value}")));
        }
    }
    let vertical_ink = ink_span(top, bottom, || {
        "the shaping reply's vertical span".to_owned()
    })?;
    let (words, partial) = lines.as_chunks::<LINE_WORDS>();
    if !partial.is_empty() {
        return Err(refused(format!(
            "the shaping reply has {} line words, not a multiple of {LINE_WORDS}",
            lines.len()
        )));
    }
    let mut metrics = Vec::with_capacity(words.len());
    let mut ink = Vec::with_capacity(words.len());
    for (line, &[advance, line_height, baseline, left, right]) in words.iter().enumerate() {
        if ![advance, line_height, baseline]
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(refused(format!(
                "line {line} of the shaping reply has a non-finite metric"
            )));
        }
        metrics.push(LineMetrics {
            advance,
            line_height,
            baseline,
        });
        ink.push(ink_span(left, right, || {
            format!("line {line} of the shaping reply")
        })?);
    }
    Ok(ShapedMetrics {
        width,
        height,
        vertical_ink,
        lines: metrics.into_boxed_slice(),
        ink: ink.into_boxed_slice(),
    })
}

/// A position the provider packs into a `long`: the offset shifted left
/// once, the low bit set when it attaches upstream.
///
/// # Errors
///
/// [`HwuiError::Text`] when the offset is outside an `int`.
pub fn unpack_position(packed: i64) -> Result<(i32, bool), HwuiError> {
    let offset = i32::try_from(packed >> 1).map_err(|_| {
        refused(format!(
            "packed position {packed} is outside the platform's offsets"
        ))
    })?;
    Ok((offset, packed & 1 == 1))
}

/// A range the provider packs into a `long`: the start in the high word,
/// the end in the low word.
#[must_use]
pub const fn unpack_range(packed: i64) -> (i32, i32) {
    let bytes = packed.to_le_bytes();
    (
        i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    )
}

/// How the Kotlin contract test prints the packed `run`: the two must
/// agree.
#[cfg(test)]
pub(in crate::hwui) fn describe_run(run: &TextRun, index: &Utf16Index) -> String {
    let colour = |colour: Option<u64>| {
        colour.map_or_else(|| "none".to_owned(), |colour| format!("{colour:016x}"))
    };
    let family = run.family.as_deref().map_or_else(
        || "default".to_owned(),
        |css| {
            family_names(css)
                .expect("the contract's families parse")
                .replace(FAMILY_SEPARATOR, "|")
        },
    );
    format!(
        "run {}..{} family={family} size={:.3} weight={} italic={} underline={} strikethrough={} foreground={} background={} lineHeight={} letterSpacingEm={:.3}",
        index.utf16(run.range.start),
        index.utf16(run.range.end),
        run.size,
        run.weight,
        run.italic,
        run.underline,
        run.strikethrough,
        colour(run.foreground),
        colour(run.background),
        run.line_height
            .map_or_else(|| "none".to_owned(), |height| format!("{height:.3}")),
        run.letter_spacing / run.size,
    )
}

/// How the Kotlin contract test prints a packed request's paragraph.
#[cfg(test)]
pub(in crate::hwui) fn describe_paragraph(request: &ShapeRequest<'_>) -> String {
    format!(
        "paragraph locale={} maxWidth={} maxLines={} align={} rtl={} ellipsis={} strict={}",
        request.locale,
        request
            .max_width
            .map_or_else(|| "none".to_owned(), |width| format!("{width:.3}")),
        request
            .max_lines
            .map_or_else(|| "none".to_owned(), |lines| lines.to_string()),
        request.alignment.code(),
        request.right_to_left,
        request.ellipsis,
        request.strict_families,
    )
}
