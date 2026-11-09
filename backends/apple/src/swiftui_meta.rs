//! The compiled `WaterUIBridge` Swift module's path as a canonical
//! `__wmeta` metadata record — the packaging pipeline reads it out of the
//! static library's metadata records, alongside the module's generated
//! `.swiftinterface`. `dir_static!` picks the profile placement: `__DATA`
//! for a debug build's linked image, `__DWARF` for a release build, where
//! `ld` strips the record out of the shipped binary.

const NAME: &[u8] = b"waterui_meta_apple_swift_module";
const PAYLOAD: &[u8] = env!("WATERUI_APPLE_SWIFT_MODULE").as_bytes();
const LEN: usize = NAME.len() + PAYLOAD.len() + 2;

waterui_meta::dir_static! {
    /// `swiftc -emit-module-path` wrote this module during `build.rs`; the
    /// record names it for the packager the way every `dir_entry` record
    /// does.
    pub static WATERUI_META_APPLE_SWIFT_MODULE: [u8; LEN] =
        waterui_meta::dir_entry::<LEN>(NAME, PAYLOAD);
}
