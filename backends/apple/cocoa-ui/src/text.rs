//! Attributed text assembly and measurement, identical in behavior on both
//! platforms.
//!
//! [`TextRun`] describes one styled run, [`build`] turns runs into an
//! attributed string, and [`measure`] lays it out through the same
//! `NSTextStorage`/`NSLayoutManager`/`NSTextContainer` stack `TextKit` uses.
//!
//! # Safety
//!
//! The `unsafe` blocks here call `objc2` bindings that are unsafe only because
//! they ask the caller to promise attribute value types — which [`build`]
//! does by construction — or because a `CFStringTokenizer`/`CFString` call is
//! an FFI boundary over objects this module constructs itself. Every entry
//! point requires a [`MainThreadMarker`], and all mutable state lives in
//! objects created inside each call.

use std::ops::Range;
use std::rc::Rc;

use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, rc::Retained};
use objc2_core_foundation::{
    CFRange, CFStringTokenizer, CFStringTokenizerTokenType, CGPoint, CGRect, CGSize,
    kCFStringTokenizerUnitLineBreak,
};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSCharacterSet, NSIntersectionRange,
    NSIntersectionRect, NSMutableAttributedString, NSNotFound, NSNumber, NSPointInRect, NSRange,
    NSString, NSStringCompareOptions,
};

use crate::font::Font;
use crate::geometry::Size;

#[cfg(target_os = "macos")]
mod platform {
    pub use objc2_app_kit::{
        NSAttributedStringNSExtendedStringDrawing, NSBackgroundColorAttributeName,
        NSColor as Color, NSFontAttributeName, NSForegroundColorAttributeName, NSKernAttributeName,
        NSLayoutManager, NSLineBreakMode, NSParagraphStyleAttributeName,
        NSStrikethroughStyleAttributeName, NSTextAlignment, NSTextContainer, NSTextStorage,
        NSUnderlineStyleAttributeName,
    };
    pub use objc2_app_kit::{NSMutableParagraphStyle, NSParagraphStyle, NSStringDrawingOptions};
}
#[cfg(target_os = "ios")]
mod platform {
    pub use objc2_ui_kit::{
        NSAttributedStringNSExtendedStringDrawing, NSBackgroundColorAttributeName,
        NSFontAttributeName, NSForegroundColorAttributeName, NSKernAttributeName, NSLayoutManager,
        NSLineBreakMode, NSParagraphStyleAttributeName, NSStrikethroughStyleAttributeName,
        NSTextAlignment, NSTextContainer, NSTextStorage, NSUnderlineStyleAttributeName,
        UIColor as Color,
    };
    pub use objc2_ui_kit::{
        NSLineBreakStrategy, NSMutableParagraphStyle, NSParagraphStyle, NSStringDrawingOptions,
    };
}
#[cfg(target_os = "ios")]
use platform::NSLineBreakStrategy;
use platform::{
    Color, NSAttributedStringNSExtendedStringDrawing, NSBackgroundColorAttributeName,
    NSFontAttributeName, NSForegroundColorAttributeName, NSKernAttributeName, NSLayoutManager,
    NSLineBreakMode, NSMutableParagraphStyle, NSParagraphStyle, NSParagraphStyleAttributeName,
    NSStrikethroughStyleAttributeName, NSStringDrawingOptions, NSTextAlignment, NSTextContainer,
    NSTextStorage, NSUnderlineStyleAttributeName,
};

/// One run of text sharing a font, colors and decorations — what a
/// `NSMutableAttributedString` append applies as one attribute set.
///
/// `font` must already carry the wanted face — [`crate::font::italic_variant`]
/// produces an italic face. A `line_height` of `0.0` leaves the typesetter's
/// default pitch in place.
pub struct TextRun<'a> {
    /// The run's content.
    pub text: &'a str,
    /// The face the run draws in.
    pub font: &'a Font,
    /// Text color; absent means the label's own color.
    pub foreground: Option<&'a Color>,
    /// Highlight behind the text.
    pub background: Option<&'a Color>,
    /// Underline the whole run.
    pub underline: bool,
    /// Strike the whole run.
    pub strikethrough: bool,
    /// Tracking added between glyphs, in points.
    pub letter_spacing: f64,
    /// Explicit baseline-to-baseline pitch in points; `0.0` uses the face's.
    pub line_height: f64,
}

