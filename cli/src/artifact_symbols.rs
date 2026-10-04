//! Symbols embedded in compiled Rust artifacts.
//!
//! Procedural macros report metadata to the CLI through `#[used] static` items
//! whose mangled names carry a `waterui_meta_*` leaf, and through plain exports
//! such as `waterui_preview_*`. Both are recovered here by enumerating the
//! artifact's symbol table, which is ground truth: macros, cfgs, and generics
//! are already resolved in it. The artifact is the one the target build just
//! produced — [`crate::build::BuiltTarget::app_library`] — never a separate
//! host-profile compile.

use std::collections::BTreeSet;
use std::path::Path;

use color_eyre::eyre::{Context as _, Result, bail};
use object::read::archive::ArchiveFile;
use object::{File, FileKind, Object, ObjectSection, ObjectSymbol};
use waterui_assets_planner::BundleMountMeta;

/// Symbols of one compiled Rust artifact: an rlib/staticlib archive (every
/// member parsed) or a single object/dylib.
pub struct ArtifactSymbols {
    data: Vec<u8>,
    names: Vec<String>,
}

impl ArtifactSymbols {
    /// An artifact that carries no symbols — the reading built no library
    /// artifact for the crate, so every lookup reports nothing found.
    ///
    /// The argument-free counterpart of [`Self::read`]; used by callers whose
    /// build legitimately produced no readable artifact.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            data: Vec::new(),
            names: Vec::new(),
        }
    }

    /// Read every symbol of the artifact at `path`.
    ///
    /// Archive members that do not parse as object files (for example
    /// `lib.rmeta`) are skipped silently.
    ///
    /// # Errors
    /// Returns an error when the file cannot be read or no part of it parses
    /// as a recognized object or archive.
    pub fn read(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)
            .wrap_err_with(|| format!("failed to read artifact {}", path.display()))?;
        let mut names = Vec::new();
        let mut objects = 0usize;
        for_each_object(&data, |file| {
            objects += 1;
            names.extend(
                file.symbols()
                    .filter_map(|symbol| symbol.name().ok())
                    .map(demangled_name),
            );
        });
        if objects == 0 {
            bail!("{} is not a recognized object or archive", path.display());
        }
        Ok(Self { data, names })
    }

    /// Whether the artifact carried no symbols at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Demangled symbol names.
    ///
    /// Names are demangled with the `{:#}` alternate form so trailing hashes
    /// are dropped; un-mangled names pass through unchanged.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// Leaf path segments (text after the last `::`, the whole name when it
    /// has none) that start with `prefix`. Deduplicated and sorted.
    pub fn leaves_with_prefix(&self, prefix: &str) -> Vec<String> {
        self.names()
            .filter_map(leaf_of)
            .filter(|leaf| leaf.starts_with(prefix))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// The bundle-mount metadata a `waterui_meta_bundle_*` static carries.
    ///
    /// The macro records the mount and project roots with
    /// `std::fs::canonicalize`, which on Windows spells them as verbatim
    /// `\\?\C:\...` paths. Those are valid for the standard library but not
    /// for what the CLI hands them to — a frontend package manager's working
    /// directory, Xcode and Gradle inputs, equality against paths the CLI
    /// resolved itself — so both roots are simplified to their ordinary
    /// spelling here, at the one place the payload enters the CLI.
    ///
    /// # Errors
    /// Returns an error when the static is missing or its payload does not
    /// decode.
    pub fn bundle_mount_meta(&self, leaf: &str) -> Result<BundleMountMeta> {
        let mut meta = BundleMountMeta::from_payload(&self.static_bytes(leaf)?)?;
        meta.path = dunce::simplified(&meta.path).to_path_buf();
        meta.project = meta
            .project
            .map(|project| dunce::simplified(&project).to_path_buf());
        Ok(meta)
    }

    /// Bytes of a `#[used] static`.
    ///
    /// Locates the symbol whose demangled leaf equals `leaf`, reads its
    /// section data from the symbol address, and cuts at the first NUL byte:
    /// payloads are NUL-terminated by contract because Mach-O symbols carry
    /// no size.
    ///
    /// # Errors
    /// Returns an error when no symbol has that leaf or when two different
    /// definitions exist.
    pub fn static_bytes(&self, leaf: &str) -> Result<Vec<u8>> {
        let mut payloads = BTreeSet::new();
        for_each_object(&self.data, |file| {
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
                let Ok(data) = section.data() else { continue };
                let Ok(offset) = usize::try_from(symbol.address().wrapping_sub(section.address()))
                else {
                    continue;
                };
                if let Some(bytes) = data.get(offset..) {
                    payloads.insert(
                        bytes
                            .split(|byte| *byte == 0)
                            .next()
                            .unwrap_or_default()
                            .to_vec(),
                    );
                }
            }
        });
        let mut payloads = payloads.into_iter();
        let Some(payload) = payloads.next() else {
            bail!("no symbol with leaf `{leaf}` carries section data in the artifact");
        };
        if payloads.next().is_some() {
            bail!("artifact defines `{leaf}` more than once with different payloads");
        }
        Ok(payload)
    }
}

