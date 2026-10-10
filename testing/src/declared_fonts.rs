//! The package under test's declared crate-local font files.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use hydrolysis::DeclaredFonts;
use serde::{Deserialize, Serialize};
use waterui_assets_planner::{FontPlatform, FontSource, GraphScope, dependency_font_declarations};

/// The platform a test binary renders on: the host it runs on, whose build
/// a declaration's `platforms` scope must include for its font to load.
#[cfg(target_os = "macos")]
const HOST_FONT_PLATFORM: FontPlatform = FontPlatform::Macos;
/// See the macOS definition.
#[cfg(target_os = "linux")]
const HOST_FONT_PLATFORM: FontPlatform = FontPlatform::Linux;
/// See the macOS definition.
#[cfg(target_os = "windows")]
const HOST_FONT_PLATFORM: FontPlatform = FontPlatform::Windows;
/// See the macOS definition. `target_os = "ios"` covers devices and
/// simulators alike.
#[cfg(target_os = "ios")]
const HOST_FONT_PLATFORM: FontPlatform = FontPlatform::Ios;
/// See the macOS definition.
#[cfg(target_os = "android")]
const HOST_FONT_PLATFORM: FontPlatform = FontPlatform::Android;

/// Suffix appended to the test executable's file name to name its cache.
const CACHE_SUFFIX: &str = ".waterui-declared-fonts.json";

/// The resolved declared font files of one test executable, keyed by the
/// identity of the executable, the package it was resolved for, and the
/// manifests of the graph's path packages.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DeclaredFontsCache {
    executable_len: u64,
    executable_modified_nanos: u128,
    manifest_dir: PathBuf,
    path_manifests: Vec<ManifestStamp>,
    fonts: Vec<PathBuf>,
}

/// A path package's `Cargo.toml` and its modification time in nanoseconds
/// since the Unix epoch.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ManifestStamp {
    path: PathBuf,
    modified_nanos: u128,
}

/// The graph's local font declarations and the stamps of its path packages'
/// manifests.
struct Resolved {
    fonts: Vec<PathBuf>,
    path_manifests: Vec<ManifestStamp>,
}

/// Installs the package under test's declared fonts into `env` unless `env`
/// already carries a [`DeclaredFonts`].
///
/// A host that staged the graph's declared fonts itself — a runtime binary
/// the `water` CLI launches, which installs an empty one — needs no cargo
/// resolution; a test binary under cargo resolves them through
/// `package_declared_fonts`.
pub fn install_declared_fonts(env: &mut waterui_core::Environment) {
    if env.get::<DeclaredFonts>().is_none() {
        env.insert(package_declared_fonts());
    }
}

/// The crate-local font files the package under test's dependency graph
/// declares under `[[package.metadata.waterui.assets.font]]`, as absolute
/// paths, resolved through `cargo metadata` the way the `water` CLI resolves
/// an application's.
///
/// The graph walked is the package under test's own closure — its normal
/// dependencies plus its own dev-dependencies — resolved with all features,
/// so a `required-feature` font a test enables is included. Fonts declared
/// by registry name or `remote_path` are left out: only the `water` CLI
/// stages those.
///
/// The result is cached next to the running test executable — its directory
/// must be writable — in `<executable>.waterui-declared-fonts.json`, keyed
/// by the executable's length and modification time, by
/// `CARGO_MANIFEST_DIR`, and by the modification time of every path
/// package's `Cargo.toml` in the graph. Registry and git packages are
/// immutable per version or revision, and a dependency change relinks the
/// binary, so the executable's identity keys the graph's shape; Cargo's
/// fingerprint does not cover `[package.metadata]`, so an edit to a path
/// package's font declarations relinks nothing, and the path packages'
/// manifests are keyed directly. A manifest that changed or no longer exists
/// invalidates the cache. The cache file is held under an exclusive lock for
/// the whole check-and-fill, so the concurrent test processes of one binary
/// run `cargo metadata` once and the rest wait and read its result.
///
/// # Panics
///
/// Panics if `CARGO_MANIFEST_DIR` — or, when the cache must be filled,
/// `CARGO` — is not set (the harness must run under `cargo test` or
/// `cargo nextest`), if `cargo metadata` fails, if a declaration is invalid,
/// or if any I/O on the executable, the cache file, or a path package's
/// manifest other than its absence fails, naming the path.
fn package_declared_fonts() -> DeclaredFonts {
    let manifest_dir = PathBuf::from(cargo_variable("CARGO_MANIFEST_DIR"));
    let executable = std::env::current_exe()
        .unwrap_or_else(|error| panic!("cannot locate the running test executable: {error}"));
    let (executable_len, executable_modified_nanos) = executable_identity(&executable);

    let mut cache_name = executable
        .file_name()
        .unwrap_or_else(|| {
            panic!(
                "test executable path `{}` has no file name",
                executable.display()
            )
        })
        .to_os_string();
    cache_name.push(CACHE_SUFFIX);
    let cache_path = executable.with_file_name(cache_name);

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&cache_path)
        .unwrap_or_else(|error| cache_io_failed(&cache_path, "open", &error));
    file.lock()
        .unwrap_or_else(|error| cache_io_failed(&cache_path, "lock", &error));

    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .unwrap_or_else(|error| cache_io_failed(&cache_path, "read", &error));
    // A fresh or unparsable cache file is a miss by construction — `create`
    // yields an empty file — so a failed parse resolves the fonts rather
    // than failing.
    if let Ok(cache) = serde_json::from_slice::<DeclaredFontsCache>(&contents)
        && cache.executable_len == executable_len
        && cache.executable_modified_nanos == executable_modified_nanos
        && cache.manifest_dir == manifest_dir
        && cache
            .path_manifests
            .iter()
            .all(|stamp| manifest_modified_nanos(&stamp.path) == Some(stamp.modified_nanos))
    {
        return DeclaredFonts::new(cache.fonts);
    }

    let resolved = resolve_declared_fonts(&manifest_dir);
    let cache = DeclaredFontsCache {
        executable_len,
        executable_modified_nanos,
        manifest_dir,
        path_manifests: resolved.path_manifests,
        fonts: resolved.fonts,
    };
    let contents = serde_json::to_vec(&cache).unwrap_or_else(|error| {
        panic!(
            "cannot serialize the declared font cache `{}`: {error}",
            cache_path.display()
        )
    });
    rewrite(&mut file, &contents)
        .unwrap_or_else(|error| cache_io_failed(&cache_path, "write", &error));
    DeclaredFonts::new(cache.fonts)
}