impl std::fmt::Debug for TextRun<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextRun")
            .field("text", &self.text)
            .field("underline", &self.underline)
            .field("strikethrough", &self.strikethrough)
            .field("letter_spacing", &self.letter_spacing)
            .field("line_height", &self.line_height)
            .finish_non_exhaustive()
    }
}

/// Builds one attributed string out of styled `runs`, in order.
#[must_use]
pub fn build(mtm: MainThreadMarker, runs: &[TextRun<'_>]) -> Retained<NSMutableAttributedString> {
    let result = NSMutableAttributedString::new();
    for run in runs {
        append(mtm, &result, run);
    }
    result
}

/// Appends `run` to `string` with its attributes set over the appended range.
pub fn append(mtm: MainThreadMarker, string: &NSMutableAttributedString, run: &TextRun<'_>) {
    let text = NSString::from_str(run.text);
    let piece =
        NSMutableAttributedString::initWithString(mtm.alloc::<NSMutableAttributedString>(), &text);
    let range = NSRange::new(0, text.length());

    let set = |name: &NSAttributedStringKey, value: &AnyObject| {
        // SAFETY: `piece` is a live mutable attributed string and each call
        // site passes a value of the attribute's documented type.
        unsafe { piece.addAttribute_value_range(name, value, range) };
    };
    // SAFETY: each attribute name is a static `NSAttributedStringKey` the
    // platform exports; the values passed are of the documented type.
    unsafe {
        set(NSFontAttributeName, run.font.as_ref());
        if let Some(foreground) = run.foreground {
            set(NSForegroundColorAttributeName, foreground.as_ref());
        }
        if let Some(background) = run.background {
            set(NSBackgroundColorAttributeName, background.as_ref());
        }
        if run.underline {
            set(
                NSUnderlineStyleAttributeName,
                NSNumber::numberWithInteger(1).as_ref(),
            );
        }
        if run.strikethrough {
            set(
                NSStrikethroughStyleAttributeName,
                NSNumber::numberWithInteger(1).as_ref(),
            );
        }
        if run.letter_spacing != 0.0 {
            set(
                NSKernAttributeName,
                NSNumber::numberWithDouble(run.letter_spacing).as_ref(),
            );
        }
        if let Some(paragraph) = paragraph_style(mtm, run) {
            set(NSParagraphStyleAttributeName, paragraph.as_ref());
        }
    }
    string.appendAttributedString(&piece);
}

/// The paragraph style a run draws with: on iOS every run carries one — it
/// locks out hyphenation and fixes the break strategy — and on both
/// platforms an explicit `line_height` becomes `lineSpacing` relative to the
/// face's typesetter pitch.
fn paragraph_style(
    mtm: MainThreadMarker,
    run: &TextRun<'_>,
) -> Option<Retained<NSMutableParagraphStyle>> {
    let pitch = crate::font::typesetter_line_height(mtm, run.font);
    let spacing = if run.line_height > 0.0 {
        run.line_height - pitch
    } else {
        0.0
    };
    #[cfg(target_os = "ios")]
    let needs_paragraph = true;
    #[cfg(target_os = "macos")]
    let needs_paragraph = spacing != 0.0;
    if !needs_paragraph {
        return None;
    }
    let style = NSMutableParagraphStyle::new();
    #[cfg(target_os = "ios")]
    {
        style.setHyphenationFactor(f32::MIN_POSITIVE);
        style.setLineBreakStrategy(NSLineBreakStrategy::Standard);
    }
    if spacing != 0.0 {
        style.setLineSpacing(spacing);
    }
    Some(style)
}

/// How a measure call constrains the text's width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WrapWidth {
    /// Never wraps: the natural width is reported.
    Free,
    /// Wraps at this width in points.
    Fixed(f64),
    /// As wide as the widest unbreakable run of the text: what a label needs
    /// when its container offers zero width.
    Unbreakable,
}

