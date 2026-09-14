//! Symbols embedded in compiled Rust artifacts.
//!
//! Procedural macros report metadata to the CLI through `#[used] static` items
//! whose mangled names carry a `waterui_meta_*` leaf, and through plain exports
//! such as `waterui_preview_*`. Both are recovered here by enumerating the
//! artifact's symbol table, which is ground truth: macros, cfgs, and generics
//! are already resolved in it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use cargo_metadata::{Message, TargetKind};
use color_eyre::eyre::{Context as _, Result, bail};
use object::read::archive::ArchiveFile;
use object::{File, FileKind, Object, ObjectSection, ObjectSymbol};

/// Symbols of one compiled Rust artifact: an rlib/staticlib archive (every
/// member parsed) or a single object/dylib.
pub struct ArtifactSymbols {
    data: Vec<u8>,
    names: Vec<String>,
}

impl ArtifactSymbols {
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

/// Build the project's library crate for the host (debug) and return the path
/// of the produced `.rlib`.
///
/// Runs `cargo build --lib --message-format=json-render-diagnostics` with
/// `project_path` as the working directory. `sccache_path`, when given, is
/// installed as `RUSTC_WRAPPER` through the same helper every other CLI build
/// uses.
///
/// # Errors
/// Returns an error when cargo fails or the project produces no rlib.
pub async fn build_host_rlib(project_path: &Path, sccache_path: Option<&Path>) -> Result<PathBuf> {
    let manifest_path = dunce::canonicalize(project_path.join("Cargo.toml"))
        .wrap_err_with(|| format!("no Cargo.toml under {}", project_path.display()))?;

    let mut cargo = smol::process::Command::new("cargo");
    cargo
        .args(["build", "--lib", "--message-format=json-render-diagnostics"])
        .current_dir(project_path)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(sccache_path) = sccache_path {
        crate::toolchain::sccache::configure_compilation_cache(&mut cargo, sccache_path);
    }
    let output = cargo
        .output()
        .await
        .wrap_err("failed to execute `cargo build --lib`")?;
    if !output.status.success() {
        bail!(
            "`cargo build --lib` failed in {}:\n{}",
            project_path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    for message in Message::parse_stream(output.stdout.as_slice()) {
        let Message::CompilerArtifact(artifact) =
            message.wrap_err("failed to parse cargo build message")?
        else {
            continue;
        };
        if artifact.manifest_path.as_std_path() != manifest_path
            || !artifact
                .target
                .kind
                .iter()
                .any(|kind| matches!(kind, TargetKind::Lib | TargetKind::RLib))
        {
            continue;
        }
        if let Some(rlib) = artifact.filenames.iter().find(|filename| {
            filename
                .as_std_path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"))
        }) {
            return Ok(rlib.clone().into_std_path_buf());
        }
    }
    bail!(
        "`cargo build --lib` produced no rlib for {}",
        manifest_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_meta_statics_and_exports_from_built_rlib() {
        futures_lite::future::block_on(async {
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meta_static");
            let rlib = build_host_rlib(&fixture, None)
                .await
                .expect("fixture crate should build");
            let symbols = ArtifactSymbols::read(&rlib).expect("rlib should parse");
            let previews = symbols.leaves_with_prefix("waterui_preview_");
            assert_eq!(previews, ["waterui_preview_meta_static_probe"]);
            assert_eq!(
                symbols
                    .static_bytes("waterui_meta_test_probe")
                    .expect("static should be present"),
                b"hello"
            );
        });
    }

    /// `#[used]` is linker-retained (`no_dead_strip` on Mach-O), so every
    /// `waterui_meta_*` static is `#[cfg(debug_assertions)]`: a release rlib
    /// must carry none. Discovery never reads the target build anyway — the
    /// CLI builds a dev-profile host rlib.
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
}
