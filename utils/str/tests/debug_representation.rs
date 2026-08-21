//! `Debug` prints the string, for both the static and the owned representation.

#[test]
fn debug_prints_the_string_for_both_representations() {
    let owned = waterui_str::Str::from(String::from("owned value"));
    let borrowed = waterui_str::Str::from_static("static value");
    assert_eq!(format!("{owned:?}"), "\"owned value\"");
    assert_eq!(format!("{borrowed:?}"), "\"static value\"");
    assert_eq!(format!("{:?}", Some(owned)), "Some(\"owned value\")");
}
