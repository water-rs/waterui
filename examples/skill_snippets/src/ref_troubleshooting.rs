//! Snippets from `.claude/skills/waterui/references/troubleshooting.md`, in
//! file order. Transcription conventions are documented in the crate README.
//!
//! troubleshooting.md has exactly one rust block; the rest of the file is prose
//! and markdown tables.

use waterui::prelude::*;

// ---------------------------------------------------------------------------
// troubleshooting.md § "## Silent bugs (compile fine, behave wrong)" — rust
// block 1/1
//
// Both lines are supposed to compile: the point of the section is that the
// broken one is a *silent* bug, not a compile error. Both do.
// ---------------------------------------------------------------------------
pub fn troubleshooting_block_01() {
    let fade = Binding::f32(1.0);

    let view = Divider;
    let _ = {
        view.opacity(fade.get()) // frozen
    };
    let view = Divider;
    let _ = {
        view.opacity(fade.clone()) // reactive
    };
}

// ---------------------------------------------------------------------------
// troubleshooting.md § "## Compile errors" (a markdown table, not a fenced
// block). Every "Fix" column entry that names an API is proven somewhere in
// this crate; the ones with no other home are proven here.
// ---------------------------------------------------------------------------
pub fn troubleshooting_table_fixes() {
    use waterui::reactive::SignalExt as _;

    let signal = Binding::bool(true);

    // `when(..)` / `.visible(signal)` / `.anyview()` / `AnyView::new(..)`
    let _ = Divider.visible(signal.clone());
    let _ = Divider.anyview();
    let _ = AnyView::new(Divider);

    // `.select(1.0 as f32, 0.3)` — the suffixed-literal inference fix.
    let _ = signal.select(1.0_f32, 0.3);

    // `Binding::default()` for `Binding<Option<T>>`.
    let _: Binding<Option<i32>> = Binding::default();

    // `use waterui::reactive::SignalExt;` when the prelude is not glob-imported.
    let _ = Binding::i32(1).map(|v| v + 1);

    // The `Url` row: `Url::parse(s)` yields an `Option`, and
    // `Url::parse_user_input(s)` is the human-typed-address form.
    let _: Option<waterui::media::Url> = waterui::media::Url::parse("https://waterui.dev");
    let _: Option<waterui::media::Url> = waterui::media::Url::parse_user_input("waterui.dev");

    // `.str_is_empty()` / `.str_len()` / `.str_contains(..)`
    let s = Binding::container(Str::from("q"));
    let _ = (s.str_is_empty(), s.str_len(), s.str_contains("q"));
}

// ---------------------------------------------------------------------------
// troubleshooting.md § "## Runtime panics" (prose): `Id::try_from(0)` fails
// because `Id` is non-zero. A runtime property, not a compile one.
// ---------------------------------------------------------------------------
pub fn troubleshooting_id_non_zero() {
    use waterui::id::Id;

    assert!(Id::try_from(0_i32).is_err());
    assert!(Id::try_from(1_i32).is_ok());
}
