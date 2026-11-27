//! Integration tests for `Code` backend delegation (`#5186`) and `Body` font
//! scaling (`#4379`) on Hydrolysis.
#![cfg(feature = "highlight")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use waterui::ViewExt as _;
use waterui::prelude::*;
use waterui::text::font::{FontWeight, ResolvedFont};
use waterui::theme::{FontSettings, Theme};
use waterui::widget::code::code;
use waterui_testing::{Role, TestStyle, UiBuilder};

const SAMPLE: &str = "fn main() {\n    let value = 42;\n    let doubled = value * 2;\n}";

fn mount_code_at_body_size(ui: UiBuilder, body_size: f32) -> f32 {
    let mut app = ui.theme(TestStyle::new()).mount_offscreen(move || {
        let theme = Theme::new()
            .fonts(FontSettings::new().body(ResolvedFont::new(body_size, FontWeight::Normal)));
        vstack((code("rust", SAMPLE), spacer())).install(theme)
    });

    let body_node = app
        .query()
        .role(Role::LABEL)
        .label_contains("fn main()")
        .single();
    body_node.bounds().height()
}

#[waterui::test(viewport = (640, 480))]
fn code_body_scales_with_installed_body_font_slot(ui: UiBuilder) {
    let compact_height = mount_code_at_body_size(ui.clone(), 14.0);
    let large_height = mount_code_at_body_size(ui, 28.0);

    assert!(
        large_height > compact_height * 1.4,
        "code block body must scale with the installed Body font slot: compact={compact_height}, large={large_height}"
    );
}

#[waterui::test(viewport = (640, 480))]
fn code_exposes_header_and_accessible_copy_button_invoking_on_copied(ui: UiBuilder) {
    let copied_count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&copied_count);

    let mut app = ui.mount(move || {
        let counter = Arc::clone(&counter);
        code("rust", "let x = 1;").on_copied(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    });

    app.query().label("Rust").assert_exists();
    let copy = app.query().role(Role::BUTTON).label("Copy").single();
    copy.tap(&mut app);
    assert_eq!(
        copied_count.load(Ordering::SeqCst),
        1,
        "tapping Copy must invoke OnCopied"
    );
}