/// The size a measured text occupies, plus where its baselines sit so
/// baseline-aligned siblings can line up with it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMetrics {
    /// Bounding size of the laid-out lines.
    pub size: Size,
    /// Distance from the text's top to the first baseline.
    pub first_baseline: Option<f64>,
    /// Distance from the text's top to the last baseline.
    pub last_baseline: Option<f64>,
}

/// The `NSTextStorage`/`NSLayoutManager`/`NSTextContainer` stack one
/// measurement or hit test runs through, laid out at `width` points.
///
/// `NSLayoutManager`'s `textStorage` back-reference is `assign` — it does not
/// retain the storage — so the storage is returned alongside the manager and
/// callers must keep it alive for as long as they query the layout. The stack
/// is returned before layout is ensured: a caller adjusting the storage's
/// attributes (see [`Layout`]) does so before the typesetter runs.
fn stack(
    mtm: MainThreadMarker,
    text: &NSAttributedString,
    width: f64,
) -> (
    Retained<NSTextStorage>,
    Retained<NSLayoutManager>,
    Retained<NSTextContainer>,
) {
    let storage = NSTextStorage::new();
    storage.setAttributedString(text);
    let layout_manager = NSLayoutManager::new();
    storage.addLayoutManager(&layout_manager);
    #[cfg(target_os = "macos")]
    let container = NSTextContainer::initWithContainerSize(
        mtm.alloc::<NSTextContainer>(),
        CGSize::new(width, f64::MAX),
    );
    #[cfg(target_os = "ios")]
    let container =
        NSTextContainer::initWithSize(mtm.alloc::<NSTextContainer>(), CGSize::new(width, f64::MAX));
    container.setLineFragmentPadding(0.0);
    layout_manager.addTextContainer(&container);
    (storage, layout_manager, container)
}

/// The `stack` triple with layout forced — what `measure` runs through.
fn layout(
    mtm: MainThreadMarker,
    text: &NSAttributedString,
    width: f64,
) -> (Retained<NSTextStorage>, Retained<NSLayoutManager>) {
    let (storage, layout_manager, container) = stack(mtm, text, width);
    layout_manager.ensureLayoutForTextContainer(&container);
    (storage, layout_manager)
}

/// Sets `alignment` on every paragraph of `storage` — the equivalent of
/// what a label's `textAlignment`/`alignment` does to the attributed string
/// it draws — preserving the other attributes each range's paragraph style
/// carries.
///
/// Ranges without a paragraph style get a default one: a default
/// `NSMutableParagraphStyle` matches the typesetter's behavior for
/// attribute-free text, so only the alignment changes.
fn apply_paragraph_alignment(storage: &NSTextStorage, alignment: NSTextAlignment) {
    let mut location = 0;
    let length = storage.length();
    while location < length {
        let mut effective = NSRange::new(location, 0);
        // SAFETY: `effective` is stack storage the call writes.
        let existing = unsafe {
            storage.attribute_atIndex_effectiveRange(
                NSParagraphStyleAttributeName,
                location,
                &raw mut effective,
            )
        };
        let style = NSMutableParagraphStyle::new();
        if let Some(existing) = existing {
            style.setParagraphStyle(
                existing
                    .downcast::<NSParagraphStyle>()
                    .expect("the paragraph-style attribute carries an NSParagraphStyle")
                    .as_ref(),
            );
        }
        style.setAlignment(alignment);
        // SAFETY: `style` is an `NSParagraphStyle`, the documented type for
        // this attribute.
        unsafe {
            storage.addAttribute_value_range(
                NSParagraphStyleAttributeName,
                style.as_ref(),
                effective,
            );
        }
        location = effective.location + effective.length;
    }
}

