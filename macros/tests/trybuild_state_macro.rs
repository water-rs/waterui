//! Compile-fail coverage for the `#[state]` attribute's own diagnostics:
//! non-item input, unions, attribute arguments, and a missing `Clone`.

#[test]
fn state_macro_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/state/fail_on_fn.rs");
    cases.compile_fail("tests/ui/state/fail_union.rs");
    cases.compile_fail("tests/ui/state/fail_with_args.rs");
    cases.compile_fail("tests/ui/state/fail_missing_clone.rs");
}
