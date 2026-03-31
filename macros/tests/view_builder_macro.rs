#[test]
fn view_builder_macro_contract() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/view_builder/pass_fn_if_let.rs");
    cases.pass("tests/ui/view_builder/pass_impl_view_body.rs");
    cases.pass("tests/ui/view_builder/pass_view_macro_closure.rs");
    cases.pass("tests/ui/view_builder/pass_semicolon_drop.rs");
    cases.compile_fail("tests/ui/view_builder/fail_async_fn.rs");
    cases.compile_fail("tests/ui/view_builder/fail_impl_attr.rs");
    cases.compile_fail("tests/ui/view_builder/fail_non_function.rs");
    cases.compile_fail("tests/ui/view_builder/fail_non_impl_trait_return.rs");
}
