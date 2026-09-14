//! Fixture for `artifact_symbols`: one `#[used]` metadata static carrying a
//! NUL-terminated payload, plus one `no_mangle` export in the preview prefix.

#[used]
#[allow(non_upper_case_globals)]
static waterui_meta_test_probe: [u8; 6] = *b"hello\0";

#[unsafe(no_mangle)]
pub extern "C" fn waterui_preview_meta_static_probe() {}