/// A text stack kept alive for hit tests: the point-to-index and
/// range-to-rects queries the label's tap and accessibility spans run.
///
/// Geometry here is container coordinates — y measured down from the top of
/// the container — matching what `drawGlyphsForGlyphRange:` consumes; the
/// label translates between its own bounds and this space.
#[derive(Debug)]
pub struct Layout {
    /// Held, queried only for its length: the manager's `textStorage` is
    /// `assign`, so the storage must outlive every query.
    storage: Retained<NSTextStorage>,
    manager: Retained<NSLayoutManager>,
    container: Retained<NSTextContainer>,
}

impl Layout {
    /// Lays `text` out at `width` points with `alignment` applied to every
    /// paragraph — the same stack [`measure`] builds, plus the label's own
    /// alignment so centered and trailing text hit-tests where it is drawn.
    ///
    /// A nonzero `line_limit` stops layout after that many lines and
    /// truncates the last one's tail, as the label does when it draws: text
    /// the label elides is neither hit nor covered by any rectangle.
    #[must_use]
    pub fn new(
        mtm: MainThreadMarker,
        text: &NSAttributedString,
        width: f64,
        alignment: NSTextAlignment,
        line_limit: usize,
    ) -> Self {
        let (storage, manager, container) = stack(mtm, text, width);
        apply_paragraph_alignment(&storage, alignment);
        if line_limit > 0 {
            container.setMaximumNumberOfLines(line_limit);
            container.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        }
        manager.ensureLayoutForTextContainer(&container);
        Self {
            storage,
            manager,
            container,
        }
    }

    /// The UTF-16 character index `point` hits, in container coordinates —
    /// `None` when the point lands outside the text.
    #[must_use]
    pub fn character_index(&self, point: CGPoint) -> Option<usize> {
        // SAFETY: a null fraction pointer skips the out parameter.
        let index = unsafe {
            self.manager
                .characterIndexForPoint_inTextContainer_fractionOfDistanceBetweenInsertionPoints(
                    point,
                    &self.container,
                    std::ptr::null_mut(),
                )
        };
        if index == NSNotFound.cast_unsigned() || index >= self.storage.length() {
            return None;
        }
        // The index lookup also answers for points in the slack past a
        // line's end: only a point inside the character's own glyph bounds
        // counts as a hit — a tap off the text fails the recognizer's gate.
        let glyph = self.manager.glyphIndexForCharacterAtIndex(index);
        self.visible_glyphs(glyph)?;
        let bounds = self
            .manager
            .boundingRectForGlyphRange_inTextContainer(NSRange::new(glyph, 1), &self.container);
        NSPointInRect(point, bounds).then_some(index)
    }

    /// The rectangles `range` covers, in container coordinates — one per
    /// line fragment the range crosses, clipped to the fragment.
    ///
    /// `range` is in UTF-16 code units over the laid-out string and is
    /// clamped to its length.
    #[must_use]
    pub fn rects(&self, range: Range<usize>) -> Vec<CGRect> {
        let string_length = self.storage.length();
        let start = range.start.min(string_length);
        let range = NSRange::new(start, range.end.min(string_length) - start);
        if range.length == 0 {
            return Vec::new();
        }
        // SAFETY: a null actual-range pointer skips the out parameter.
        let glyphs = unsafe {
            self.manager
                .glyphRangeForCharacterRange_actualCharacterRange(range, std::ptr::null_mut())
        };
        let mut rects = Vec::new();
        let mut glyph = glyphs.location;
        while glyph < glyphs.location + glyphs.length {
            let mut line = NSRange::new(0, 0);
            // SAFETY: `line` is a valid out pointer for the fragment's glyph range.
            let line_rect = unsafe {
                self.manager
                    .lineFragmentRectForGlyphAtIndex_effectiveRange(glyph, &raw mut line)
            };
            // A glyph past the container's last line belongs to no fragment.
            if line.length == 0 {
                break;
            }
            if let Some(visible) = visible_glyphs(&self.manager, line) {
                let covered = NSIntersectionRange(glyphs, visible);
                if covered.length > 0 {
                    let bounds = self
                        .manager
                        .boundingRectForGlyphRange_inTextContainer(covered, &self.container);
                    rects.push(NSIntersectionRect(bounds, line_rect));
                }
            }
            glyph = line.location + line.length;
        }
        rects
    }

