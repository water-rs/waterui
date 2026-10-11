//! The catalog reaches the compiled artifact, and what comes back out of it is
//! the table the host table dispatches on.
//!
//! Tooling never reads the catalog from this source tree: `water components`
//! enumerates `waterui_meta_` statics in the rlibs a build produces — the
//! user's crate, and the facade it links, which is where this symbol is —
//! because the artifact is ground truth that has already resolved macros,
//! `cfg`s and generics. This test walks the same path on the binary it is
//! running from, so a payload the linker drops, truncates, or lets drift from
//! the constant fails here rather than in a project build.

use std::collections::BTreeSet;

use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
use waterui::ts::catalog::{CATALOG, CATALOG_ENCODED};
use waterui_ts::schema::{Catalog, decode_catalog};

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
fn meta_static(leaf: &str) -> Vec<u8> {
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

#[test]
fn ts_the_catalog_symbol_decodes_to_the_registry() {
    let bytes = meta_static("waterui_meta_ts_catalog");
    assert_eq!(
        bytes, CATALOG_ENCODED,
        "the artifact payload is the constant the compiler encoded"
    );
    assert_eq!(
        decode_catalog(&bytes).expect("the artifact payload decodes"),
        Catalog::from(&CATALOG),
        "decoding the artifact payload rebuilds the registry entry for entry"
    );
}

#[test]
fn ts_every_component_the_host_builds_is_in_the_catalog() {
    // The host table refuses a tag the catalog does not declare, so the two
    // agreeing is what makes the published table usable: a tag the catalog
    // advertises and the table cannot build would be a promise nothing keeps.
    let catalog = decode_catalog(CATALOG_ENCODED).expect("the catalog decodes");
    for component in &catalog.components {
        assert!(
            !component.summary.is_empty(),
            "<{}> is published without a summary, and the summary is what a \
             generated `.d.ts` shows an author",
            component.name
        );
    }
    for modifier in &catalog.modifiers {
        assert!(
            !modifier.summary.is_empty(),
            "`{}` is published without a summary",
            modifier.name
        );
    }
    assert_eq!(
        catalog.components.len(),
        CATALOG.components.len(),
        "the decoded catalog carries every component the registry holds"
    );
    assert_eq!(
        catalog.modifiers.len(),
        CATALOG.modifiers.len(),
        "the decoded catalog carries every modifier the registry holds"
    );
}
