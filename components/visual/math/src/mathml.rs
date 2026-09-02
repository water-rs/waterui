//! Publishing the semantic tree as `MathML`.
//!
//! A formula drawn through [`crate::scene`] reaches the screen as filled paths
//! and glyph runs. To a screen reader that is a picture with no content, which
//! is why the tree is kept after layout rather than consumed by it: `MathML` is
//! what assistive technology actually reads, and this is where it comes from.
//!
//! The markup is written through an XML writer rather than assembled from
//! strings, so tags balance and content is escaped by construction — a formula
//! containing `<` or `&` is ordinary, not an edge case.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

use crate::ast::{MathItem, MathStyle};

/// Why a tree could not be written as `MathML`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MathMlError {
    /// The writer failed, or produced bytes that are not UTF-8.
    #[error("could not write MathML: {message}")]
    Write {
        /// What went wrong.
        message: String,
    },
}

/// Renders `item` as a `MathML` `<math>` element.
///
/// `style` sets the `display` attribute, which is the difference between a
/// formula announced as set on its own line and one announced inline.
///
/// # Errors
///
/// Returns [`MathMlError`] if the underlying writer fails.
pub fn to_mathml(item: &MathItem, style: MathStyle) -> Result<String, MathMlError> {
    let mut writer = Writer::new(Vec::new());

    let mut root = BytesStart::new("math");
    root.push_attribute(("xmlns", "http://www.w3.org/1998/Math/MathML"));
    root.push_attribute((
        "display",
        if style.is_display() {
            "block"
        } else {
            "inline"
        },
    ));
    write(&mut writer, Event::Start(root))?;
    element(&mut writer, item)?;
    write(&mut writer, Event::End(BytesEnd::new("math")))?;

    String::from_utf8(writer.into_inner()).map_err(|error| MathMlError::Write {
        message: error.to_string(),
    })
}

fn write(writer: &mut Writer<Vec<u8>>, event: Event<'_>) -> Result<(), MathMlError> {
    writer
        .write_event(event)
        .map_err(|error| MathMlError::Write {
            message: error.to_string(),
        })
}

fn leaf(writer: &mut Writer<Vec<u8>>, tag: &str, text: &str) -> Result<(), MathMlError> {
    write(writer, Event::Start(BytesStart::new(tag)))?;
    write(writer, Event::Text(BytesText::new(text)))?;
    write(writer, Event::End(BytesEnd::new(tag)))
}

fn wrap(
    writer: &mut Writer<Vec<u8>>,
    tag: &str,
    children: &[&MathItem],
) -> Result<(), MathMlError> {
    write(writer, Event::Start(BytesStart::new(tag)))?;
    for child in children {
        element(writer, child)?;
    }
    write(writer, Event::End(BytesEnd::new(tag)))
}

fn element(writer: &mut Writer<Vec<u8>>, item: &MathItem) -> Result<(), MathMlError> {
    match item {
        MathItem::Ident(text) => leaf(writer, "mi", text.as_str()),
        MathItem::Number(text) => leaf(writer, "mn", text.as_str()),
        MathItem::Text(text) => leaf(writer, "mtext", text.as_str()),
        MathItem::Operator(operator) => leaf(writer, "mo", operator.glyph.as_str()),
        MathItem::Space(em) => {
            let mut space = BytesStart::new("mspace");
            let width = alloc::format!("{em}em");
            space.push_attribute(("width", width.as_str()));
            write(writer, Event::Empty(space))
        }
        MathItem::Row(items) => {
            let children: Vec<&MathItem> = items.iter().collect();
            wrap(writer, "mrow", &children)
        }
        MathItem::Fraction {
            numerator,
            denominator,
        } => wrap(writer, "mfrac", &[numerator, denominator]),
        MathItem::Radical { radicand, degree } => match degree {
            None => wrap(writer, "msqrt", &[radicand]),
            Some(degree) => wrap(writer, "mroot", &[radicand, degree]),
        },
        MathItem::Scripts { base, sub, sup } => match (sub, sup) {
            (Some(sub), Some(sup)) => wrap(writer, "msubsup", &[base, sub, sup]),
            (Some(sub), None) => wrap(writer, "msub", &[base, sub]),
            (None, Some(sup)) => wrap(writer, "msup", &[base, sup]),
            (None, None) => element(writer, base),
        },
        MathItem::Fenced { open, body, close } => {
            write(writer, Event::Start(BytesStart::new("mrow")))?;
            if let Some(open) = open {
                leaf(writer, "mo", open.glyph.as_str())?;
            }
            element(writer, body)?;
            if let Some(close) = close {
                leaf(writer, "mo", close.glyph.as_str())?;
            }
            write(writer, Event::End(BytesEnd::new("mrow")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::latex;

    fn mathml(source: &str) -> String {
        let item = latex::parse(source).unwrap_or_else(|error| panic!("`{source}`: {error}"));
        to_mathml(&item, MathStyle::Text).expect("MathML must be writable")
    }

    #[test]
    fn a_fraction_is_an_mfrac() {
        let markup = mathml(r"\frac{a}{b}");
        assert!(markup.contains("<mfrac>"), "got {markup}");
        assert!(markup.contains("</mfrac>"), "got {markup}");
    }

    #[test]
    fn a_square_root_is_an_msqrt_and_a_cube_root_an_mroot() {
        assert!(mathml(r"\sqrt{x}").contains("<msqrt>"));
        assert!(mathml(r"\sqrt[3]{x}").contains("<mroot>"));
    }

    #[test]
    fn scripts_choose_the_matching_element() {
        assert!(mathml("x^2").contains("<msup>"));
        assert!(mathml("x_i").contains("<msub>"));
        assert!(mathml("x_i^2").contains("<msubsup>"));
    }

    #[test]
    fn the_root_declares_the_mathml_namespace() {
        let markup = mathml("x");
        assert!(
            markup.contains("http://www.w3.org/1998/Math/MathML"),
            "assistive technology keys off the namespace: {markup}"
        );
    }

    #[test]
    fn display_mode_reaches_the_markup() {
        let item = latex::parse(r"\frac{a}{b}").expect("parses");
        let block = to_mathml(&item, MathStyle::Display).expect("writes");
        let inline = to_mathml(&item, MathStyle::Text).expect("writes");
        assert!(block.contains("display=\"block\""), "got {block}");
        assert!(inline.contains("display=\"inline\""), "got {inline}");
    }

    /// A relation like `<` must not corrupt the markup. This is exactly what
    /// hand-assembled XML gets wrong.
    #[test]
    fn markup_special_characters_are_escaped() {
        let markup = mathml("a<b");
        assert!(
            !markup.contains("<mo><</mo>"),
            "a raw `<` would make the document unparseable: {markup}"
        );
        assert!(markup.contains("&lt;"), "expected an escaped `<`: {markup}");
    }
}
