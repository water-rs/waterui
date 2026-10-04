//! Fixture for `artifact_symbols`: one `#[used]` metadata static carrying a
//! NUL-terminated payload, plus one `no_mangle` export in the preview prefix.
//! The static mirrors what the macros emit: `#[cfg(debug_assertions)]` so a
//! release rlib carries none — `#[used]` is linker-retained, and the CLI only
//! ever reads a dev-profile host rlib.

#[cfg(debug_assertions)]
#[used]
#[allow(non_upper_case_globals)]
static waterui_meta_test_probe: [u8; 6] = *b"hello\0";

#[unsafe(no_mangle)]
pub extern "C" fn waterui_preview_meta_static_probe() {}
