//! `install` claiming a fence inside a real Markdown document.
//!
//! The unit of work here is the whole path: `pulldown-cmark` keeps the info
//! token, `Code` offers it to the hook, and the hook turns it into a diagram —
//! while every other fence in the same document is declined and comes back as
//! an ordinary code block.

use hydrolysis_m3::install as install_m3;
use waterui::env::use_env;
use waterui::metadata::Metadata;
use waterui::prelude::*;
use waterui::widget::RichText;
use waterui::widget::code::CodeConfig;
use waterui_testing::{OffscreenApp, Role, ui as test_ui};

const DOC: &str = "\
# Pipeline

```mermaid
flowchart LR
    Parse --> Layout
    Layout --> Draw
```

```rust
fn main() {}
```
";

fn app() -> OffscreenApp {
    test_ui()
        .viewport(900, 900)
        .theme(install_m3)
        .mount_offscreen(|| {
            use_env(|mut env: Environment| {
                waterui_mermaid::install(&mut env);
                Metadata::new(RichText::from_markdown(DOC), env)
            })
        })
}

#[core::prelude::v1::test]
fn a_mermaid_fence_becomes_a_diagram() {
    let mut app = app();
    for label in ["Parse", "Layout", "Draw"] {
        app.query().role(Role::LABEL).label(label).assert_exists();
    }
}

/// The hook declines every other token, so the `rust` fence keeps the code
/// block's header. Without this the test above would still pass with a hook
/// that swallowed the whole document.
#[core::prelude::v1::test]
fn every_other_fence_is_still_a_code_block() {
    let mut app = app();
    app.query().role(Role::LABEL).label("Rust").assert_exists();
}

/// A hook already on the environment is the platform's, and wins.
#[core::prelude::v1::test]
fn install_yields_to_a_hook_that_is_already_there() {
    let mut app = test_ui()
        .viewport(600, 400)
        .theme(install_m3)
        .mount_offscreen(|| {
            use_env(|mut env: Environment| {
                env.insert_hook::<CodeConfig, AnyView>(|_env, _config| {
                    AnyView::new(text("bridged"))
                });
                waterui_mermaid::install(&mut env);
                Metadata::new(RichText::from_markdown(DOC), env)
            })
        });

    app.query()
        .role(Role::LABEL)
        .label("bridged")
        .assert_exists();
    assert!(
        app.query()
            .role(Role::LABEL)
            .label("Parse")
            .all()
            .is_empty(),
        "the hook already installed must keep the fence, not hand it to Mermaid"
    );
}
