//! Integration tests for the `text!` macro.

#[test]
fn text_macro_contract() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/text/pass_named_placeholders.rs");
    cases.compile_fail("tests/ui/text/fail_empty_placeholder.rs");
    cases.compile_fail("tests/ui/text/fail_positional_placeholder.rs");
    cases.compile_fail("tests/ui/text/fail_positional_format_spec.rs");
    cases.compile_fail("tests/ui/text/fail_positional_plural_marker.rs");
    cases.compile_fail("tests/ui/text/fail_non_identifier_placeholder.rs");
}
