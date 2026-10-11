//! Access to the framework checkout this crate lives inside.
//!
//! The framework revision is the repository's own HEAD and the "checkout" is
//! the workspace root — the standalone repository's git-pin dance is gone.
//! Tests that need real framework source — a `cargo metadata` graph over real
//! manifests, a checkout layout — read the enclosing tree. The Android-backend
//! resolution still fetches the pinned backend over the network; its callers
//! stay `#[ignore]`d and run in the nightly job.

use zenwave::Client as _;

mod clone;
pub use clone::checkout;

/// The `<owner>/<repo>` slug of a `https://github.com/…` remote.
fn github_slug(git: &str) -> &str {
    git.trim_end_matches('/')
        .trim_end_matches(".git")
        .strip_prefix("https://github.com/")
        .unwrap_or_else(|| panic!("{git} is not a github.com remote"))
}

/// `GET`s `url` and returns the body, asserting a success status.
pub fn fetch(url: &str) -> Vec<u8> {
    smol::block_on(async {
        let mut client = zenwave::client();
        let response = client
            .method(zenwave::Method::GET, url)
            .and_then(|request| request.header("User-Agent", env!("CARGO_PKG_NAME")))
            .unwrap_or_else(|error| panic!("invalid request to {url}: {error}"))
            .await
            .unwrap_or_else(|error| panic!("GET {url} failed: {error}"));
        assert!(
            response.status().is_success(),
            "GET {url} returned HTTP {}",
            response.status()
        );
        response
            .into_body()
            .into_bytes()
            .await
            .unwrap_or_else(|error| panic!("reading {url} failed: {error}"))
            .to_vec()
    })
}

/// The `raw.githubusercontent.com` URL of `path` at `rev` in the repository
/// `git` names.
pub fn raw_url(git: &str, rev: &str, path: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/{}/{rev}/{path}",
        github_slug(git)
    )
}

/// The `(commit, repository)` of the Android runtime the enclosing checkout
/// builds against: the `android-backend-revision` / `android-backend-url`
/// literals the workspace's root manifest declares.
pub fn android_backend() -> (String, String) {
    let manifest_path = checkout().join("Cargo.toml");
    let manifest = std::fs::read(&manifest_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", manifest_path.display()));
    let manifest: toml::Value = toml::from_str(
        std::str::from_utf8(&manifest)
            .unwrap_or_else(|error| panic!("the workspace manifest is not UTF-8: {error}")),
    )
    .unwrap_or_else(|error| panic!("the workspace manifest is not TOML: {error}"));
    let metadata = &manifest["package"]["metadata"]["waterui"];
    match (
        metadata
            .get("android-backend-revision")
            .and_then(toml::Value::as_str),
        metadata
            .get("android-backend-url")
            .and_then(toml::Value::as_str),
    ) {
        (Some(revision), Some(url)) => (revision.to_owned(), url.to_owned()),
        _ => panic!(
            "the workspace manifest declares android-backend-revision and \
             android-backend-url together or not at all"
        ),
    }
}