    /// The visible part of the line fragment holding `glyph`, or `None` when
    /// `glyph` itself is elided.
    fn visible_glyphs(&self, glyph: usize) -> Option<NSRange> {
        let mut line = NSRange::new(0, 0);
        // SAFETY: `line` is a valid out pointer for the fragment's glyph range.
        unsafe {
            self.manager
                .lineFragmentRectForGlyphAtIndex_effectiveRange(glyph, &raw mut line);
        }
        visible_glyphs(&self.manager, line).filter(|visible| {
            (visible.location..visible.location + visible.length).contains(&glyph)
        })
    }
}

/// The glyphs of the line fragment `line` that are drawn — all of them,
/// less the tail a line limit truncates — or `None` when none are.
fn visible_glyphs(manager: &NSLayoutManager, line: NSRange) -> Option<NSRange> {
    if line.length == 0 {
        return None;
    }
    let truncated = manager.truncatedGlyphRangeInLineFragmentForGlyphAtIndex(line.location);
    let end = if truncated.location == NSNotFound.cast_unsigned() {
        line.location + line.length
    } else {
        truncated.location
    };
    (end > line.location).then(|| NSRange::new(line.location, end - line.location))
}

/// One interactive region of a label's text — the span a tap or an
/// assistive activation can run.
pub struct SpanLink<'a> {
    /// The span's range over the label's plain text, in UTF-16 code units.
    pub range: Range<usize>,
    /// The text the region's accessibility element announces.
    pub text: &'a str,
    /// Runs when the region is activated.
    pub activate: Rc<dyn Fn()>,
}

impl std::fmt::Debug for SpanLink<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanLink")
            .field("range", &self.range)
            .field("text", &self.text)
            .finish_non_exhaustive()
    }
}

/// The glyph index at which each laid-out line begins: a glyph's effective
/// range starts where its line fragment starts.
fn line_starts(layout_manager: &NSLayoutManager, glyph_count: usize) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut effective_range = NSRange::new(0, 0);
    let mut glyph = 0;
    while glyph < glyph_count {
        // SAFETY: `effective_range` is stack storage the call writes into.
        unsafe {
            layout_manager
                .lineFragmentRectForGlyphAtIndex_effectiveRange(glyph, &raw mut effective_range);
        }
        if effective_range.length == 0 {
            break;
        }
        starts.push(effective_range.location);
        glyph = effective_range.location + effective_range.length;
    }
    starts
}

/// The distance from the top of `layout_manager`'s first line to the baseline
/// the glyph `line_start` opens.
fn baseline_of(layout_manager: &NSLayoutManager, line_start: usize) -> f64 {
    // SAFETY: a null effective-range pointer just skips the out parameter.
    let line_rect = unsafe {
        layout_manager
            .lineFragmentRectForGlyphAtIndex_effectiveRange(line_start, std::ptr::null_mut())
    };
    line_rect.origin.y + layout_manager.locationForGlyphAtIndex(line_start).y
}

