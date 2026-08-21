//! The macOS application bundle CEF refuses to start without.
//!
//! `cef_initialize` traps inside the framework when the browser process is not
//! bundled, and its child processes are launched from helper applications
//! Chromium locates relative to the outer bundle, so a plain
//! `target/debug/deps/real_engine-*` can never initialize CEF. This module
//! stages the same layout `water package` produces — main bundle, staged
//! framework, runtime manifest and the five helper applications — and the test
//! binary re-executes itself from it.
//!
//! The executables are hard links rather than copies: a debug build of this
//! test is hundreds of megabytes and there are six of them, and a link is
//! indistinguishable from a copy to `NSBundle`, which resolves the bundle from
//! the path the process was executed with.

use std::path::{Path, PathBuf};

use askama::Template;
use serde::Serialize;

/// The name every bundle, helper and executable in the staged layout derives
/// from.
///
/// Chromium computes a helper's path from the main executable's file name, so
/// this has to match `CFBundleExecutable` in `Info.plist`.
pub const EXECUTABLE: &str = "waterui-cef-real-engine";

const BUNDLE_IDENTIFIER: &str = "dev.waterui.browser-cef.real-engine";

const FRAMEWORK: &str = "Chromium Embedded Framework.framework";

/// The helper variants Chromium chooses between by child process type, as
/// `(name suffix, bundle identifier suffix)`.
///
/// All five exist for the same reason they do in a packaged application: the
/// GPU, renderer and alerts processes are launched from their own bundles, and
/// a missing one is a launch failure rather than a fallback.
const HELPER_VARIANTS: [(&str, &str); 5] = [
    ("", ""),
    (" (Alerts)", ".alerts"),
    (" (GPU)", ".gpu"),
    (" (Plugin)", ".plugin"),
    (" (Renderer)", ".renderer"),
];

/// The manifest `CefRuntimePaths::validate` reads to confirm the staged runtime
/// is the one this build links against.
#[derive(Serialize)]
struct RuntimeIdentity<'a> {
    schema_version: u32,
    engine: &'a str,
    version: String,
    platform: &'a str,
    architecture: &'a str,
}

#[derive(Template)]
#[template(path = "macos/CefHelperInfo.plist.tpl", escape = "none")]
struct HelperInfoPlist<'a> {
    bundle_identifier: &'a str,
    helper_name: &'a str,
    product_name: &'a str,
}

/// Whether this process is already running as the bundled executable.
///
/// The condition is the one CEF actually cares about — an executable inside
/// `Contents/MacOS` is what makes `NSBundle` report a bundle — rather than a
/// flag this test hands itself.
pub fn running_bundled() -> bool {
    executable()
        .parent()
        .is_some_and(|directory| directory.ends_with("Contents/MacOS"))
}

/// Whether this process was launched by Chromium as one of its child processes.
pub fn is_child_process() -> bool {
    std::env::args().any(|argument| argument.starts_with("--type="))
}

/// The CEF distribution `cef-dll-sys` downloaded and this crate linked against.
///
/// # Panics
///
/// Panics when the compiled-in distribution is not on disk, which means the
/// build directory it came from was removed after this test was linked.
pub fn distribution() -> PathBuf {
    let distribution = PathBuf::from(env!("WATERUI_CEF_DISTRIBUTION"));
    let framework = distribution.join(FRAMEWORK);
    assert!(
        framework.is_dir(),
        "the CEF distribution this test linked against is gone: {} does not exist. It is \
         downloaded by `cef-dll-sys` into the build directory, so rebuild with `cargo test -p \
         waterui-browser-cef --features real-engine --test real_engine` rather than running a \
         stale binary.",
        framework.display()
    );
    distribution
}

/// Everything this test writes, inside the build directory cargo already owns.
pub fn workspace() -> PathBuf {
    PathBuf::from(env!("OUT_DIR")).join("real-engine")
}

