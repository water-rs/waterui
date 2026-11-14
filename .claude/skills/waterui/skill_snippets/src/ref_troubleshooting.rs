//! Snippets from `.claude/skills/waterui/references/troubleshooting.md`, in
//! file order. Transcription conventions are documented in the crate README.
//!
//! troubleshooting.md has exactly one rust block; the rest of the file is prose
//! and markdown tables, whose API names are proven by the `(prose)` functions
//! below (not counted among the file's rust blocks).

use waterui::prelude::*;

// ---------------------------------------------------------------------------
// troubleshooting.md § "## Silent bugs (compile fine, behave wrong)" — rust
// block 1/1
//
// Both lines are supposed to compile: the point of the section is that the
// broken one is a *silent* bug, not a compile error. Both do.
// ---------------------------------------------------------------------------
// `fade.snapshot()` is the doc's deliberate broken line — the section's point is
// that it compiles while silently freezing the view.
#[allow(unknown_lints, signal_get_in_view)]
pub fn troubleshooting_block_01() {
    let fade = Binding::f32(1.0);

    let view = Divider;
    let _ = {
        view.opacity(fade.snapshot()) // frozen
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
    let _ = Divider.visible(signal);
    let _ = Divider.anyview();
    let _ = AnyView::new(Divider);

    // `Binding::default()` for `Binding<Option<T>>`.
    let _: Binding<Option<i32>> = Binding::default();

    // `use waterui::reactive::SignalExt;` when the prelude is not glob-imported.
    let _ = Binding::i32(1).map(|v| v + 1);

    // The `Url` row: `Url::parse(s)` yields an `Option`, and
    // `Url::parse_user_input(s)` is the human-typed-address form.
    use waterui::media::Url;
    let _: Option<Url> = Url::parse("https://waterui.dev");
    let _: Option<Url> = Url::parse_user_input("waterui.dev");

    // `.str_is_empty()` / `.str_len()` / `.str_contains(..)`
    let s = Binding::container(Str::from("q"));
    let _ = (s.str_is_empty(), s.str_len(), s.str_contains("q"));
}

// ---------------------------------------------------------------------------
// troubleshooting.md § "## Compile errors" (prose, not counted among the file's
// rust blocks): the `E0525` / `E0507` row's fix, verbatim, with `c` and `fade`
// as glue.
// ---------------------------------------------------------------------------
pub fn troubleshooting_when_fn_builder() -> impl View {
    use waterui::widget::condition::when;

    let c = Binding::bool(true);
    let fade = Binding::f32(1.0);
    when(c, move || text("Saved").opacity(fade.clone()))
}

// ---------------------------------------------------------------------------
// troubleshooting.md § "## Compile errors" (prose): the `s!` and
// `Identifiable` import rows.
// ---------------------------------------------------------------------------
pub fn troubleshooting_imports() {
    use waterui::Identifiable;
    use waterui::reactive::s;

    struct Tagged(i32);
    impl Identifiable for Tagged {
        type Id = i32;
        fn id(&self) -> Self::Id {
            self.0
        }
    }

    let count = Binding::i32(3);
    let _ = s!("{count}");
    let _ = Tagged(1).id();
}

// ---------------------------------------------------------------------------
// troubleshooting.md § "## Runtime panics" (prose): the `Code` "Invalid
// language" entry — `Language::try_from(tok)`, `Language::Plaintext`, and
// `.info(tok)` keeping the original token.
// ---------------------------------------------------------------------------
pub fn troubleshooting_code_language(tok: &'static str) -> impl View {
    use waterui::text::highlight::Language;
    use waterui::widget::Code;

    let language = Language::try_from(tok).unwrap_or(Language::Plaintext);
    Code::new(language, "fn main() {}").info(tok)
}

// ---------------------------------------------------------------------------
// troubleshooting.md § "## Compile errors" (prose): the `shader!` import.
// ---------------------------------------------------------------------------
pub fn troubleshooting_shader_import() -> impl View {
    use waterui::graphics::shader;

    shader!("starfield.wgsl")
}

// ---------------------------------------------------------------------------
// troubleshooting.md § "Silent bugs" (prose): the resident-window and
// snackbar-outside-a-handler entries.
// ---------------------------------------------------------------------------
pub fn troubleshooting_resident_and_snackbar() -> impl View {
    use waterui::app::{App, LastWindowPolicy};
    use waterui::env::use_env;
    use waterui::snackbar::{Snackbar, SnackbarManager};

    let _ = App::new(|| Divider, Environment::new())
        .on_last_window_closed(LastWindowPolicy::StayResident);
    use_env(|manager: SnackbarManager| {
        let _callback = move || manager.show(Snackbar::new("Saved"));
        Divider
    })
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
