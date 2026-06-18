//! Integration tests for the `#[waterui::test]` attribute macro.

#[test]
fn waterui_test_macro_contract() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/waterui_test/pass_sync.rs");
    cases.pass("tests/ui/waterui_test/pass_async.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_missing_view_arg.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_param_count.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_param_not_mut_ref.rs");
    cases.compile_fail("tests/ui/waterui_test/fail_non_unit_return.rs");
}
