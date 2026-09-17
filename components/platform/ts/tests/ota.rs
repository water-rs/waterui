//! The bundle loader and the over-the-air client, end to end: real bundles
//! built on the real library, signed with a key generated here, served from a
//! socket the test owns, cached in a directory that goes away with the test.
//!
//! Every test says what it observes. The point of each is that removing the
//! feature it guards — a check in the verifier, a record in the store, a step
//! in the selection — makes it fail.

#![cfg(feature = "ota")]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;

use ed25519_dalek::{Signer as _, SigningKey};
use nami::Computed;
use waterui::ts::{
    Baseline, BundleStore, Components, FetchError, LaunchError, Launched, Loader, Mount, NoProps,
    Ota, Outcome, RUNTIME_FINGERPRINT, Rejection, RequiredModule, Requirement,
};
use waterui_core::Environment;
use waterui_graphics::color::ColorScheme;
use waterui_locale::locales;
use waterui_testing::{SemanticApp, ui};
use waterui_ts::engine::{JsRuntime as _, JsValue};
use waterui_ts::schema::{
    BundleFile, BundleManifest, ContractHash, HexBytes, RuntimeFingerprint, Sha256Digest,
    SignedManifest, TsProps,
};

#[path = "support/http.rs"]
mod http;

/// The `waterui` JavaScript library, as a classic script.
const LIBRARY: &str = include_str!("fixtures/library.js");

/// The one module every bundle here carries and the binary mounts.
const MODULE: &str = "src/main.tsx";

/// What the binary requires: this build's runtime, and `MODULE` mounted with
/// no props.
const REQUIREMENT: Requirement = Requirement::new(
    RUNTIME_FINGERPRINT,
    &[RequiredModule::new(MODULE, NoProps::CONTRACT_HASH)],
);

/// The version the baseline is built as.
const BASELINE_VERSION: u64 = 10;

/// A whole application bundle: the library, one module whose view is a
/// `<Text>` reading `label`, and the `installRuntimeGlobal` call. `tag` is
/// published on `globalThis.__waterui_test_bundle`, so a test can ask the
/// runtime which bundle it evaluated.
fn bundle(tag: &str, label: &str, contract: u64) -> String {
    format!(
        "{LIBRARY}\n(function () {{\n  const {{ jsx, installRuntimeGlobal }} = globalThis.waterui;\n  \
         globalThis.__waterui_test_bundle = \"{tag}\";\n  \
         function Main() {{ return jsx(\"Text\", {{ children: \"{label}\" }}); }}\n  \
         installRuntimeGlobal({{ \"{MODULE}\": Main }}, {{ \"{MODULE}\": \"{contract:016x}\" }});\n\
         }})();\n"
    )
}

/// A bundle that throws while it evaluates, after publishing its tag.
fn throwing_bundle(tag: &str) -> String {
    format!(
        "{LIBRARY}\nglobalThis.__waterui_test_bundle = \"{tag}\";\nthrow new Error(\"boom\");\n"
    )
}

/// The manifest of `source` at `version`, for this binary's runtime, with
/// `MODULE` declared at its real contract.
fn manifest(version: u64, source: &str) -> BundleManifest {
    manifest_for(version, source, RUNTIME_FINGERPRINT, NoProps::CONTRACT_HASH)
}

fn manifest_for(
    version: u64,
    source: &str,
    runtime: RuntimeFingerprint,
    contract: u64,
) -> BundleManifest {
    use sha2::Digest as _;
    BundleManifest {
        version,
        runtime,
        bundle: BundleFile {
            url: format!("bundle-{version}.js"),
            sha256: Sha256Digest::new(sha2::Sha256::digest(source.as_bytes()).into()),
        },
        modules: BTreeMap::from([(String::from(MODULE), ContractHash(contract))]),
        translations: BTreeMap::new(),
    }
}