/// Lays `attributed` out under `wrap` and reports its metrics, snapped up to
/// `scale` (device pixels per point; `1.0` at minimum).
///
/// `line_limit` of `0` measures the whole text; otherwise only the first
/// `line_limit` lines count toward the size, and the last baseline is the
/// last kept line's.
#[must_use]
pub fn measure(
    mtm: MainThreadMarker,
    attributed: &NSAttributedString,
    wrap: WrapWidth,
    line_limit: usize,
    scale: f64,
) -> TextMetrics {
    let scale = scale.max(1.0);
    let width = match wrap {
        WrapWidth::Free => f64::MAX,
        WrapWidth::Fixed(width) => width,
        WrapWidth::Unbreakable => widest_unbreakable_run(attributed, scale),
    };
    let (_storage, layout_manager) = layout(mtm, attributed, width);
    let glyph_count = layout_manager.numberOfGlyphs();
    if glyph_count == 0 {
        return TextMetrics {
            size: Size::ZERO,
            first_baseline: None,
            last_baseline: None,
        };
    }

    let starts = line_starts(&layout_manager, glyph_count);
    // A line limit caps the measurement at the last visible line: the
    // platform label truncates that line with an ellipsis, so the hidden
    // remainder must not reserve height.
    let (measured, last_baseline_glyph) = if line_limit > 0 && starts.len() > line_limit {
        let last_line_start = starts[line_limit - 1];
        let mut last_line_range = NSRange::new(last_line_start, 0);
        // SAFETY: `last_line_range` is stack storage the call writes into.
        unsafe {
            layout_manager.lineFragmentRectForGlyphAtIndex_effectiveRange(
                last_line_start,
                &raw mut last_line_range,
            );
        }
        // SAFETY: a null actual-range pointer skips the out parameter.
        let char_range = unsafe {
            layout_manager.characterRangeForGlyphRange_actualGlyphRange(
                NSRange::new(0, last_line_range.location + last_line_range.length),
                std::ptr::null_mut(),
            )
        };
        // `characterRange(forGlyphRange:)` can return a range reaching past
        // the string end for glyph ranges that cover trailing control
        // characters; `attributedSubstring(from:)` throws on overflow.
        let char_range = NSRange::new(
            char_range.location,
            char_range
                .length
                .min(attributed.length().saturating_sub(char_range.location)),
        );
        (
            attributed.attributedSubstringFromRange(char_range),
            last_line_range.location,
        )
    } else {
        (Retained::from(attributed), glyph_count - 1)
    };

    // `boundingRect` with `UsesLineFragmentOrigin` applies the platform's
    // default leading — identical to what NSTextField/UILabel report for the
    // same attributed string.
    let bounds = measured.boundingRectWithSize_options_context(
        CGSize::new(width, f64::MAX),
        NSStringDrawingOptions::UsesLineFragmentOrigin,
        None,
    );
    let ceil_to_scale = |v: f64| (v * scale).ceil() / scale;
    // The leaf's width includes two points of horizontal breathing room on
    // top of the bare text bounds, the inset a single-line NSTextField/UILabel
    // keeps inside its frame before the first drawn pixel.
    TextMetrics {
        size: Size::new(
            ceil_to_scale(bounds.size.width + 2.0),
            ceil_to_scale(bounds.size.height),
        ),
        first_baseline: Some(baseline_of(&layout_manager, 0)),
        last_baseline: Some(baseline_of(&layout_manager, last_baseline_glyph)),
    }
}

/// The width in points of the widest run of `attributed` that cannot be
/// broken across lines — the answer `WrapWidth::Unbreakable` measures.
///
/// Runs come from `CFStringTokenizer`'s line-break unit (UAX #14); each run
/// is clipped at its last non-whitespace character and measured at `scale`
/// resolution.
fn widest_unbreakable_run(attributed: &NSAttributedString, scale: f64) -> f64 {
    let text = attributed.string();
    let len = text.length();
    if len == 0 {
        return 0.0;
    }
    // SAFETY: the string is alive for the tokenizer's lifetime, which ends
    // with this function.
    let tokenizer = unsafe {
        CFStringTokenizer::new(
            None,
            Some(text.as_ref()),
            CFRange::new(0, len.cast_signed()),
            kCFStringTokenizerUnitLineBreak,
            None,
        )
    };
    let Some(tokenizer) = tokenizer else {
        // A string this long has no unbreakable-run answer worth pretending
        // at; saturate instead of miscounting.
        return f64::from(u32::try_from(len).unwrap_or(u32::MAX));
    };

    let non_whitespace = NSCharacterSet::whitespaceAndNewlineCharacterSet().invertedSet();
    let mut widest = 0.0_f64;
    loop {
        if tokenizer.advance_to_next_token() == CFStringTokenizerTokenType::None {
            break;
        }
        let token = tokenizer.current_token_range();
        // Clip the token at its last non-whitespace character.
        let range = NSRange::new(token.location.cast_unsigned(), token.length.cast_unsigned());
        let clipped = text.rangeOfCharacterFromSet_options_range(
            &non_whitespace,
            NSStringCompareOptions::BackwardsSearch,
            range,
        );
        if clipped.location == NSNotFound.cast_unsigned() {
            continue;
        }
        let range = NSRange::new(
            token.location.cast_unsigned(),
            clipped.location + clipped.length - token.location.cast_unsigned(),
        );
        let piece = attributed.attributedSubstringFromRange(range);
        let rect = piece.boundingRectWithSize_options_context(
            CGSize::new(f64::MAX, f64::MAX),
            NSStringDrawingOptions::UsesLineFragmentOrigin,
            None,
        );
        widest = widest.max((rect.size.width * scale).ceil() / scale);
    }
    widest
}

