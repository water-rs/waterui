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

use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, rc::Retained};
use objc2_core_foundation::{
    CFRange, CFStringTokenizer, CFStringTokenizerTokenType, CGSize, kCFStringTokenizerUnitLineBreak,
};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSCharacterSet, NSMutableAttributedString,
    NSNotFound, NSNumber, NSRange, NSString, NSStringCompareOptions,
};

use crate::font::Font;
use crate::geometry::Size;

#[cfg(target_os = "macos")]
mod platform {
    pub use objc2_app_kit::{
        NSAttributedStringNSExtendedStringDrawing, NSBackgroundColorAttributeName,
        NSColor as Color, NSFontAttributeName, NSForegroundColorAttributeName, NSKernAttributeName,
        NSLayoutManager, NSParagraphStyleAttributeName, NSStrikethroughStyleAttributeName,
        NSTextContainer, NSTextStorage, NSUnderlineStyleAttributeName,
    };
    pub use objc2_app_kit::{NSMutableParagraphStyle, NSStringDrawingOptions};
}
#[cfg(target_os = "ios")]
mod platform {
    pub use objc2_ui_kit::{
        NSAttributedStringNSExtendedStringDrawing, NSBackgroundColorAttributeName,
        NSFontAttributeName, NSForegroundColorAttributeName, NSKernAttributeName, NSLayoutManager,
        NSParagraphStyleAttributeName, NSStrikethroughStyleAttributeName, NSTextContainer,
        NSTextStorage, NSUnderlineStyleAttributeName, UIColor as Color,
    };
    pub use objc2_ui_kit::{NSLineBreakStrategy, NSMutableParagraphStyle, NSStringDrawingOptions};
}
#[cfg(target_os = "ios")]
use platform::NSLineBreakStrategy;
use platform::{
    Color, NSAttributedStringNSExtendedStringDrawing, NSBackgroundColorAttributeName,
    NSFontAttributeName, NSForegroundColorAttributeName, NSKernAttributeName, NSLayoutManager,
    NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSStrikethroughStyleAttributeName,
    NSStringDrawingOptions, NSTextContainer, NSTextStorage, NSUnderlineStyleAttributeName,
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
/// measurement runs through, laid out at `width` points.
///
/// `NSLayoutManager`'s `textStorage` back-reference is `assign` — it does not
/// retain the storage — so the storage is returned alongside the manager and
/// callers must keep it alive for as long as they query the layout.
fn layout(
    mtm: MainThreadMarker,
    text: &NSAttributedString,
    width: f64,
) -> (Retained<NSTextStorage>, Retained<NSLayoutManager>) {
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
    layout_manager.ensureLayoutForTextContainer(&container);
    (storage, layout_manager)
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
