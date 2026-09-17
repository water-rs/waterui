//! Compile-fail coverage for the `#[diagnostic::on_unimplemented]` wording on
//! `WaterUI`'s first-contact traits: `View`, `Views`, `Handler`, `Extractor`,
//! `Identifiable`, `IntoText`, and `IntoLabel`.
//!
//! The cases exercise the exact trait-bound errors an author meets first, so
//! the emitted `.stderr` files pin the WaterUI-specific message, label, and
//! note each trait reports instead of rustc's bare "trait is not implemented".

#[test]
fn first_contact_trait_diagnostics() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/first_contact/view.rs");
    cases.compile_fail("tests/ui/first_contact/views.rs");
    cases.compile_fail("tests/ui/first_contact/handler.rs");
    cases.compile_fail("tests/ui/first_contact/extractor.rs");
    cases.compile_fail("tests/ui/first_contact/identifiable.rs");
    cases.compile_fail("tests/ui/first_contact/into_text.rs");
    cases.compile_fail("tests/ui/first_contact/into_label.rs");
}