/// Signs `manifest` with `key`, the way `water ota publish` does.
fn sign(manifest: BundleManifest, key: &SigningKey) -> SignedManifest {
    let signature = key.sign(&manifest.signed_bytes());
    SignedManifest {
        manifest,
        signature: HexBytes::new(signature.to_bytes()),
    }
}

/// A fresh ed25519 key pair from the operating system's entropy.
fn keypair() -> SigningKey {
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed).expect("the operating system provides entropy");
    SigningKey::from_bytes(&seed)
}

/// The baseline: a bundle whose text reads `baseline`, at
/// `BASELINE_VERSION`.
fn baseline() -> Baseline {
    let source = bundle("baseline", "baseline", NoProps::CONTRACT_HASH);
    let manifest = manifest(BASELINE_VERSION, &source).to_json();
    Baseline::new(
        Box::leak(source.into_boxed_str()),
        Box::leak(manifest.into_boxed_str()),
    )
}

/// The environment a launch starts from: a colour scheme for the theme the
/// module may read, and a locale for the text it renders.
fn environment() -> Environment {
    Environment::new()
        .store::<ColorScheme, Computed<ColorScheme>>(Computed::constant(ColorScheme::Light))
        .extending(locales::EN)
}

fn loader() -> Loader<Components> {
    Loader::new(REQUIREMENT, baseline(), Components, environment())
}

/// An `Ota` over `store` for `key`, with a manifest URL that nothing here
/// fetches from: launching reads the store, never the network.
fn ota(store: &BundleStore, key: &SigningKey) -> Ota {
    Ota::new(
        key.verifying_key().to_bytes(),
        "http://127.0.0.1:9/manifest.json",
        store.clone(),
    )
    .expect("the key and the URL are valid")
}

/// Writes a signed bundle into `store` the way a successful fetch does.
fn cache(store: &BundleStore, key: &SigningKey, version: u64, source: &str) {
    let signed = sign(manifest(version, source), key);
    store_write(store, version, &signed.to_json(), source.as_bytes());
}

/// The store's on-disk layout, written directly: one directory per version
/// holding `manifest.json` and `bundle.js`.
fn store_write(store: &BundleStore, version: u64, manifest: &str, bundle: &[u8]) {
    let dir = store.root().join("bundles").join(version.to_string());
    std::fs::create_dir_all(&dir).expect("the version directory is created");
    std::fs::write(dir.join("manifest.json"), manifest).expect("the manifest is written");
    std::fs::write(dir.join("bundle.js"), bundle).expect("the bundle is written");
}

/// The versions the store holds on disk.
fn cached_versions(store: &BundleStore) -> Vec<u64> {
    let dir = store.root().join("bundles");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut versions: Vec<u64> = entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    versions.sort_unstable();
    versions
}

/// The state file, parsed.
fn state(store: &BundleStore) -> serde_json::Value {
    let path = store.root().join("state.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("reading {}: {error}", path.display());
    });
    serde_json::from_str(&text).expect("the state file is JSON")
}

/// The tag the evaluated bundle published.
fn evaluated(launched: &Launched) -> String {
    match launched
        .handle()
        .runtime()
        .bridge()
        .engine()
        .eval("globalThis.__waterui_test_bundle", "test.js")
        .expect("the tag reads")
    {
        JsValue::String(tag) => tag,
        other => panic!("the bundle published {other:?} as its tag"),
    }
}

/// Mounts `MODULE` from the launched runtime in a test session over the
/// launched environment.
fn session(launched: &Launched) -> SemanticApp {
    let view = Mount::new::<NoProps>(MODULE, NoProps {})
        .try_mount(launched.environment())
        .unwrap_or_else(|error| panic!("mounting the module: {error}"));
    let view = RefCell::new(Some(view));
    ui().environment(launched.environment().clone())
        .viewport(320, 240)
        .mount(move || {
            view.borrow_mut()
                .take()
                .expect("the test session realizes its root once")
        })
}