/// `text` with the Unicode bidirectional control characters removed, so a
/// label's accessibility value reads as the text renders.
///
/// Strips U+061C, U+202A…U+202E and U+2066…U+2069.
#[must_use]
pub fn strip_bidi_controls(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !matches!(
                *c,
                '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
            )
        })
        .collect()
}

/// `text`'s length in UTF-16 code units — the count `AppKit`/`UIKit`
/// selection and marked-text ranges use.
#[must_use]
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The byte offset `utf16_offset` code units into `text`, or `None` when the
/// offset is past the end or lands inside a surrogate pair — an offset the
/// platform never reports for valid content.
#[must_use]
pub fn byte_offset_at_utf16(text: &str, utf16_offset: usize) -> Option<usize> {
    let mut units = 0usize;
    let mut byte = 0usize;
    for ch in text.chars() {
        if units == utf16_offset {
            return Some(byte);
        }
        if units > utf16_offset {
            return None;
        }
        units += ch.len_utf16();
        byte += ch.len_utf8();
    }
    (units == utf16_offset).then_some(byte)
}

/// The UTF-16 code-unit offset `byte_offset` lands on, or `None` when it is
/// past the end or not a char boundary — an edge a plain-text range contract
/// rejects.
#[must_use]
pub fn utf16_offset_at_byte(text: &str, byte_offset: usize) -> Option<usize> {
    if byte_offset > text.len() || !text.is_char_boundary(byte_offset) {
        return None;
    }
    Some(text[..byte_offset].chars().map(char::len_utf16).sum())
}

#[cfg(test)]
mod tests {
    use super::{byte_offset_at_utf16, utf16_len, utf16_offset_at_byte};

    /// ASCII, CJK (UTF-16 BMP) and emoji (surrogate pairs) exercise every
    /// branch of the offset mapping.
    const TEXT: &str = "a中😀b";
    // UTF-8 bytes: a(1) 中(3) 😀(4) b(1) → 9 bytes; UTF-16: 1+1+2+1 = 5.

    #[test]
    fn utf16_len_counts_code_units() {
        assert_eq!(utf16_len(TEXT), 5);
        assert_eq!(utf16_len(""), 0);
    }

    #[test]
    fn utf16_offsets_map_to_byte_offsets() {
        // a|中|😀|b → units 0,1,2,4,5 ↔ bytes 0,1,4,8,9.
        for (unit, byte) in [(0, 0), (1, 1), (2, 4), (4, 8), (5, 9)] {
            assert_eq!(byte_offset_at_utf16(TEXT, unit), Some(byte), "unit {unit}");
        }
        // Mid-pair and past-the-end are not positions the contract accepts.
        assert_eq!(byte_offset_at_utf16(TEXT, 3), None);
        assert_eq!(byte_offset_at_utf16(TEXT, 6), None);
    }

    #[test]
    fn byte_offsets_map_to_utf16_offsets() {
        for (byte, unit) in [(0, 0), (1, 1), (4, 2), (8, 4), (9, 5)] {
            assert_eq!(utf16_offset_at_byte(TEXT, byte), Some(unit), "byte {byte}");
        }
        assert_eq!(utf16_offset_at_byte(TEXT, 2), None);
        assert_eq!(utf16_offset_at_byte(TEXT, 5), None);
        assert_eq!(utf16_offset_at_byte(TEXT, 10), None);
    }
}
