//! The runtime fingerprint: what the binary computes at launch is what the
//! `water` CLI derives from the two artifact statics.
//!
//! The CLI reads `waterui_meta_ts_runtime_library` out of the runtime crate's
//! rlib and `waterui_meta_ts_runtime_catalog` out of the facade's, decodes
//! each as a runtime half, and combines them with `RuntimeFingerprint::new`.
//! The binary names `waterui::ts::RUNTIME_FINGERPRINT`, built from the same
//! two constants. This reads both statics back out of the test executable's
//! own symbol table — the path the CLI takes — and holds the two to each
//! other.

use waterui::ts::catalog::{CATALOG_HALF_ENCODED, CATALOG_HASH};
use waterui::ts::{LIBRARY_HALF_ENCODED, LIBRARY_HASH, RUNTIME_FINGERPRINT};
use waterui_ts::schema::{FORMAT_VERSION, RuntimeFingerprint, RuntimePart, decode_runtime_half};

#[path = "support/artifact.rs"]
mod artifact;

#[test]
fn ts_the_library_half_reaches_the_artifact_and_decodes_to_the_constant() {
    let bytes = artifact::meta_static("waterui_meta_ts_runtime_library");
    assert_eq!(bytes, LIBRARY_HALF_ENCODED);
    let half = decode_runtime_half(&bytes).expect("the library half decodes");
    assert_eq!(half.part, RuntimePart::Library);
    assert_eq!(half.hash, LIBRARY_HASH);
}

#[test]
fn ts_the_catalog_half_reaches_the_artifact_and_decodes_to_the_constant() {
    let bytes = artifact::meta_static("waterui_meta_ts_runtime_catalog");
    assert_eq!(bytes, CATALOG_HALF_ENCODED);
    let half = decode_runtime_half(&bytes).expect("the catalog half decodes");
    assert_eq!(half.part, RuntimePart::Catalog);
    assert_eq!(half.hash, CATALOG_HASH);
}

#[test]
fn ts_the_fingerprint_the_cli_derives_is_the_one_the_binary_computes() {
    let library = decode_runtime_half(&artifact::meta_static("waterui_meta_ts_runtime_library"))
        .expect("the library half decodes");
    let catalog = decode_runtime_half(&artifact::meta_static("waterui_meta_ts_runtime_catalog"))
        .expect("the catalog half decodes");
    let derived = RuntimeFingerprint::new(library.hash, catalog.hash);
    assert_eq!(derived, RUNTIME_FINGERPRINT);
    assert_eq!(derived.format(), FORMAT_VERSION);
    // The text form is what the manifest carries, and it parses back.
    let text = derived.to_string();
    assert_eq!(
        text.parse::<RuntimeFingerprint>().expect("the text parses"),
        RUNTIME_FINGERPRINT
    );
    assert_eq!(
        text,
        format!("{FORMAT_VERSION}-{LIBRARY_HASH:016x}-{CATALOG_HASH:016x}")
    );
}

#[test]
fn ts_the_library_hash_is_over_the_files_the_runtime_ships() {
    // Recomputed here the way the constant is defined, from the same files:
    // a file added to `src/js` without being listed in `library.rs` fails
    // this rather than silently leaving the fingerprint unchanged.
    use waterui_ts::schema::{HASH_BASIS, hash_extend};
    let mut names: Vec<_> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src/js"))
        .expect("src/js is readable")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("js"))
        })
        .collect();
    names.sort();
    let mut state = HASH_BASIS;
    for name in &names {
        let contents = std::fs::read(format!("{}/src/js/{name}", env!("CARGO_MANIFEST_DIR")))
            .expect("the file is readable");
        state = hash_extend(state, name.as_bytes());
        state = hash_extend(state, &[0]);
        state = hash_extend(state, &contents);
        state = hash_extend(state, &[0]);
    }
    assert_eq!(state, LIBRARY_HASH, "the files hashed are {names:?}");
}