/// Whether anything at all exists under `root`.
fn untouched(root: &Path) -> bool {
    !root.exists()
}

// ---------------------------------------------------------------------------
// The baseline
// ---------------------------------------------------------------------------

#[test]
fn ts_ota_the_baseline_launches_and_touches_nothing() {
    let launched = loader()
        .baseline_only()
        .unwrap_or_else(|error| panic!("launching the baseline: {error}"));
    assert!(launched.is_baseline());
    assert_eq!(launched.version(), BASELINE_VERSION);
    assert_eq!(evaluated(&launched), "baseline");
    launched
        .booted()
        .expect("nothing to record for the baseline");

    let mut app = session(&launched);
    app.query().label("baseline").assert_exists();
}

#[test]
fn ts_ota_a_baseline_that_does_not_match_the_binary_is_a_hard_error() {
    // The same bundle, whose manifest declares another runtime: a
    // build-pipeline bug, and never a bundle to fall past.
    let source = bundle("baseline", "baseline", NoProps::CONTRACT_HASH);
    let manifest = manifest_for(
        BASELINE_VERSION,
        &source,
        RuntimeFingerprint::new(1, 2),
        NoProps::CONTRACT_HASH,
    )
    .to_json();
    let baseline = Baseline::new(
        Box::leak(source.into_boxed_str()),
        Box::leak(manifest.into_boxed_str()),
    );
    let error = Loader::new(REQUIREMENT, baseline, Components, environment())
        .baseline_only()
        .expect_err("a baseline for another runtime is refused");
    assert!(
        matches!(
            error,
            LaunchError::BaselineRejected(Rejection::Runtime { .. })
        ),
        "{error}"
    );
}

#[test]
fn ts_ota_a_baseline_whose_bytes_are_not_its_manifests_is_a_hard_error() {
    let source = bundle("baseline", "baseline", NoProps::CONTRACT_HASH);
    let manifest = manifest(BASELINE_VERSION, &source).to_json();
    let altered = source.replace("baseline", "tampered");
    let baseline = Baseline::new(
        Box::leak(altered.into_boxed_str()),
        Box::leak(manifest.into_boxed_str()),
    );
    let error = Loader::new(REQUIREMENT, baseline, Components, environment())
        .baseline_only()
        .expect_err("a baseline that is not its manifest's file is refused");
    assert!(
        matches!(
            error,
            LaunchError::BaselineRejected(Rejection::Digest { .. })
        ),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// Verification before anything is written
// ---------------------------------------------------------------------------

/// Serves `signed` and `source` and runs one fetch against them.
///
/// Answers with the fetch outcome and the server, whose request log says
/// whether the bundle was downloaded at all.
fn fetch(
    store: &BundleStore,
    key: &SigningKey,
    signed: &SignedManifest,
    source: &str,
) -> (Result<Outcome, FetchError>, http::Server) {
    let version = signed.manifest.version;
    let server = http::Server::serve(BTreeMap::from([
        (
            String::from("/manifest.json"),
            signed.to_json().into_bytes(),
        ),
        (format!("/bundle-{version}.js"), source.as_bytes().to_vec()),
    ]));
    let ota = Ota::new(
        key.verifying_key().to_bytes(),
        &server.url("manifest.json"),
        store.clone(),
    )
    .expect("the key and the URL are valid");
    let outcome = futures_lite::future::block_on(ota.fetch(&REQUIREMENT, BASELINE_VERSION));
    (outcome, server)
}

#[test]
fn ts_ota_a_tampered_bundle_is_rejected_before_anything_is_written() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let source = bundle("v11", "eleven", NoProps::CONTRACT_HASH);
    let signed = sign(manifest(11, &source), &key);
    // The file served is not the file the manifest was published with.
    let altered = source.replace("eleven", "twelve");

    let (outcome, server) = fetch(&store, &key, &signed, &altered);
    let error = outcome.expect_err("the altered bundle is refused");
    assert!(
        matches!(error, FetchError::Rejected(Rejection::Digest { .. })),
        "{error}"
    );
    assert!(untouched(store.root()), "nothing was written to the store");
    assert_eq!(
        server.requests(),
        ["/manifest.json", "/bundle-11.js"],
        "the manifest passed, so the bundle was downloaded and then refused"
    );
}

#[test]
fn ts_ota_a_manifest_altered_after_signing_is_rejected_before_the_bundle_is_fetched() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let source = bundle("v11", "eleven", NoProps::CONTRACT_HASH);
    let mut signed = sign(manifest(11, &source), &key);
    // A version bump after signing: the document no longer matches its
    // signature.
    signed.manifest.version = 12;

    let (outcome, server) = fetch(&store, &key, &signed, &source);
    let error = outcome.expect_err("the altered manifest is refused");
    assert!(
        matches!(error, FetchError::Rejected(Rejection::Signature)),
        "{error}"
    );
    assert!(untouched(store.root()));
    assert_eq!(
        server.requests(),
        ["/manifest.json"],
        "the bundle was never requested"
    );
}

