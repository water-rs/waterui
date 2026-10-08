//! `water create` against the published stable `framework.json`.
//!
//! The fixture is the `framework.json` attached to the v0.5.2 framework
//! release, verbatim. That release pins its Apple backend as the separate
//! Swift `apple-backend` package and its Hydrolysis backend as a registry
//! crate, so it declares neither `apple-backend-path` nor
//! `hydrolysis-path`. This CLI scaffolds only a framework that carries
//! both backends in its own tree. The test pins down that such a framework
//! is rejected while the certification is verified: before any network
//! fetch and before the project directory exists, with an error that names
//! what is missing and the ways forward.

use std::path::Path;

use waterui_cli::project::{CreateOptions, FailToCreateProject, Project};
use waterui_cli::project_types::BundleIdentifier;

#[test]
fn create_rejects_the_published_stable_manifest_without_backend_members() {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/framework_manifest_v0.5.2.json");
    let directory = tempfile::tempdir().expect("scratch directory");
    let project = directory.path().join("stable-probe");

    let result = smol::block_on(Project::create(
        &waterui_cli::toolchain::Host::current(),
        &project,
        CreateOptions {
            name: "Stable Probe".to_owned(),
            bundle_identifier: BundleIdentifier::try_from("dev.waterui.stableProbe")
                .expect("valid bundle identifier"),
            waterui_path: None,
            channel: None,
            framework_manifest: Some(manifest),
            framework: None,
            framework_lock: None,
            author: "waterui-test".to_owned(),
            web: None,
        },
    ));

    let Err(FailToCreateProject::Framework(error)) = result else {
        panic!("the v0.5.2 manifest must fail framework resolution");
    };
    let error = format!("{error:#}");
    assert!(
        error.contains(
            "the stable framework v0.5.2 declares no `apple-backend-path`, `hydrolysis-path`:"
        ),
        "{error}"
    );
    assert!(
        error.contains("`water create <name> --channel dev`"),
        "{error}"
    );
    // The CLI line that scaffolds v0.5.2 starts at the floor the framework
    // declares (`minimum-cli-version = "0.3.2"`).
    assert!(
        error.contains("cargo install waterui-cli --version '>=0.3.2, <"),
        "{error}"
    );
    assert!(
        !project.exists(),
        "a rejected framework must leave nothing behind"
    );
}
