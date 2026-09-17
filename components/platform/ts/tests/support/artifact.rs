//! Reading a `waterui_meta_*` static back out of the binary a test runs in.
//!
//! This is the path the `water` CLI takes through a user crate's rlib: find
//! the symbol, read its section from the symbol's address, and cut at the
//! first NUL, because a Mach-O symbol carries no size. Walking it here means a
//! payload the linker drops, truncates, or lets drift from the constant fails
//! in a test rather than in a project build.
//!
//! Included with `#[path]` rather than through `support/mod.rs`, so a test
//! target that needs only this does not pull the tree-comparison helpers in
//! with it.

use std::collections::BTreeSet;

use object::{Object as _, ObjectSection as _, ObjectSymbol as _};

/// The leaf segment of a demangled symbol name.
fn leaf_of(name: &str) -> Option<&str> {
    name.rsplit("::").next().filter(|leaf| !leaf.is_empty())
}

/// Demangle a raw symbol name, dropping the trailing disambiguation hash and
/// the leading underscore Mach-O adds to unmangled names.
fn demangled_name(raw: &str) -> String {
    let demangled = format!("{:#}", rustc_demangle::demangle(raw));
    demangled
        .strip_prefix('_')
        .map_or_else(|| demangled.clone(), str::to_owned)
}

/// Bytes of the `#[used] static` whose demangled leaf is `leaf`, read from the
/// running executable exactly the way the CLI reads them from an rlib.
///
/// # Panics
///
/// Panics when no such symbol carries section data, or when two symbols with
/// that leaf carry different payloads.
pub fn meta_static(leaf: &str) -> Vec<u8> {
    let path = std::env::current_exe().expect("the test binary has a path");
    let data = std::fs::read(&path).expect("the test binary is readable");
    let file = object::File::parse(&*data).expect("the test binary parses as an object file");
    let mut payloads = BTreeSet::new();
    for symbol in file.symbols() {
        let Ok(raw) = symbol.name() else { continue };
        if leaf_of(&demangled_name(raw)) != Some(leaf) {
            continue;
        }
        let Some(index) = symbol.section_index() else {
            continue;
        };
        let Ok(section) = file.section_by_index(index) else {
            continue;
        };
        let Ok(section_data) = section.data() else {
            continue;
        };
        let Ok(offset) = usize::try_from(symbol.address().wrapping_sub(section.address())) else {
            continue;
        };
        if let Some(bytes) = section_data.get(offset..) {
            payloads.insert(
                bytes
                    .split(|byte| *byte == 0)
                    .next()
                    .unwrap_or_default()
                    .to_vec(),
            );
        }
    }
    let mut payloads = payloads.into_iter();
    let found = payloads
        .next()
        .unwrap_or_else(|| panic!("no symbol with leaf `{leaf}` carries section data"));
    assert!(
        payloads.next().is_none(),
        "`{leaf}` is defined more than once with different payloads"
    );
    found
}