#[test]
fn ts_ota_a_bundle_signed_with_another_key_is_rejected() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let other = keypair();
    let source = bundle("v11", "eleven", NoProps::CONTRACT_HASH);
    let signed = sign(manifest(11, &source), &other);

    let (outcome, server) = fetch(&store, &key, &signed, &source);
    let error = outcome.expect_err("another publisher's bundle is refused");
    assert!(
        matches!(error, FetchError::Rejected(Rejection::Signature)),
        "{error}"
    );
    assert!(untouched(store.root()));
    assert_eq!(server.requests(), ["/manifest.json"]);
}

#[test]
fn ts_ota_a_bundle_for_another_runtime_is_rejected_before_the_bundle_is_fetched() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let source = bundle("v11", "eleven", NoProps::CONTRACT_HASH);
    let other_runtime = RuntimeFingerprint::new(
        RUNTIME_FINGERPRINT.library() ^ 1,
        RUNTIME_FINGERPRINT.catalog(),
    );
    let signed = sign(
        manifest_for(11, &source, other_runtime, NoProps::CONTRACT_HASH),
        &key,
    );

    let (outcome, server) = fetch(&store, &key, &signed, &source);
    let error = outcome.expect_err("a bundle for another runtime is refused");
    match error {
        FetchError::Rejected(Rejection::Runtime { declared, expected }) => {
            assert_eq!(declared, other_runtime);
            assert_eq!(expected, RUNTIME_FINGERPRINT);
        }
        other => panic!("{other}"),
    }
    assert!(untouched(store.root()));
    assert_eq!(server.requests(), ["/manifest.json"]);
}

#[test]
fn ts_ota_a_bundle_missing_a_mounted_module_is_rejected_before_the_bundle_is_fetched() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let source = bundle("v11", "eleven", NoProps::CONTRACT_HASH);
    let mut manifest = manifest(11, &source);
    manifest.modules.clear();
    manifest.modules.insert(
        String::from("src/other.tsx"),
        ContractHash(NoProps::CONTRACT_HASH),
    );
    let signed = sign(manifest, &key);

    let (outcome, server) = fetch(&store, &key, &signed, &source);
    let error = outcome.expect_err("a bundle without the mounted module is refused");
    match error {
        FetchError::Rejected(Rejection::MissingModule { id }) => assert_eq!(id, MODULE),
        other => panic!("{other}"),
    }
    assert!(untouched(store.root()));
    assert_eq!(server.requests(), ["/manifest.json"]);
}

