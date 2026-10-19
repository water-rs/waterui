//! Reading a `waterui_meta_*` static back out of the binary a test runs in.
//!
//! This is the path the `water` CLI takes through a user crate's rlib: walk
//! the `.wmeta` (`__wmeta` on Mach-O) metadata directory's `name`, NUL,
//! `payload`, NUL records and match the name — never the symbol table,
//! because a linked PE image keeps no COFF symbol table at all. Walking it
//! here means a payload the linker drops, truncates, or lets drift from the
//! constant fails in a test rather than in a project build.
//!
//! Included with `#[path]` rather than through `support/mod.rs`, so a test
//! target that needs only this does not pull the tree-comparison helpers in
//! with it.

use std::collections::BTreeSet;

use object::{Object as _, ObjectSection as _};
use waterui_ts_schema::{DIR_SECTION, dir_records};

/// Payloads the `.wmeta` (`__wmeta` on Mach-O) metadata directory holds for
/// `leaf`. The section is the whole channel — it survives into a linked
/// image on every format, while a symbol table may not exist at all, so
/// each record names itself.
fn dir_payloads(file: &object::File<'_>, leaf: &str) -> BTreeSet<Vec<u8>> {
    let mut payloads = BTreeSet::new();
    for section in file.sections() {
        let Ok(name) = section.name() else { continue };
        if !matches!(name, DIR_SECTION | "__wmeta") {
            continue;
        }
        let Ok(data) = section.data() else { continue };
        for record in dir_records(data) {
            if record.name == leaf.as_bytes() {
                payloads.insert(record.payload.to_vec());
            }
        }
    }
    payloads
}

/// Bytes of the `#[used] static` whose record names `leaf`, read from the
/// running executable exactly the way the CLI reads them from an rlib.
///
/// # Panics
///
/// Panics when no record named `leaf` is in the directory section, or when
/// two records with that name carry different payloads.
pub fn meta_static(leaf: &str) -> Vec<u8> {
    let path = std::env::current_exe().expect("the test binary has a path");
    let data = std::fs::read(&path).expect("the test binary is readable");
    let file = object::File::parse(&*data).expect("the test binary parses as an object file");
    let mut payloads = dir_payloads(&file, leaf).into_iter();
    let found = payloads
        .next()
        .unwrap_or_else(|| panic!("no metadata record named `{leaf}` in the directory section"));
    assert!(
        payloads.next().is_none(),
        "`{leaf}` is defined more than once with different payloads"
    );
    found
}
