//! A link inside a Markdown paragraph must stay part of the paragraph's flowing
//! text and keep the surrounding emphasis and heading style.
#![cfg(feature = "flow-markdown")]

use waterui::widget::{RichText, RichTextElement};

/// The single text element a block with only inline content collapses to.
fn only_text(rich: &RichText) -> &waterui::text::styled::StyledStr {
    match rich.elements() {
        [RichTextElement::Text(text)] => text,
        other => panic!("expected one flowing text element, got {other:?}"),
    }
}

#[test]
fn a_link_does_not_split_its_paragraph() {
    let rich = RichText::from_markdown(
        "Visit [docs](https://example.com) for a very long wrapping sentence.",
    );
    let text = only_text(&rich);
    assert_eq!(
        &*text.to_plain(),
        "Visit docs for a very long wrapping sentence."
    );
    let [span] = text.tap_spans() else {
        panic!("expected one link span, got {:?}", text.tap_spans());
    };
    assert_eq!(&text.to_plain()[span.range()], "docs");
}

#[test]
fn a_link_keeps_the_heading_font() {
    let rich = RichText::from_markdown("# Heading [docs](https://example.com) tail");
    let text = only_text(&rich);
    let fonts: Vec<String> = text
        .chunks()
        .iter()
        .map(|(_, style)| format!("{:?}", style.get_font()))
        .collect();
    assert!(fonts.len() >= 3, "{fonts:?}");
    assert!(fonts.iter().all(|font| *font == fonts[0]), "{fonts:?}");
}

#[test]
fn a_link_inside_emphasis_is_italic_and_emphasis_continues_after_it() {
    let rich = RichText::from_markdown("*see [docs](https://example.com) now*");
    let text = only_text(&rich);
    assert_eq!(&*text.to_plain(), "see docs now");
    assert!(
        text.chunks().iter().all(|(_, style)| style.get_italic()),
        "{:?}",
        text.chunks()
    );
}