#[test]
fn ts_ota_a_module_built_against_another_contract_is_rejected_naming_both_hashes() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let source = bundle("v11", "eleven", NoProps::CONTRACT_HASH ^ 1);
    let signed = sign(
        manifest_for(11, &source, RUNTIME_FINGERPRINT, NoProps::CONTRACT_HASH ^ 1),
        &key,
    );

    let (outcome, server) = fetch(&store, &key, &signed, &source);
    let error = outcome.expect_err("a module built against another contract is refused");
    match error {
        FetchError::Rejected(Rejection::ContractMismatch {
            id,
            expected,
            declared,
        }) => {
            assert_eq!(id, MODULE);
            assert_eq!(expected, ContractHash(NoProps::CONTRACT_HASH));
            assert_eq!(declared, ContractHash(NoProps::CONTRACT_HASH ^ 1));
        }
        other => panic!("{other}"),
    }
    assert!(untouched(store.root()));
    assert_eq!(server.requests(), ["/manifest.json"]);
}

#[test]
fn ts_ota_a_bundle_not_newer_than_the_baseline_is_not_fetched() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let source = bundle("v10", "ten", NoProps::CONTRACT_HASH);
    let signed = sign(manifest(BASELINE_VERSION, &source), &key);

    let (outcome, server) = fetch(&store, &key, &signed, &source);
    assert_eq!(
        outcome.expect("the fetch completes"),
        Outcome::NotNewer {
            version: BASELINE_VERSION,
            baseline: BASELINE_VERSION
        }
    );
    assert!(untouched(store.root()));
    assert_eq!(server.requests(), ["/manifest.json"]);
}

#[test]
fn ts_ota_a_server_that_is_not_there_is_not_an_error_of_the_launch() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let ota = ota(&store, &key);
    let error = futures_lite::future::block_on(ota.fetch(&REQUIREMENT, BASELINE_VERSION))
        .expect_err("port 9 answers nothing");
    assert!(matches!(error, FetchError::Network { .. }), "{error}");
    assert!(untouched(store.root()));
}

// ---------------------------------------------------------------------------
// A valid download, and the launch that follows
// ---------------------------------------------------------------------------

#[test]
fn ts_ota_a_valid_download_is_cached_and_chosen_on_the_next_launch_over_the_baseline() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let source = bundle("v11", "eleven", NoProps::CONTRACT_HASH);
    let signed = sign(manifest(11, &source), &key);

    // This launch: the baseline, then the fetch that fills the cache.
    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert!(
        launched.is_baseline(),
        "an empty store launches the baseline"
    );
    let (outcome, server) = fetch(&store, &key, &signed, &source);
    assert_eq!(
        outcome.expect("the fetch completes"),
        Outcome::Cached { version: 11 }
    );
    assert_eq!(server.requests(), ["/manifest.json", "/bundle-11.js"]);
    assert_eq!(cached_versions(&store), [11]);
    assert_eq!(
        evaluated(&launched),
        "baseline",
        "the running process keeps the bundle it launched with"
    );
    drop(launched);

    // The next launch: the cached bundle, verified again, mounted for real.
    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert!(!launched.is_baseline());
    assert_eq!(launched.version(), 11);
    assert_eq!(evaluated(&launched), "v11");
    assert_eq!(
        state(&store)["booting"],
        11,
        "the launch recorded that 11 is booting before it evaluated it"
    );
    let mut app = session(&launched);
    app.query().label("eleven").assert_exists();
    launched.booted().expect("the record clears");
    assert_eq!(state(&store)["booting"], serde_json::Value::Null);

    // Fetching the same version again downloads nothing.
    let (outcome, server) = fetch(&store, &key, &signed, &source);
    assert_eq!(
        outcome.expect("the fetch completes"),
        Outcome::AlreadyCached { version: 11 }
    );
    assert_eq!(server.requests(), ["/manifest.json"]);
}

#[test]
fn ts_ota_the_newest_verified_bundle_wins() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    cache(
        &store,
        &key,
        11,
        &bundle("v11", "eleven", NoProps::CONTRACT_HASH),
    );
    cache(
        &store,
        &key,
        13,
        &bundle("v13", "thirteen", NoProps::CONTRACT_HASH),
    );
    cache(
        &store,
        &key,
        12,
        &bundle("v12", "twelve", NoProps::CONTRACT_HASH),
    );

    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert_eq!(launched.version(), 13);
    assert_eq!(evaluated(&launched), "v13");
}