impl std::fmt::Debug for ArtifactSymbols {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtifactSymbols")
            .field("symbols", &self.names.len())
            .finish_non_exhaustive()
    }
}

/// Invoke `f` on every object file contained in `data`: each member of an
/// archive, or `data` itself when it is a single object/dylib.
fn for_each_object<'a>(data: &'a [u8], mut f: impl FnMut(File<'a>)) {
    if matches!(FileKind::parse(data), Ok(FileKind::Archive))
        && let Ok(archive) = ArchiveFile::parse(data)
    {
        for member in archive.members().flatten() {
            if member.name() == b"lib.rmeta" {
                continue;
            }
            if let Ok(member_data) = member.data(data)
                && let Ok(file) = File::parse(member_data)
            {
                f(file);
            }
        }
        return;
    }
    if let Ok(file) = File::parse(data) {
        f(file);
    }
}

/// Demangle a raw symbol name, dropping the trailing disambiguation hash.
///
/// Mach-O prepends `_` to every external symbol; the legacy/v0 manglings
/// absorb it during demangling, but `#[no_mangle]` names keep it, so it is
/// stripped afterwards.
fn demangled_name(raw: &str) -> String {
    let demangled = format!("{:#}", rustc_demangle::demangle(raw));
    demangled
        .strip_prefix('_')
        .map_or_else(|| demangled.clone(), str::to_owned)
}

