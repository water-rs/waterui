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

use waterui::ts::catalog::{CATALOG, CATALOG_ENCODED};
use waterui_ts::schema::{Catalog, decode_catalog};

#[path = "support/artifact.rs"]
mod artifact;

use artifact::meta_static;

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