#[test]
fn ts_ota_a_cached_bundle_that_no_longer_verifies_is_removed_not_marked_bad() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    // Cached by a previous binary whose runtime differed: stale now.
    let source = bundle("v12", "twelve", NoProps::CONTRACT_HASH);
    let stale = sign(
        manifest_for(
            12,
            &source,
            RuntimeFingerprint::new(1, 2),
            NoProps::CONTRACT_HASH,
        ),
        &key,
    );
    store_write(&store, 12, &stale.to_json(), source.as_bytes());
    cache(
        &store,
        &key,
        11,
        &bundle("v11", "eleven", NoProps::CONTRACT_HASH),
    );
    // Cached at some point, altered since.
    let signed = sign(
        manifest(13, &bundle("v13", "thirteen", NoProps::CONTRACT_HASH)),
        &key,
    );
    store_write(&store, 13, &signed.to_json(), b"not the bundle");

    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert_eq!(launched.version(), 11);
    assert_eq!(cached_versions(&store), [11], "12 and 13 were removed");
    assert_eq!(
        state(&store)["bad"],
        serde_json::json!([]),
        "neither is marked bad: 12 may verify under the binary it was cached for"
    );
}

#[test]
fn ts_ota_cached_bundles_the_baseline_supersedes_are_removed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    cache(
        &store,
        &key,
        9,
        &bundle("v9", "nine", NoProps::CONTRACT_HASH),
    );
    cache(
        &store,
        &key,
        BASELINE_VERSION,
        &bundle("v10", "ten", NoProps::CONTRACT_HASH),
    );

    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert!(launched.is_baseline());
    assert_eq!(cached_versions(&store), Vec::<u64>::new());
}

// ---------------------------------------------------------------------------
// Rollback
// ---------------------------------------------------------------------------

#[test]
fn ts_ota_a_bundle_that_throws_is_marked_bad_and_the_previous_one_loads() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    cache(
        &store,
        &key,
        11,
        &bundle("v11", "eleven", NoProps::CONTRACT_HASH),
    );
    cache(&store, &key, 12, &throwing_bundle("v12"));

    // First launch: 12 throws and is fallen past in the same launch.
    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert_eq!(launched.version(), 11);
    assert_eq!(evaluated(&launched), "v11");
    assert_eq!(state(&store)["bad"], serde_json::json!([12]));
    assert_eq!(cached_versions(&store), [11]);
    launched.booted().expect("the record clears");
    drop(launched);

    // Second launch, with 12 cached again by a fetch that trusts the store's
    // memory: it is skipped without being read.
    cache(&store, &key, 12, &throwing_bundle("v12"));
    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert_eq!(launched.version(), 11);
    assert_eq!(evaluated(&launched), "v11");

    // And a fetch of 12 never downloads it again.
    let source = throwing_bundle("v12");
    let signed = sign(manifest(12, &source), &key);
    let (outcome, server) = fetch(&store, &key, &signed, &source);
    assert_eq!(
        outcome.expect("the fetch completes"),
        Outcome::KnownBad { version: 12 }
    );
    assert_eq!(server.requests(), ["/manifest.json"]);
}

#[test]
fn ts_ota_a_boot_record_left_set_marks_the_version_bad_on_the_next_launch() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    cache(
        &store,
        &key,
        11,
        &bundle("v11", "eleven", NoProps::CONTRACT_HASH),
    );
    cache(
        &store,
        &key,
        12,
        &bundle("v12", "twelve", NoProps::CONTRACT_HASH),
    );

    // A launch that evaluates 12 and dies before reporting that it booted —
    // a `tsx!` mount that panicked, say. Not calling `booted` is that death.
    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert_eq!(launched.version(), 12);
    assert_eq!(state(&store)["booting"], 12);
    drop(launched);

    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert_eq!(launched.version(), 11, "12 is marked bad and 11 is next");
    assert_eq!(evaluated(&launched), "v11");
    assert_eq!(state(&store)["bad"], serde_json::json!([12]));
    assert_eq!(state(&store)["booting"], 11);
    assert_eq!(cached_versions(&store), [11]);
    launched.booted().expect("the record clears");
    assert_eq!(state(&store)["booting"], serde_json::Value::Null);
}

