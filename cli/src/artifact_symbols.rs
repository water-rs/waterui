//! Symbols embedded in compiled Rust artifacts.
//!
//! Procedural macros report metadata to the CLI through `waterui_meta_*`
//! records in the `.wmeta` metadata directory, and through plain exports such
//! as `waterui_preview_*`. The records are read from the directory section,
//! the exports by enumerating the artifact's symbol table; both are ground
//! truth, because macros, cfgs, and generics are already resolved in them. The artifact is the one the target build just
//! produced — [`crate::build::BuiltTarget::app_library`] — never a separate
//! host-profile compile.

use std::collections::BTreeSet;
use std::path::Path;

use color_eyre::eyre::{Context as _, Result, bail};
use object::read::archive::ArchiveFile;
use object::{File, FileKind, Object, ObjectSection, ObjectSymbol};
use waterui_assets_planner::BundleMountMeta;
use waterui_meta::{DIR_SECTION, dir_records};

/// Symbols of one compiled Rust artifact: an rlib/staticlib archive (every
/// member parsed) or a single object/dylib.
pub struct ArtifactSymbols {
    data: Vec<u8>,
    has_symbols: bool,
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
            has_symbols: false,
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
        let mut has_symbols = false;
        let mut objects = 0usize;
        for_each_object(&data, |file| {
            objects += 1;
            has_symbols |= file.symbols().next().is_some();
        });
        if objects == 0 {
            bail!("{} is not a recognized object or archive", path.display());
        }
        Ok(Self { data, has_symbols })
    }

    /// Whether the artifact carried no symbols at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        !self.has_symbols
    }

    /// Leaf names the artifact carries that start with `prefix`:
    /// demangled symbol leaves plus the names of `name`, NUL, `payload`,
    /// NUL records parked in the `.wmeta` (`__wmeta` on Mach-O) metadata
    /// directory — which answer on a linked image that keeps no symbol
    /// table at all, like a PE binary. Deduplicated and sorted.
    ///
    /// Names are demangled with the `{:#}` alternate form so trailing hashes
    /// are dropped; un-mangled names pass through unchanged. Only a name
    /// whose raw form contains `prefix` is demangled: both Rust manglings
    /// spell an identifier's ASCII characters verbatim — a punycoded one
    /// keeps its ASCII code points in order ahead of the encoded rest — so
    /// a leaf that starts with an ASCII `prefix` carries it in the raw
    /// name too.
    #[must_use]
    pub fn leaves_with_prefix(&self, prefix: &str) -> Vec<String> {
        let mut leaves = BTreeSet::new();
        for_each_object(&self.data, |file| {
            leaves.extend(
                file.symbols()
                    .filter_map(|symbol| symbol.name().ok())
                    .filter(|raw| raw.contains(prefix))
                    .filter_map(|raw| {
                        leaf_of(&demangled_name(raw))
                            .filter(|leaf| leaf.starts_with(prefix))
                            .map(str::to_owned)
                    }),
            );
            for_each_dir_section(&file, |data| {
                leaves.extend(
                    dir_records(data)
                        .filter_map(|record| std::str::from_utf8(record.name).ok())
                        .filter(|name| name.starts_with(prefix))
                        .map(str::to_owned),
                );
            });
        });
        leaves.into_iter().collect()
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
        let mut meta = BundleMountMeta::from_payload(&self.meta_record(leaf)?)?;
        meta.path = dunce::simplified(&meta.path).to_path_buf();
        meta.project = meta
            .project
            .map(|project| dunce::simplified(&project).to_path_buf());
        Ok(meta)
    }

    /// The payload of the metadata-directory record named `leaf`: a
    /// `name`, NUL, `payload`, NUL record parked in the `.wmeta` (`__wmeta`
    /// on Mach-O) section. A record names itself, so it resolves on a
    /// linked image that carries no symbol table.
    ///
    /// # Errors
    /// Returns an error when no record has that name or when two records
    /// with that name carry different payloads.
    pub fn meta_record(&self, leaf: &str) -> Result<Vec<u8>> {
        let mut payloads = BTreeSet::new();
        for_each_object(&self.data, |file| {
            for_each_dir_section(&file, |data| {
                payloads.extend(
                    dir_records(data)
                        .filter(|record| record.name == leaf.as_bytes())
                        .map(|record| record.payload.to_vec()),
                );
            });
        });
        let mut payloads = payloads.into_iter();
        let Some(payload) = payloads.next() else {
            bail!("no metadata record named `{leaf}` in the artifact");
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
            .field("has_symbols", &self.has_symbols)
            .finish_non_exhaustive()
    }
}