/// Runs `cargo metadata` on `manifest_dir`'s package and returns its graph's
/// local font declarations as absolute paths, with the stamps of its path
/// packages' manifests.
fn resolve_declared_fonts(manifest_dir: &Path) -> Resolved {
    let cargo = PathBuf::from(cargo_variable("CARGO"));
    let manifest = manifest_dir.join("Cargo.toml");
    // `--all-features`: a `required-feature` font a test enables must
    // resolve. `--filter-platform`: an unfiltered graph names every
    // platform's packages, which a build on a clean `CARGO_HOME` never
    // downloads. `--offline`: a test run must not reach the network.
    let metadata = cargo_metadata::MetadataCommand::new()
        .cargo_path(&cargo)
        .manifest_path(&manifest)
        .other_options(vec![
            "--all-features".to_string(),
            "--offline".to_string(),
            "--filter-platform".to_string(),
            env!("WATERUI_TESTING_TARGET").to_string(),
        ])
        .exec()
        .unwrap_or_else(|error| {
            panic!(
                "`cargo metadata` on `{}` failed resolving the package under test's declared \
                 fonts: {error}",
                manifest.display()
            )
        });
    let fonts = dependency_font_declarations(&metadata, GraphScope::Test)
        .unwrap_or_else(|error| panic!("{error}"))
        .into_iter()
        .filter(|declaration| declaration.bundled_on(HOST_FONT_PLATFORM))
        .filter_map(|declaration| match declaration.source {
            FontSource::Local {
                crate_root,
                relative_path,
            } => Some(crate_root.join(relative_path)),
            FontSource::Remote { .. } | FontSource::BuiltIn => None,
        })
        .collect();
    let path_manifests = metadata
        .packages
        .iter()
        .filter(|package| package.source.is_none())
        .map(|package| {
            let path = package.manifest_path.as_std_path().to_path_buf();
            let modified_nanos = manifest_modified_nanos(&path).unwrap_or_else(|| {
                panic!(
                    "`cargo metadata` reported the manifest `{}`, which does not exist",
                    path.display()
                )
            });
            ManifestStamp {
                path,
                modified_nanos,
            }
        })
        .collect();
    Resolved {
        fonts,
        path_manifests,
    }
}

/// A manifest's modification time in nanoseconds since the Unix epoch, or
/// `None` if it no longer exists.
fn manifest_modified_nanos(path: &Path) -> Option<u128> {
    match std::fs::metadata(path) {
        Ok(metadata) => Some(modified_nanos(&metadata, path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("cannot stat the manifest `{}`: {error}", path.display()),
    }
}

/// The executable's length and modification time in nanoseconds since the
/// Unix epoch.
fn executable_identity(executable: &Path) -> (u64, u128) {
    let metadata = std::fs::metadata(executable).unwrap_or_else(|error| {
        panic!(
            "cannot stat the test executable `{}`: {error}",
            executable.display()
        )
    });
    (metadata.len(), modified_nanos(&metadata, executable))
}

/// `path`'s modification time, from its `metadata`, in nanoseconds since the
/// Unix epoch.
fn modified_nanos(metadata: &std::fs::Metadata, path: &Path) -> u128 {
    metadata
        .modified()
        .unwrap_or_else(|error| {
            panic!(
                "cannot read the modification time of `{}`: {error}",
                path.display()
            )
        })
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| {
            panic!(
                "`{}` is modified before the Unix epoch: {error}",
                path.display()
            )
        })
        .as_nanos()
}

fn rewrite(file: &mut File, contents: &[u8]) -> std::io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(contents)
}

fn cargo_variable(name: &str) -> std::ffi::OsString {
    std::env::var_os(name).unwrap_or_else(|| {
        panic!(
            "`{name}` is not set: waterui-testing resolves the package under test's declared \
             fonts through `cargo metadata`, so the harness must run under `cargo test` or \
             `cargo nextest`"
        )
    })
}

fn cache_io_failed(path: &Path, action: &str, error: &std::io::Error) -> ! {
    panic!(
        "cannot {action} the declared font cache `{}`: {error}",
        path.display()
    )
}