/// Stages the bundle around a fresh copy of this executable and returns the
/// executable to run.
///
/// # Panics
///
/// Panics when the bundle cannot be written.
pub fn stage() -> PathBuf {
    let application = workspace().join(format!("{EXECUTABLE}.app"));
    if application.exists() {
        std::fs::remove_dir_all(&application).expect("clear the previously staged bundle");
    }
    let contents = application.join("Contents");
    let frameworks = contents.join("Frameworks");
    let executable = contents.join("MacOS").join(EXECUTABLE);

    create_directory(&frameworks);
    create_directory(&contents.join("MacOS"));
    create_directory(&contents.join("Resources/waterui-browser/cef"));
    write(&contents.join("Info.plist"), include_str!("Info.plist"));
    write(&contents.join("PkgInfo"), "APPL????");
    link(&executable);

    // The framework has to live inside the bundle for real. Chromium's child
    // processes load it *after* entering the seatbelt sandbox, which grants them
    // the bundle's own paths and nothing else, so a symlink out to the build
    // directory fails with `file system sandbox blocked open()` in every helper.
    // Hard links give the bundle real paths without copying 322 MB.
    clone_tree(&distribution().join(FRAMEWORK), &frameworks.join(FRAMEWORK));

    let identity = RuntimeIdentity {
        schema_version: 1,
        engine: "cef",
        version: format!(
            "{}.{}.{}",
            cef::sys::CEF_VERSION_MAJOR,
            cef::sys::CEF_VERSION_MINOR,
            cef::sys::CEF_VERSION_PATCH
        ),
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
    };
    write(
        &contents.join("Resources/waterui-browser/cef/runtime.json"),
        &serde_json::to_string_pretty(&identity).expect("serialize the staged runtime identity"),
    );

    for (name_suffix, identifier_suffix) in HELPER_VARIANTS {
        let helper_name = format!("{EXECUTABLE} Helper{name_suffix}");
        let helper_contents = frameworks
            .join(format!("{helper_name}.app"))
            .join("Contents");
        create_directory(&helper_contents.join("MacOS"));
        link(&helper_contents.join("MacOS").join(&helper_name));
        write(&helper_contents.join("PkgInfo"), "APPL????");
        write(
            &helper_contents.join("Info.plist"),
            &HelperInfoPlist {
                bundle_identifier: &format!("{BUNDLE_IDENTIFIER}.helper{identifier_suffix}"),
                helper_name: &helper_name,
                product_name: EXECUTABLE,
            }
            .render()
            .expect("render the CEF helper Info.plist"),
        );
    }

    executable
}

fn executable() -> PathBuf {
    std::env::current_exe().expect("resolve this test executable")
}

fn create_directory(path: &Path) {
    std::fs::create_dir_all(path)
        .unwrap_or_else(|error| panic!("create {}: {error}", path.display()));
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents)
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

fn link(path: &Path) {
    std::fs::hard_link(executable(), path)
        .unwrap_or_else(|error| panic!("link this executable into {}: {error}", path.display()));
}

/// Reproduces a directory tree as real directories and hard-linked files.
fn clone_tree(from: &Path, to: &Path) {
    create_directory(to);
    let entries =
        std::fs::read_dir(from).unwrap_or_else(|error| panic!("read {}: {error}", from.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("read {}: {error}", from.display()));
        let source = entry.path();
        let destination = to.join(entry.file_name());
        let kind = entry
            .file_type()
            .unwrap_or_else(|error| panic!("stat {}: {error}", source.display()));
        if kind.is_dir() {
            clone_tree(&source, &destination);
        } else if kind.is_symlink() {
            let target = std::fs::read_link(&source)
                .unwrap_or_else(|error| panic!("read link {}: {error}", source.display()));
            std::os::unix::fs::symlink(target, &destination)
                .unwrap_or_else(|error| panic!("link {}: {error}", destination.display()));
        } else {
            std::fs::hard_link(&source, &destination)
                .unwrap_or_else(|error| panic!("link {}: {error}", destination.display()));
        }
    }
}
