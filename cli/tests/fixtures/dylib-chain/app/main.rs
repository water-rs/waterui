fn main() {
    // A referenced symbol keeps the `DT_NEEDED` the link would
    // otherwise drop under `--as-needed`.
    assert_eq!(waterui_dylib::fixture_marker(), 42);
}
