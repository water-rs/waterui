//! Edge Text Example - extreme text cases not covered by other examples.
//!
//! Unbreakable strings, combining marks, ZWJ emoji sequences, mixed-script
//! lines, empty/whitespace text, and size extremes — the cases that break
//! text measurement and line breaking.

use waterui::app::App;
use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::preview;

fn section(title: &'static str, content: impl View) -> impl View {
    vstack((
        text(title).sub_headline().foreground(MutedForeground),
        content,
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(6.0)
    .padding_with(EdgeInsets::all(12.0))
}

/// 160-char run with no break opportunities — tests overflow wrapping.
fn unbroken() -> impl View {
    section(
        "Unbroken 160-char string",
        text("abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789"),
    )
}

/// Combining marks must cluster onto their base glyph, not wrap separately.
fn combining() -> impl View {
    section(
        "Combining marks",
        text("cafe\u{0301} nai\u{0308}ve a\u{0328} o\u{0323}\u{0302} Z\u{0350}"),
    )
}

/// ZWJ emoji are one grapheme cluster; splitting them is a layout bug.
fn emoji() -> impl View {
    section(
        "Emoji sequences",
        text("👨\u{200D}👩\u{200D}👧\u{200D}👦 🏳\u{FE0F}\u{200D}🌈 👍🏽 🇺🇳 1\u{FE0F}\u{20E3}"),
    )
}

/// Latin, CJK, Arabic and Hebrew in a single line — bidi ordering per run.
fn mixed_scripts() -> impl View {
    section(
        "Mixed scripts",
        text("Latin 中文 العربية עברית 日本語 한국어 123"),
    )
}

/// Empty and whitespace-only text must not collapse the stack or crash.
fn empties() -> impl View {
    section(
        "Empty and whitespace",
        vstack((
            hstack((text("[before]"), text(""), text("[after]"))),
            hstack((text("[before]"), text("   "), text("[after]"))),
        ))
        .spacing(4.0),
    )
}

/// 9pt and 48pt side by side — baseline alignment across extreme sizes.
fn size_extremes() -> impl View {
    section(
        "Size extremes",
        hstack((
            text("tiny 9").size(9.0),
            text("huge 48").size(48.0),
            text("body").body(),
        ))
        .spacing(8.0)
        .alignment(VerticalAlignment::LastBaseline),
    )
}

/// A paragraph long enough to force multiple wraps at 800pt window width.
fn long_paragraph() -> impl View {
    section(
        "Wrapping paragraph",
        text("The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. How vexingly quick daft zebras jump! Sphinx of black quartz, judge my vow. 敏捷的棕色狐狸跳过懒狗。素早い茶色のキツネが怠けた犬を飛び越える。"),
    )
}

#[preview]
pub fn demo() -> impl View {
    scroll(
        vstack((
            text("Edge Text").title(),
            text("Extreme text measurement and line breaking")
                .sub_headline()
                .foreground(MutedForeground),
            Divider,
            vstack((
                unbroken(),
                Divider,
                combining(),
                Divider,
                emoji(),
                Divider,
                mixed_scripts(),
            ))
            .alignment(HorizontalAlignment::Leading),
            vstack((
                Divider,
                empties(),
                Divider,
                size_extremes(),
                Divider,
                long_paragraph(),
                spacer().min_height(16.0),
            ))
            .alignment(HorizontalAlignment::Leading),
        ))
        .alignment(HorizontalAlignment::Leading)
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
