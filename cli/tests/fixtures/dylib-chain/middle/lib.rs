/// Marker the top image forwards; its DLL records the middle image's name.
pub extern "C" fn middle_marker() -> u8 {
    waterui_dylib_foundation::layer_marker()
}
