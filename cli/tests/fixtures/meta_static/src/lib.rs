//! Fixture for `artifact_symbols`: one `#[used]` metadata static carrying a
//! NUL-terminated payload, one `#[used]` metadata-directory record parked in
//! the `.wmeta` section the way the real macros emit it, plus one
//! `no_mangle` export in the preview prefix. The statics mirror what the
//! macros emit: `#[cfg(debug_assertions)]` so a release rlib carries none —
//! `#[used]` is linker-retained, and the CLI only ever reads a dev-profile
//! host rlib.

#[cfg(debug_assertions)]
#[used]
#[allow(non_upper_case_globals)]
static waterui_meta_test_probe: [u8; 6] = *b"hello\0";

// The `.wmeta` spelling: `name`, NUL, `payload`, NUL — the channel that
// answers on a linked image whose symbol table is gone (a PE keeps none).
#[cfg(debug_assertions)]
#[used]
#[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".wmeta"))]
#[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__wmeta"))]
#[allow(non_upper_case_globals)]
static waterui_meta_test_dir_probe: [u8; 38] = *b"waterui_meta_test_dir_probe\0dir probe\0";

#[unsafe(no_mangle)]
pub extern "C" fn waterui_preview_meta_static_probe() {}
