//! The compiled `WaterUIBridge` Swift module's path as a canonical
//! `__wmeta` metadata record — the packaging pipeline reads it out of the
//! static library's symbol records, alongside the module's generated
//! `.swiftinterface`.

const NAME: &[u8] = b"waterui_meta_apple_swift_module";
const PAYLOAD: &[u8] = env!("WATERUI_APPLE_SWIFT_MODULE").as_bytes();
const LEN: usize = NAME.len() + PAYLOAD.len() + 2;

/// `swiftc -emit-module-path` wrote this module during `build.rs`; the
/// record names it for the packager the way every `dir_entry` record does.
/// `#[used]` keeps the record in the archive so the packager can read it
/// back out of the static library — the canonical `__wmeta` convention.
#[used]
#[unsafe(link_section = "__DATA,__wmeta")]
pub static WATERUI_META_APPLE_SWIFT_MODULE: [u8; LEN] =
    waterui_meta::dir_entry::<LEN>(NAME, PAYLOAD);
