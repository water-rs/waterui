/// Marker the fixture binary calls so the linker keeps the dependency.
/// The reference into the middle layer forces the chain's `DT_NEEDED`.
pub extern "C" fn fixture_marker() -> u8 {
    waterui_dylib_middle::middle_marker()
}