#[test]
fn ts_ota_the_baseline_is_never_marked_bad() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    cache(&store, &key, 11, &throwing_bundle("v11"));

    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert!(launched.is_baseline());
    assert_eq!(state(&store)["booting"], serde_json::Value::Null);
    assert_eq!(state(&store)["bad"], serde_json::json!([11]));
    launched.booted().expect("nothing to record");
    assert_eq!(state(&store)["booting"], serde_json::Value::Null);
}

// ---------------------------------------------------------------------------
// Translations
// ---------------------------------------------------------------------------

#[test]
fn ts_ota_a_bundle_carrying_translations_installs_them_for_its_modules() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let source = bundle("v11", "greeting", NoProps::CONTRACT_HASH);
    let mut manifest = manifest(11, &source);
    manifest.translations.insert(
        String::from("en"),
        String::from("greeting = \"Hello from the update\"\n"),
    );
    store_write(
        &store,
        11,
        &sign(manifest, &key).to_json(),
        source.as_bytes(),
    );

    let launched = loader()
        .load(&ota(&store, &key))
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert_eq!(launched.version(), 11);
    let mut app = session(&launched);
    app.query().label("Hello from the update").assert_exists();
}

#[test]
fn ts_ota_translations_that_do_not_parse_reject_the_bundle() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    let source = bundle("v11", "greeting", NoProps::CONTRACT_HASH);

    let mut bad_locale = manifest(11, &source);
    bad_locale.translations.insert(
        String::from("not a locale"),
        String::from("greeting = \"x\"\n"),
    );
    let (outcome, _server) = fetch(&store, &key, &sign(bad_locale, &key), &source);
    assert!(
        matches!(
            outcome.expect_err("refused"),
            FetchError::Rejected(Rejection::Locale { .. })
        ),
        "an invalid locale tag is a rejection, not a panic"
    );

    let mut bad_toml = manifest(11, &source);
    bad_toml
        .translations
        .insert(String::from("en"), String::from("greeting = \n"));
    let (outcome, _server) = fetch(&store, &key, &sign(bad_toml, &key), &source);
    assert!(
        matches!(
            outcome.expect_err("refused"),
            FetchError::Rejected(Rejection::Translation { .. })
        ),
        "a translation file that does not parse is a rejection"
    );
    assert!(untouched(store.root()));
}

/// The baseline-only launch has no `Ota` in its signature and reads no
/// store: this is the observation behind the type-level guarantee that a
/// build which configures no manifest URL constructs no client.
#[test]
fn ts_ota_a_baseline_only_launch_reads_no_store() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = BundleStore::new(dir.path().join("store"));
    let key = keypair();
    cache(
        &store,
        &key,
        11,
        &bundle("v11", "eleven", NoProps::CONTRACT_HASH),
    );
    let root_before = std::fs::metadata(store.root()).expect("the store exists");

    let launched = loader()
        .baseline_only()
        .unwrap_or_else(|error| panic!("launching: {error}"));
    assert!(launched.is_baseline());
    assert_eq!(evaluated(&launched), "baseline");
    assert!(
        !store.root().join("state.json").exists(),
        "no state was written"
    );
    assert_eq!(
        std::fs::metadata(store.root())
            .expect("the store still exists")
            .modified()
            .expect("mtime"),
        root_before.modified().expect("mtime"),
        "the store directory was not touched"
    );
}
