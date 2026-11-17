//! Who gets to decide what a fenced code block looks like.
//!
//! A fence carries an info token the author wrote — `rust`, `mermaid`,
//! `puzzle` — and only one of those resolves to a `Language`. Before
//! `CodeConfig`, the other two were indistinguishable from a fence with no info
//! string at all, so nothing could claim them. These assertions are about the
//! token surviving far enough for a hook to dispatch on it, and about the
//! default rendering staying exactly what it was for every fence a hook
//! declines.
#![cfg(feature = "flow-markdown")]

use waterui::ViewExt as _;
use waterui::prelude::*;
use waterui::view::hook::Hook;
use waterui::widget::code::CodeConfig;
use waterui::widget::{RichText, flow_markdown};
use waterui_testing::UiBuilder;

/// One fence a `Language` answers to and one it does not.
const DOC: &str = "\
```rust
fn main() {}
```

```puzzle
this is not a programming language
```
";

/// The `Code` widget names its language in its header, which is the assertion
/// `FlowMarkdown`'s old parallel path could never have passed: it captioned
/// every block \"Code\" and put the language nowhere.
#[waterui::test(viewport = (700, 900))]
fn an_unclaimed_fence_names_its_language(ui: UiBuilder) {
    let mut app = ui.mount(|| RichText::from_markdown(DOC));

    app.query().label("Rust").assert_exists();
    app.query().label("Copy").assert_exists();
}

/// Claims the `puzzle` fences and declines every other.
#[derive(Clone)]
struct Puzzles;

impl Hook for Puzzles {
    type Target = CodeConfig;

    fn apply(&self, config: CodeConfig, _env: &Environment) -> Result<impl View, CodeConfig> {
        match config.info().map(core::ops::Deref::deref) {
            Some("puzzle") => Ok(text("solved").a11y_label("claimed")),
            _ => Err(config),
        }
    }
}

/// A hook sees the token as written and claims the one it recognises. The
/// `rust` fence in the same document is offered to the same hook, is declined,
/// and comes back as the ordinary code block.
#[waterui::test(viewport = (700, 900))]
fn a_hook_claims_only_the_token_it_recognises(ui: UiBuilder) {
    let mut app = ui.mount(|| RichText::from_markdown(DOC).hooks(Puzzles));

    app.query().label("claimed").assert_exists();
    app.query().label("Rust").assert_exists();
}

/// A streamed document renders its fences through the same `Code` widget, so
/// the hook covers it too and the language reaches the accessibility tree.
#[waterui::test(viewport = (700, 900))]
fn a_fence_in_flow_markdown_names_its_language(ui: UiBuilder) {
    let mut app = ui.mount(|| flow_markdown(DOC));

    app.query().label("Rust").assert_exists();
}
