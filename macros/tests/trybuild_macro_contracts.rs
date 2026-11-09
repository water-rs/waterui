//! Compile-time rejection tests for the `WaterUI` macros and the
//! `#[diagnostic::on_unimplemented]` wording on the first-contact traits.
//!
//! Every case lives in one `TestCases`, run by one test, and every case is
//! `compile_fail`. trybuild builds every `TestCases` of this crate in the same
//! fixture project under `target/tests/trybuild/waterui-macros` and holds that
//! project's lock for the whole run, so split sets queue behind one another.
//! A set with only `compile_fail` cases is checked with `cargo check`, all of
//! its cases in one `--keep-going` invocation; a single `pass` case switches
//! the whole set to a `cargo build` of the framework's dependency graph and
//! one `cargo clean` plus `cargo build` per case, run one after another. The
//! expansions that must compile are therefore ordinary integration tests in
//! `macro_expansion.rs`, compiled by the test build itself.
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

    cases.compile_fail("tests/ui/text/fail_empty_placeholder.rs");
    cases.compile_fail("tests/ui/text/fail_positional_placeholder.rs");
    cases.compile_fail("tests/ui/text/fail_positional_format_spec.rs");
    cases.compile_fail("tests/ui/text/fail_positional_plural_marker.rs");
    cases.compile_fail("tests/ui/text/fail_non_identifier_placeholder.rs");

    cases.compile_fail("tests/ui/view_builder/fail_async_fn.rs");
    cases.compile_fail("tests/ui/view_builder/fail_impl_attr.rs");
    cases.compile_fail("tests/ui/view_builder/fail_non_function.rs");
    cases.compile_fail("tests/ui/view_builder/fail_non_impl_trait_return.rs");

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
