//! Compile-fail coverage for `include_web!`'s expansion-time checks.

#[test]
fn include_web_rejects_bad_invocations() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/include_web/fail_missing_root.rs");
    cases.compile_fail("tests/ui/include_web/fail_no_package_json.rs");
    cases.compile_fail("tests/ui/include_web/fail_unknown_option.rs");
    cases.compile_fail("tests/ui/include_web/fail_wrong_option_type.rs");
}
