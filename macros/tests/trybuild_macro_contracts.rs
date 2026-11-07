//! Compile-time contract tests for the `WaterUI` macros and the
//! `#[diagnostic::on_unimplemented]` wording on the first-contact traits.
//!
//! Every case lives in one `TestCases`, run by one test. trybuild builds every
//! `TestCases` of this crate in the same fixture project under
//! `target/tests/trybuild/waterui-macros`, holds that project's lock for the
//! whole run, and compiles the fixture's dependency graph separately for each
//! build mode: `cargo check` when a set has only `compile_fail` cases,
//! `cargo build` when it has a `pass` case. Split across test binaries, the
//! sets queued behind one another on that lock — each waiting for the full run
//! of every set ahead of it — and the graph was compiled once per mode. One set
//! compiles it in one mode and runs without a queue.
//!
//! - `first_contact`: the trait-bound errors an author meets first on `View`,
//!   `Views`, `Handler`, `Extractor`, `Identifiable`, `IntoText` and
//!   `IntoLabel`; the `.stderr` files pin the WaterUI-specific message, label
//!   and note each trait reports instead of rustc's bare "trait is not
//!   implemented".
//! - `state`: the `#[state]` attribute's own diagnostics — non-item input,
//!   unions, attribute arguments and a missing `Clone`.
//! - `text`, `view_builder`, `waterui_test`, `waterui_bench`: the `text!`
//!   macro, the `view!` builder, and the `#[waterui::test]` and
//!   `#[waterui::bench]` attributes.

#[test]
fn macro_contracts() {
    let cases = trybuild::TestCases::new();

    cases.compile_fail("tests/ui/first_contact/view.rs");
    cases.compile_fail("tests/ui/first_contact/views.rs");
    cases.compile_fail("tests/ui/first_contact/handler.rs");
    cases.compile_fail("tests/ui/first_contact/extractor.rs");
    cases.compile_fail("tests/ui/first_contact/identifiable.rs");
    cases.compile_fail("tests/ui/first_contact/into_text.rs");
    cases.compile_fail("tests/ui/first_contact/into_label.rs");

    cases.compile_fail("tests/ui/state/fail_on_fn.rs");
    cases.compile_fail("tests/ui/state/fail_union.rs");
    cases.compile_fail("tests/ui/state/fail_with_args.rs");
    cases.compile_fail("tests/ui/state/fail_missing_clone.rs");

    cases.pass("tests/ui/text/pass_named_placeholders.rs");
    cases.compile_fail("tests/ui/text/fail_empty_placeholder.rs");
    cases.compile_fail("tests/ui/text/fail_positional_placeholder.rs");
    cases.compile_fail("tests/ui/text/fail_positional_format_spec.rs");
    cases.compile_fail("tests/ui/text/fail_positional_plural_marker.rs");
    cases.compile_fail("tests/ui/text/fail_non_identifier_placeholder.rs");

    cases.pass("tests/ui/view_builder/pass_fn_if_let.rs");
    cases.pass("tests/ui/view_builder/pass_impl_view_body.rs");
    cases.pass("tests/ui/view_builder/pass_view_macro_closure.rs");
    cases.pass("tests/ui/view_builder/pass_semicolon_drop.rs");
    cases.compile_fail("tests/ui/view_builder/fail_async_fn.rs");
    cases.compile_fail("tests/ui/view_builder/fail_impl_attr.rs");
    cases.compile_fail("tests/ui/view_builder/fail_non_function.rs");
    cases.compile_fail("tests/ui/view_builder/fail_non_impl_trait_return.rs");

    cases.pass("tests/ui/waterui_test/pass_sync.rs");
    cases.pass("tests/ui/waterui_test/pass_async.rs");
    cases.pass("tests/ui/waterui_test/pass_manual_mount.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_manual_mount_ref_param.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_offscreen_without_view.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_offscreen_without_theme.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_param_count.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_param_not_mut_ref.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_non_unit_return.rs");

    cases.compile_fail("tests/ui/waterui_bench/fail_bench_without_theme.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_param_count.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_mounted_return.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_manual_unit_return.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_unknown_argument.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_duplicate_budget.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_async.rs");
}