/// Invoke `f` on the bytes of every metadata-directory section `file`
/// carries — `.wmeta` on COFF and ELF, `__wmeta` on Mach-O.
fn for_each_dir_section<'a>(file: &File<'a>, mut f: impl FnMut(&'a [u8])) {
    for section in file.sections() {
        let Ok(name) = section.name() else { continue };
        if !matches!(name, DIR_SECTION | "__wmeta") {
            continue;
        }
        let Ok(data) = section.data() else { continue };
        f(data);
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

    /// The `cargo build` artifact stream for `fixture`'s own manifest — the
    /// same `compiler-artifact` scan [`crate::build::compiler_artifacts`]
    /// applies to a real target build, so tests read exactly the artifacts a
    /// `water` build produces.
    fn built_fixture(
        host: &crate::toolchain::Host,
        fixture: &Path,
        extra_args: &[&str],
    ) -> std::io::Result<Vec<cargo_metadata::Artifact>> {
        let output = host
            .std_command("cargo")?
            .args(["build", "--message-format=json-render-diagnostics"])
            .args(extra_args)
            .current_dir(fixture)
            .output()
            .expect("cargo build runs");
        assert!(
            output.status.success(),
            "the fixture build must succeed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        crate::build::compiler_artifacts(&output.stdout)
            .map_err(|error| std::io::Error::other(format!("{error}")))
    }

    /// The library artifact `cargo build --lib` reported for `fixture`'s own
    /// manifest, selected the way [`crate::build::app_library_artifact`]
    /// selects it for a real target build.
    fn built_lib_fixture(fixture: &Path, extra_args: &[&str]) -> std::io::Result<PathBuf> {
        let mut args = Vec::with_capacity(extra_args.len() + 1);
        args.push("--lib");
        args.extend_from_slice(extra_args);
        let artifacts = built_fixture(
            &crate::toolchain::Host::current().without_cargo_target_dir(),
            fixture,
            &args,
        )?;
        Ok(
            crate::build::app_library_artifact(&artifacts, &fixture.join("Cargo.toml"))
                .expect("the artifact scan parses")
                .expect("cargo reported a library artifact for the fixture"),
        )
    }

    #[test]
    fn reads_meta_records_and_exports_from_built_rlib() -> std::io::Result<()> {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meta_static");
        let rlib = built_lib_fixture(&fixture, &[])?;
        let symbols = ArtifactSymbols::read(&rlib).expect("rlib should parse");
        let previews = symbols.leaves_with_prefix("waterui_preview_");
        assert_eq!(previews, ["waterui_preview_meta_static_probe"]);
        assert_eq!(
            symbols
                .meta_record("waterui_meta_test_dir_probe")
                .expect("the dir record should be present"),
            b"dir probe",
            "a `.wmeta` record answers with its payload field, not the record"
        );
        Ok(())
    }

    /// The same fixture built for `wasm32-unknown-unknown`: an rlib there is
    /// an archive of wasm objects, which `for_each_object` must parse so the
    /// `waterui_*` names still enumerate. Static payload bytes live in wasm
    /// data segments the `object` reader does not expose back to a section
    /// index, so this format covers enumeration only.
    #[test]
    fn reads_meta_static_names_from_wasm_rlib() -> std::io::Result<()> {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meta_static");
        let rlib = built_lib_fixture(&fixture, &["--target", "wasm32-unknown-unknown"])?;
        let symbols = ArtifactSymbols::read(&rlib).expect("wasm rlib should parse");
        let previews = symbols.leaves_with_prefix("waterui_preview_");
        assert_eq!(previews, ["waterui_preview_meta_static_probe"]);
        assert_eq!(
            symbols.leaves_with_prefix("waterui_meta_"),
            ["waterui_meta_test_dir_probe"],
            "the wasm rlib must enumerate the meta static names"
        );
        Ok(())
    }

    /// `#[used]` is linker-retained (`no_dead_strip` on Mach-O), so every
    /// `waterui_meta_*` static is `#[cfg(debug_assertions)]`: a release rlib
    /// must carry none.
    #[test]
    fn release_rlib_carries_no_meta_statics() -> std::io::Result<()> {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meta_static");
        let rlib = built_lib_fixture(&fixture, &["--release"])?;
        let symbols = ArtifactSymbols::read(&rlib).expect("release rlib should parse");
        assert!(
            symbols.leaves_with_prefix("waterui_meta_").is_empty(),
            "a release rlib must not carry waterui_meta_* statics"
        );
        Ok(())
    }

    /// The fixture's rlib, selected by target name and kind — never the
    /// `waterui-meta` path dependency's.
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    fn retention_lib(artifacts: &[cargo_metadata::Artifact]) -> PathBuf {
        artifacts
            .iter()
            .find(|artifact| {
                artifact.target.name == "meta_retention"
                    && artifact.target.kind.iter().any(|kind| {
                        matches!(
                            kind,
                            cargo_metadata::TargetKind::Lib | cargo_metadata::TargetKind::RLib
                        )
                    })
            })
            .and_then(|artifact| {
                artifact
                    .filenames
                    .iter()
                    .find(|path| path.extension() == Some("rlib"))
            })
            .map(|path| path.as_std_path().to_path_buf())
            .expect("cargo reported the fixture rlib")
    }

    /// The fixture's linked binary, selected by target name and kind.
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    fn retention_bin(artifacts: &[cargo_metadata::Artifact]) -> PathBuf {
        artifacts
            .iter()
            .find(|artifact| {
                artifact.target.name == "meta-retention"
                    && artifact
                        .target
                        .kind
                        .iter()
                        .any(|kind| matches!(kind, cargo_metadata::TargetKind::Bin))
            })
            .and_then(|artifact| artifact.executable.clone())
            .map(Into::into)
            .expect("cargo reported the fixture binary")
    }

    /// Stages `tests/fixtures/meta_retention/src/` into `fixture` and writes
    /// its manifest: one package, the checkout's `utils/meta` by path, one
    /// codegen unit in both profiles so linking the marker pulls the object
    /// the records live in.
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    fn stage_retention_fixture(cli_dir: &Path, fixture: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(fixture.join("src"))?;
        for source in ["lib.rs", "main.rs"] {
            std::fs::copy(
                cli_dir
                    .join("tests/fixtures/meta_retention/src")
                    .join(source),
                fixture.join("src").join(source),
            )?;
        }

        let mut package = cargo_toml::Package::new(
            "meta-retention".to_string(),
            "0.0.0".parse().expect("fixture version"),
        );
        package.edition = cargo_toml::Inheritable::Set(cargo_toml::Edition::E2024);
        let mut manifest = cargo_toml::Manifest::<()> {
            package: Some(package),
            ..Default::default()
        };
        manifest.dependencies.insert(
            "waterui-meta".to_string(),
            cargo_toml::Dependency::Detailed(Box::new(cargo_toml::DependencyDetail {
                path: Some(
                    cli_dir
                        .join("../utils/meta")
                        .to_str()
                        .expect("the meta crate path is UTF-8")
                        .to_string(),
                ),
                ..Default::default()
            })),
        );
        let single_cgu = cargo_toml::Profile {
            codegen_units: Some(1),
            ..Default::default()
        };
        manifest.profile.dev = Some(single_cgu.clone());
        manifest.profile.release = Some(single_cgu);
        std::fs::write(
            fixture.join("Cargo.toml"),
            toml::to_string(&manifest)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
        )
    }

    /// The records the shared `waterui_meta::dir_static!` emits stay in the
    /// rlib in every profile — the CLI reads them there — while the
    /// `__DWARF,__wmeta` placement lets `ld` strip them from a release
    /// binary: debug's `__DATA,__wmeta` keeps them in the linked image.
    ///
    /// Runs its own `cargo build` twice against fixture sources, so it is
    /// ignored for the suite and selected on an Apple aarch64 host.
    #[cfg(all(target_vendor = "apple", target_arch = "aarch64"))]
    #[ignore = "builds the fixture for debug and release"]
    #[test]
    fn apple_release_metadata_retention() -> std::io::Result<()> {
        const RECORDS: [(&str, &[u8]); 3] = [
            ("waterui_meta_retention_root", b"root record"),
            ("waterui_meta_retention_private", b"private module record"),
            ("waterui_meta_retention_anon", b"anonymous const record"),
        ];

        let cli_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let staging = tempfile::tempdir()?;
        let fixture = staging.path().join("meta-retention");
        stage_retention_fixture(cli_dir, &fixture)?;

        // A persistent target under the worktree lets a nextest retry and a
        // later run resume the two fixture builds rather than recompile.
        let target_dir = cli_dir
            .parent()
            .expect("cli has a worktree parent")
            .join("target/meta-retention");
        std::fs::create_dir_all(&target_dir)?;
        let host = crate::toolchain::Host::current()
            .with_env("CARGO_TARGET_DIR", &target_dir)
            .with_env("CARGO_NET_OFFLINE", "true");

        let debug = built_fixture(&host, &fixture, &["--offline"])?;
        let release = built_fixture(&host, &fixture, &["--offline", "--release"])?;

        for archive in [retention_lib(&debug), retention_lib(&release)] {
            let symbols = ArtifactSymbols::read(&archive).expect("fixture rlib parses");
            for (name, payload) in RECORDS {
                assert_eq!(
                    symbols
                        .meta_record(name)
                        .unwrap_or_else(|_| panic!("{name} in {}", archive.display())),
                    payload,
                    "{name} in {}",
                    archive.display(),
                );
            }
        }
        let debug_symbols =
            ArtifactSymbols::read(&retention_bin(&debug)).expect("the debug image parses");
        for (name, payload) in RECORDS {
            assert_eq!(
                debug_symbols
                    .meta_record(name)
                    .unwrap_or_else(|_| panic!("{name} in the debug image")),
                payload,
                "{name} in the debug image",
            );
        }

        let release_image = std::fs::read(retention_bin(&release))?;
        let object = File::parse(&*release_image).expect("the release image parses");
        assert!(
            object
                .sections()
                .all(|section| section.name() != Ok("__wmeta")),
            "ld stripped __DWARF,__wmeta from the release binary"
        );
        Ok(())
    }

    /// The `web_meta` fixture staged against the pinned framework: its sources
    /// copied beside a manifest whose `waterui` dependency resolves the
    /// enclosing checkout by path, so the revision lives in one place.
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
    /// under this one's build.
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
        // `crate/` itself is wiped too — its `Cargo.lock` is re-seeded from
        // the checkout's below rather than kept, so a lock the first build
        // resolved cannot pin the checkout's git-pinned split crates to a
        // stale `dev` commit (#3577).
        if staged.is_dir() {
            std::fs::remove_dir_all(&staged).expect("the staged crate is removable");
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
        // The fixture is built with plain `cargo`, so the deployment targets
        // its `Water.toml` declares reach the build the way they reach a
        // project's own `cargo` commands: through the generated
        // `.cargo/config.toml`.
        let declared = crate::project::Manifest::parse(
            &std::fs::read_to_string(staged.join("Water.toml")).expect("the fixture Water.toml"),
        )
        .expect("the fixture Water.toml parses")
        .apple
        .deployment_targets;
        let cargo_config = futures_lite::future::block_on(async {
            crate::framework::ResolvedFramework::for_local_checkout(
                &crate::toolchain::Host::current(),
                &crate::pinned_framework::checkout(),
            )
            .await
            .expect("the pinned framework resolves")
            .cargo_config(&staged, &declared)
            .await
            .expect("the fixture's deployment targets resolve")
        });
        std::fs::create_dir_all(staged.join(".cargo")).expect("the cargo config directory");
        std::fs::write(staged.join(".cargo/config.toml"), cargo_config.to_string())
            .expect("the cargo config is written");
        // The staged lock starts from the checkout's — the fixture build
        // runs without `--locked`, so Cargo prunes it to the fixture's graph
        // while keeping every version the checkout records.
        std::fs::copy(
            crate::pinned_framework::checkout().join("Cargo.lock"),
            staged.join("Cargo.lock"),
        )
        .expect("the fixture lock seeds from the checkout's");
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
            patch: toml::Table,
            profile: Profile,
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
        // The `[patch]` table the local-checkout resolver computes for the
        // pinned framework — its root `[patch.crates-io]` plus every
        // framework member — so the fixture tracks the selected revision's
        // dependency graph instead of a hand-maintained list that drifts
        // stale (#1638). An extracted crate the facade links from crates.io
        // (e.g. `waterui-image`) otherwise resolves registry copies of
        // framework crates, and every `View` becomes a different type on
        // either side.
        let patches = crate::framework::checkout_patch_tables(&checkout)
            .expect("the framework patch set resolves");
        let patch = toml::Value::try_from(&patches)
            .expect("the patch set serializes")
            .as_table()
            .expect("a patch set is a table")
            .clone();
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
                    path: checkout.to_string_lossy().into_owned(),
                    default_features: false,
                    features: vec!["webview", "assets"],
                },
            ))
            .collect(),
            patch,
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
    fn reads_include_web_mount_meta_from_built_rlib() -> std::io::Result<()> {
        futures_lite::future::block_on(async {
            let (fixture, _restage_guard) = web_meta_fixture();
            let project = fixture.join("crate");
            let rlib = built_lib_fixture(&project, &[])?;
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
            Ok(())
        })
    }
}
