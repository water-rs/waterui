//! `DefaultHighlighter` hands syntect each line with its ending, so scopes that
//! end at the newline do not leak into the next line.

#![cfg(feature = "highlight")]

use waterui_color::ColorScheme;
use waterui_text::highlight::{DefaultHighlighter, Highlighter, Language};

fn colours(code: &str, language: Language) -> Vec<(String, String)> {
    DefaultHighlighter::new(ColorScheme::Dark)
        .highlight(language, code)
        .into_iter()
        .map(|c| (c.text().to_string(), format!("{:?}", c.color())))
        .collect()
}

#[test]
fn rust_line_comment_does_not_leak_into_next_line() {
    let alone = colours("fn main() {}", Language::Rust);
    let after_comment = colours("// note\nfn main() {}", Language::Rust);
    let tail = &after_comment[after_comment.len() - alone.len()..];
    assert_eq!(
        alone, tail,
        "second line highlights differently after a comment line: {after_comment:#?}"
    );
}

#[test]
fn python_comment_does_not_leak_into_next_line() {
    let alone = colours("def f():\n    return 1", Language::Python);
    let with = colours("# note\ndef f():\n    return 1", Language::Python);
    let tail = &with[with.len().saturating_sub(alone.len())..];
    assert_eq!(
        alone, tail,
        "code after a `#` comment line is coloured as comment: {with:#?}"
    );
}

#[test]
fn chunks_reproduce_the_input() {
    let code = "# note\ndef f():\n    return 1\n";
    let joined: String = colours(code, Language::Python)
        .into_iter()
        .map(|(text, _)| text)
        .collect();
    assert_eq!(joined, code);
}
