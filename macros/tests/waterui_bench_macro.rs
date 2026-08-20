//! Integration tests for the `#[waterui::bench]` attribute macro.

#[test]
fn waterui_bench_macro_contract() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/waterui_bench/pass_mounted.rs");
    cases.pass("tests/ui/waterui_bench/pass_budgets.rs");
    cases.pass("tests/ui/waterui_bench/pass_manual.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_param_count.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_mounted_return.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_manual_unit_return.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_unknown_argument.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_duplicate_budget.rs");
    cases.compile_fail("tests/ui/waterui_bench/fail_async.rs");
}