/// The leaf segment of a possibly-qualified demangled name.
fn leaf_of(name: &str) -> Option<&str> {
    name.rsplit("::").next().filter(|leaf| !leaf.is_empty())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    /// The library artifact `cargo build --lib` reported for `fixture`'s own
    /// manifest — the same `compiler-artifact` stream scan
    /// [`crate::build::app_library_artifact`] applies to a real target build,
    /// so these tests read exactly the artifact a `water` build produces.
    fn built_lib_fixture(fixture: &Path, extra_args: &[&str]) -> PathBuf {
        let output = std::process::Command::new("cargo")
            .args(["build", "--lib", "--message-format=json-render-diagnostics"])
            .args(extra_args)
            .current_dir(fixture)
            .output()
            .expect("cargo build runs");
        assert!(
            output.status.success(),
            "the fixture build must succeed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        crate::build::app_library_artifact(&output.stdout, &fixture.join("Cargo.toml"))
            .expect("the artifact scan parses")
            .expect("cargo reported a library artifact for the fixture")
    }

    #[test]
    fn reads_meta_statics_and_exports_from_built_rlib() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meta_static");
        let rlib = built_lib_fixture(&fixture, &[]);
        let symbols = ArtifactSymbols::read(&rlib).expect("rlib should parse");
        let previews = symbols.leaves_with_prefix("waterui_preview_");
        assert_eq!(previews, ["waterui_preview_meta_static_probe"]);
        assert_eq!(
            symbols
                .static_bytes("waterui_meta_test_probe")
                .expect("static should be present"),
            b"hello"
        );
    }

    /// The same fixture built for `wasm32-unknown-unknown`: an rlib there is
    /// an archive of wasm objects, which `for_each_object` must parse so the
    /// `waterui_*` names still enumerate. Static payload bytes live in wasm
    /// data segments the `object` reader does not expose back to a section
    /// index, so this format covers enumeration only.
    #[test]
    fn reads_meta_static_names_from_wasm_rlib() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meta_static");
        let rlib = built_lib_fixture(&fixture, &["--target", "wasm32-unknown-unknown"]);
        let symbols = ArtifactSymbols::read(&rlib).expect("wasm rlib should parse");
        let previews = symbols.leaves_with_prefix("waterui_preview_");
        assert_eq!(previews, ["waterui_preview_meta_static_probe"]);
        assert!(
            symbols
                .names()
                .any(|name| name.ends_with("waterui_meta_test_probe")),
            "the wasm rlib must enumerate the meta static name"
        );
    }

    /// `#[used]` is linker-retained (`no_dead_strip` on Mach-O), so every
    /// `waterui_meta_*` static is `#[cfg(debug_assertions)]`: a release rlib
    /// must carry none.
    #[test]
    fn release_rlib_carries_no_meta_statics() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meta_static");
        let status = std::process::Command::new("cargo")
            .args(["build", "--lib", "--release"])
            .current_dir(&fixture)
            .status()
            .expect("cargo build --release runs");
        assert!(status.success(), "the release fixture build must succeed");
        let symbols = ArtifactSymbols::read(&fixture.join("target/release/libmeta_static.rlib"))
            .expect("release rlib should parse");
        assert!(
            symbols.leaves_with_prefix("waterui_meta_").is_empty(),
            "a release rlib must not carry waterui_meta_* statics"
        );
    }

    /// The `web_meta` fixture staged against the pinned framework: its sources
    /// copied beside a manifest whose `waterui` dependency is the git pin this
    /// crate's own manifest carries, so the revision lives in one place.
    ///
    /// The fixture lives under `target/test-fixtures/` rather than a tempdir:
    /// its `cargo build --lib` compiles the pinned framework's graph, which is
    /// the expensive part, and a persistent target dir lets a nextest retry —
    /// and the next cached CI run — resume that compile instead of restarting
    /// it cold every attempt. Sources stage into a wiped `crate/` beside the
    /// persistent `target/` so a file deleted from `tests/fixtures/web_meta`
    /// cannot survive in the staged tree.
    ///
    /// The returned file handle is a held lock covering the fixture for the
    /// whole test: a second caller's restage waits rather than wiping `crate/`
    /// under this one's build. `crate/Cargo.lock` survives the wipe on
    /// purpose — it is a build artifact, not fixture content, and regenerating
    /// it re-resolves the pinned git dependency on every retry.
    fn web_meta_fixture() -> (PathBuf, std::fs::File) {
        let sources = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/web_meta");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-fixtures/web-meta");
        std::fs::create_dir_all(&fixture).expect("the fixture directory is creatable");
        let lock_path = fixture.join(".restage.lock");
        let restage_lock = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .expect("the restage lock opens");
        fs4::FileExt::lock(&restage_lock).expect("the restage lock acquires");

        let staged = fixture.join("crate");
        for entry in std::fs::read_dir(&fixture).expect("the fixture directory is readable") {
            let path = entry.expect("a fixture entry is readable").path();
            if path == fixture.join("target") || path == staged || path == lock_path {
                continue;
            }
            if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            }
            .expect("a stale fixture entry is removable");
        }
        // `crate/` itself is wiped too, except `Cargo.lock` — the one build
        // artifact worth keeping across a restage.
        if staged.is_dir() {
            for entry in std::fs::read_dir(&staged).expect("the staged crate is readable") {
                let path = entry.expect("a staged entry is readable").path();
                if path == staged.join("Cargo.lock") {
                    continue;
                }
                if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                }
                .expect("a stale staged entry is removable");
            }
        }
        fs_extra::dir::copy(
            &sources,
            &staged,
            &fs_extra::dir::CopyOptions::new()
                .content_only(true)
                .overwrite(true),
        )
        .expect("the fixture sources copy");
        std::fs::write(staged.join("Cargo.toml"), web_meta_manifest())
            .expect("the manifest is written");
        (fixture, restage_lock)
    }

    /// The staged fixture's `Cargo.toml`, with the `waterui` dependency
    /// resolved by path to the enclosing workspace — the checkout this crate
    /// lives in, which is the revision its own framework deps compile.
    fn web_meta_manifest() -> String {
        #[derive(serde::Serialize)]
        struct Manifest {
            package: Package,
            workspace: toml::Table,
            dependencies: std::collections::BTreeMap<&'static str, Dependency>,
            patch: Patch,
            profile: Profile,
        }
        #[derive(serde::Serialize)]
        struct Patch {
            #[serde(rename = "crates-io")]
            crates_io: std::collections::BTreeMap<&'static str, PathSource>,
        }
        #[derive(serde::Serialize)]
        struct PathSource {
            path: String,
        }
        #[derive(serde::Serialize)]
        struct Package {
            name: &'static str,
            version: &'static str,
            edition: &'static str,
        }
        #[derive(serde::Serialize)]
        struct Dependency {
            path: String,
            #[serde(rename = "default-features")]
            default_features: bool,
            features: Vec<&'static str>,
        }
        #[derive(serde::Serialize)]
        struct Profile {
            dev: DevProfile,
        }
        /// The rlib is read for `waterui_meta_*` statics, which live behind
        /// `debug_assertions` — debug info itself buys the test nothing, and
        /// emitting it for the whole framework graph is a real slice of a cold
        /// build's time.
        #[derive(serde::Serialize)]
        struct DevProfile {
            debug: u8,
        }

        let checkout = crate::pinned_framework::checkout();
        let at = |member: &str| PathSource {
            path: checkout.join(member).to_string_lossy().into_owned(),
        };
        // The lean facade: `include_web!` expands against `waterui::webview`
        // and `waterui::Bundle`, nothing else of the framework is needed.
        let manifest = Manifest {
            package: Package {
                name: "web-meta",
                version: "0.0.0",
                edition: "2024",
            },
            workspace: toml::Table::new(),
            dependencies: std::iter::once((
                "waterui",
                Dependency {
                    path: at("").path,
                    default_features: false,
                    features: vec!["webview", "assets"],
                },
            ))
            .collect(),
            // The extracted `waterui-image` the facade links from crates.io
            // depends on the registry copies of these crates; without the
            // redirect the graph carries two of each and every `View` is a
            // different type on either side.
            patch: Patch {
                crates_io: [
                    ("waterui-core", "core"),
                    ("waterui-graphics", "components/visual/graphics"),
                    ("waterui-layout", "components/foundation/layout"),
                    ("waterui-macros", "macros"),
                ]
                .into_iter()
                .map(|(name, member)| (name, at(member)))
                .collect(),
            },
            profile: Profile {
                dev: DevProfile { debug: 0 },
            },
        };
        toml::to_string(&manifest).expect("the manifest serializes")
    }

    /// `include_web!` is the one web mount an application declares; its
    /// metadata must reach the CLI through the same `waterui_meta_bundle_*`
    /// channel a plain `include_bundle!` uses, carrying the frontend project
    /// root so `water run` knows what to build (#587). The macro expands
    /// against the `waterui` facade, which this crate does not link, so the
    /// fixture builds the enclosing workspace's facade — a cold framework
    /// compile that belongs to the nightly job.
    #[test]
    #[ignore = "builds the fixture against the enclosing workspace"]
    fn reads_include_web_mount_meta_from_built_rlib() {
        futures_lite::future::block_on(async {
            let (fixture, _restage_guard) = web_meta_fixture();
            let project = fixture.join("crate");
            let rlib = built_lib_fixture(&project, &[]);
            let symbols = ArtifactSymbols::read(&rlib).expect("rlib should parse");
            let meta = symbols
                .bundle_mount_meta("waterui_meta_bundle_web")
                .expect("payload should decode as BundleMountMeta");
            assert_eq!(meta.mount, "web");
            assert!(
                meta.path.ends_with("dist"),
                "the default out dir is dist: {}",
                meta.path.display()
            );
            assert_eq!(
                meta.project.as_deref(),
                Some(
                    dunce::canonicalize(project.join("web"))
                        .as_deref()
                        .expect("web root")
                )
            );
        });
    }
}
