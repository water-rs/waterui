//! Builds the WPE bridge and web-process extension from the exact resolved
//! `waterui-browser-wpe` source against its pinned prebuilt `WebKit` SDK.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use askama::Template as _;
use eyre::Context as _;
use futures_util::future::try_join;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::project::ResolvedWpeRuntime;
use crate::toolchain::{Host, Toolchain as _};

/// The freshly built native outputs, installed into an immutable prefix.
pub(super) struct NativeBridge {
    /// The installed prefix holding `lib/libwaterui_wpe.so` and
    /// `lib/waterui-wpe/extensions/waterui_wpe_extension.so`.
    pub prefix: PathBuf,
    /// The SHA-256 of the source bytes the build came from — the value the
    /// `runtime.json` `native_bridge` stamp records.
    pub native_source_sha256: String,
}

/// The SDK entries the source build replaces: the published bridge shared
/// object, the web-process extension and the obsolete bundled header. The
/// SDK stays the engine — its bridge bytes never reach an application.
pub(super) const SDK_NATIVE_EXCLUDES: [&str; 3] = [
    "lib/libwaterui_wpe.so",
    "lib/waterui-wpe/extensions/waterui_wpe_extension.so",
    "include/waterui_wpe.h",
];

const REQUIRED_OUTPUTS: [&str; 2] = [
    "lib/libwaterui_wpe.so",
    "lib/waterui-wpe/extensions/waterui_wpe_extension.so",
];

/// The pkg-config modules a source build resolves; their `.pc` files must
/// exist under the SDK or the configure step could bind a system WPE
/// instead.
const REQUIRED_SDK_MODULES: [&str; 3] = [
    "lib/pkgconfig/wpe-webkit-2.0.pc",
    "lib/pkgconfig/wpe-platform-2.0.pc",
    "lib/pkgconfig/wpe-web-process-extension-2.0.pc",
];

const COMPILER_CONFIG: [&str; 6] = [
    "CC",
    "CFLAGS",
    "CXX",
    "CXXFLAGS",
    "LDFLAGS",
    "CMAKE_TOOLCHAIN_FILE",
];

/// The whole three-step `CMake` build shares one budget — a wedged toolchain
/// cannot stretch it per invocation.
const BUILD_BUDGET: Duration = Duration::from_secs(60);

/// The pkg-config descriptors a relocated SDK can answer for — normalized
/// to `${pcfiledir}`-relative prefixes inside the candidate, never the
/// ready cache or host files.
const SDK_PKGCONFIG_DIRS: [&str; 2] = ["lib/pkgconfig", "share/pkgconfig"];

/// Renders a normalized `.pc` descriptor: `prefix=${pcfiledir}/<parents>`
/// first, then the original text with its own `prefix` line removed and
/// old-root references rebased to `${prefix}` fragments. The prefix leads
/// because it depends on no variable, so every later line may use `${prefix}`.
#[derive(askama::Template)]
#[template(path = "browser_runtime/wpe_pc_descriptor.tpl", escape = "none")]
struct PcDescriptorTemplate<'a> {
    parents: &'a str,
    segments: &'a [String],
    tail: &'a [String],
}

/// All `.pc` files under the SDK's descriptor directories — errors on
/// symlinks (a hostile `.pc` could reach outside the candidate) and on any
/// I/O other than a missing directory.
fn sdk_pc_files(sdk_root: &Path) -> eyre::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for directory in SDK_PKGCONFIG_DIRS {
        let directory = sdk_root.join(directory);
        match std::fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => eyre::bail!("{} is not a directory", directory.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        for entry in walkdir::WalkDir::new(&directory) {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type().is_dir() {
                continue;
            }
            if path.extension().is_none_or(|e| e != "pc") {
                continue;
            }
            if entry.file_type().is_symlink() {
                eyre::bail!(
                    "{} is a symlink; SDK descriptors must be regular files",
                    path.display()
                );
            }
            files.push(path.to_path_buf());
        }
    }
    Ok(files)
}

/// Normalizes every `.pc` inside the SDK extraction candidate so the staged
/// runtime's descriptors resolve relative to wherever the SDK lands —
/// `${pcfiledir}` is pkg-config's own variable for the descriptor's
/// directory, so no `--define-prefix` is needed and host descriptors are
/// never touched.
pub(super) async fn normalize_sdk_descriptors(sdk_candidate: PathBuf) -> eyre::Result<()> {
    smol::unblock(move || {
        for descriptor in sdk_pc_files(&sdk_candidate)? {
            let body = std::fs::read_to_string(&descriptor)?;
            std::fs::write(
                &descriptor,
                normalize_pc_source(&sdk_candidate, &descriptor, &body)?,
            )?;
        }
        Ok(())
    })
    .await
}

/// Rebases absolute `old_root` references on `${prefix}` — only at a whole
/// path: preceded by start, `=`, whitespace or a quote (or a `-I`/`-L`
/// flag), and followed by `/`, whitespace, a quote or end. `/another/old`
/// and `/old-sibling` stay literal. Returns the fragments joined on
/// `${prefix}` by the template.
fn rebase_root_references(text: &str, old_root: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut pending = 0usize;
    let mut offset = 0usize;
    while let Some(index) = text[offset..].find(old_root) {
        let at = offset + index;
        let before_ok = at == 0
            || matches!(
                text.as_bytes()[at - 1],
                b'=' | b' ' | b'\t' | b'\n' | b'"' | b'\''
            )
            || (at >= 2
                && matches!(text.as_bytes()[at - 2], b' ' | b'\t' | b'\n' | b'"' | b'\'')
                && matches!(text.as_bytes()[at - 1], b'I' | b'L'));
        let after = &text[at + old_root.len()..];
        let after_ok = after.is_empty()
            || after.starts_with('/')
            || after.starts_with(char::is_whitespace)
            || after.starts_with('"')
            || after.starts_with('\'');
        if before_ok && after_ok {
            segments.push(text[pending..at].to_owned());
            pending = at + old_root.len();
        }
        offset = at + old_root.len();
    }
    segments.push(text[pending..].to_owned());
    segments
}

/// Normalizes one descriptor's text: the `prefix` line (wherever it sits) is
/// removed and a `${pcfiledir}`-relative declaration is emitted first, since
/// it depends on no variable and later lines may reference `${prefix}`;
/// every absolute reference to the old root elsewhere becomes `${prefix}`.
/// Unknown fields, comments and whitespace are preserved byte-for-byte.
fn normalize_pc_source(sdk_root: &Path, descriptor: &Path, body: &str) -> eyre::Result<String> {
    let mut old_prefix = None;
    let mut line_start = 0usize;
    let mut line_end = 0usize;
    for index in body.match_indices('\n').map(|(i, _)| i).chain([body.len()]) {
        let line = &body[line_start..index];
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "prefix"
            && old_prefix.is_none()
        {
            let value = value.trim();
            if value.is_empty() {
                eyre::bail!("{} declares an empty prefix", descriptor.display());
            }
            old_prefix = Some(value.to_owned());
            line_end = if index < body.len() { index + 1 } else { index };
            break;
        }
        line_start = index + 1;
    }
    let old_prefix =
        old_prefix.ok_or_else(|| eyre::eyre!("{} declares no prefix", descriptor.display()))?;
    let relative_dir = descriptor
        .parent()
        .expect("a descriptor path under the SDK root has a parent directory")
        .strip_prefix(sdk_root)?;
    let parents = std::iter::repeat_n("..", relative_dir.components().count())
        .collect::<Vec<_>>()
        .join("/");
    // The new `${pcfiledir}` prefix goes first: it has no variable
    // dependency, so descriptors may reference `${prefix}` anywhere after.
    let mut rest = String::with_capacity(body.len());
    rest.push_str(&body[..line_start]);
    rest.push_str(&body[line_end..]);
    let segments = rebase_root_references(&rest, &old_prefix);
    Ok(PcDescriptorTemplate {
        parents: &parents,
        segments: &segments,
        tail: if segments.len() > 1 {
            &segments[1..]
        } else {
            &[]
        },
    }
    .render()?)
}

/// The framed digest of every `.pc` under the SDK — a normalization or SDK
/// edit changes it, and the bridge's cache key follows.
fn sdk_pkg_config_sha256_blocking(sdk_root: &Path) -> eyre::Result<String> {
    let mut framed: BTreeMap<String, String> = BTreeMap::new();
    for path in sdk_pc_files(sdk_root)? {
        let relative = path.strip_prefix(sdk_root)?.to_string_lossy().into_owned();
        framed.insert(relative, hex::encode(Sha256::digest(std::fs::read(path)?)));
    }
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&framed)?)))
}

const MANIFEST: &str = "native_bridge.json";

/// The source tree read once, hashed once, and written from the same bytes —
/// the digest covers exactly what `CMake` compiles.
struct Snapshot {
    files: Vec<(PathBuf, Vec<u8>)>,
}

impl Snapshot {
    /// Reads `directory` into memory in one blocking section.
    fn read(directory: &Path) -> eyre::Result<Self> {
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(directory) {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type().is_dir() {
                continue;
            }
            let relative = path.strip_prefix(directory)?.to_path_buf();
            files.push((relative, std::fs::read(path)?));
        }
        files.sort();
        Ok(Self { files })
    }

    /// The structural SHA-256 of the snapshot: an ordered path →
    /// content-hash map serialized before hashing, so path/content
    /// boundaries can never collide.
    fn sha256(&self) -> String {
        let framed: BTreeMap<String, String> = self
            .files
            .iter()
            .map(|(relative, bytes)| {
                (
                    relative.to_string_lossy().into_owned(),
                    hex::encode(Sha256::digest(bytes)),
                )
            })
            .collect();
        hex::encode(Sha256::digest(
            serde_json::to_vec(&framed).expect("an ordered map serializes"),
        ))
    }

    /// Writes the same bytes [`Snapshot::read`] hashed.
    fn write_to(&self, directory: &Path) -> eyre::Result<()> {
        for (relative, bytes) in &self.files {
            let target = directory.join(relative);
            std::fs::create_dir_all(target.parent().expect("a file path has a parent"))?;
            std::fs::write(&target, bytes)?;
        }
        Ok(())
    }
}

/// The cache-key input: source and SDK digests, target, expected ABI and the
/// compiler environment a build differs under — serialized, hashed, hexed,
/// never a path fragment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BuildInput {
    native_source_sha256: String,
    sdk_sha256: String,
    target: String,
    bridge_abi: u32,
    sdk_pkg_config_sha256: String,
    compiler: BTreeMap<String, String>,
}

/// The published cache marker — written only after both outputs exist. The
/// outputs are typed fields, not a map: a missing or empty entry cannot
/// satisfy deserialization, and a cache hit re-verifies both SHA-256s on
/// every reuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NativeBuildManifest {
    inputs: BuildInput,
    outputs: NativeBuildOutputs,
}

/// The two required native outputs' recorded checksums.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NativeBuildOutputs {
    /// The SHA-256 of `lib/libwaterui_wpe.so`.
    bridge_sha256: String,
    /// The SHA-256 of `lib/waterui-wpe/extensions/waterui_wpe_extension.so`.
    extension_sha256: String,
}

fn cache_key(input: &BuildInput) -> eyre::Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(input)?)))
}

/// Builds and installs the bridge and extension from `source`'s native
/// directory against the SDK rooted at `sdk_root`.
///
/// A successful build lands in a content-addressed persistent cache keyed on
/// the source digest, SDK digest, target, expected ABI and compiler
/// configuration. A cache hit re-validates the recorded inputs and every
/// output checksum before reuse — a modified or partial entry errors, it
/// never serves stale bytes. The build publishes into a unique candidate
/// directory and renames it into place only after verification, so a partial
/// build can never occupy the key.
///
/// # Errors
///
/// Returns an error when the source snapshot, configure, build or install
/// step fails, when a required output or SDK module is missing, or when a
/// cached entry fails validation.
pub(super) async fn build(
    host: &Host,
    source: &ResolvedWpeRuntime,
    sdk_root: &Path,
    sdk_sha256: &str,
) -> eyre::Result<NativeBridge> {
    let source_directory = source.native_source().to_path_buf();
    let sdk_directory = sdk_root.to_path_buf();
    let (snapshot, sdk_pkg_config_sha256) = futures_util::future::try_join(
        smol::unblock(move || Snapshot::read(&source_directory)),
        smol::unblock(move || sdk_pkg_config_sha256_blocking(&sdk_directory)),
    )
    .await?;
    let native_source_sha256 = snapshot.sha256();
    let input = BuildInput {
        native_source_sha256: native_source_sha256.clone(),
        sdk_sha256: sdk_sha256.to_owned(),
        target: source.target().to_string(),
        bridge_abi: source.bridge_abi(),
        sdk_pkg_config_sha256,
        compiler: compiler_environment(host),
    };
    let key = cache_key(&input)?;
    let cache_root = host
        .cache_dir()
        .ok_or_else(|| eyre::eyre!("platform does not expose a cache directory"))?
        .join("waterui/browser-runtimes/wpe-source-build");
    let prefix = cache_root.join(&key);

    match smol::fs::metadata(&prefix).await {
        Ok(_) => {
            validate_cached(&prefix, &input).await?;
            return Ok(NativeBridge {
                prefix,
                native_source_sha256,
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    require_sdk_modules(sdk_root).await?;

    // Build into a unique candidate, verify, then rename into place — a
    // partial build can never satisfy a later cache probe.
    let candidate = cache_root.join(format!("{key}.candidate.{}", uuid::Uuid::new_v4()));
    let build_result = async {
        smol::fs::create_dir_all(candidate.join("build")).await?;
        smol::fs::create_dir_all(candidate.join("source")).await?;
        let snapshot_target = candidate.join("source");
        smol::unblock(move || snapshot.write_to(&snapshot_target)).await?;
        cmake_build(
            host,
            source,
            sdk_root,
            &candidate.join("source"),
            &candidate.join("build"),
            &candidate,
        )
        .await?;
        let outputs = verify_outputs(&candidate).await?;
        write_manifest(&candidate, &input, &outputs).await?;
        Ok::<(), eyre::Report>(())
    }
    .await;

    if let Err(error) = build_result {
        let cleanup = smol::fs::remove_dir_all(&candidate).await;
        return Err(match cleanup {
            Ok(()) => error,
            Err(cleanup) => error
                .wrap_err("the failed build's candidate directory could not be removed")
                .wrap_err(cleanup.to_string()),
        });
    }

    publish_candidate(&candidate, &prefix, || validate_cached(&prefix, &input)).await?;
    Ok(NativeBridge {
        prefix,
        native_source_sha256,
    })
}

/// Publishes one complete cache directory, validating a concurrent winner
/// before discarding the candidate owned by this invocation.
pub(super) async fn publish_candidate<F, Fut>(
    candidate: &Path,
    destination: &Path,
    validate_existing: F,
) -> eyre::Result<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = eyre::Result<()>>,
{
    match smol::fs::rename(candidate, destination).await {
        Ok(()) => {}
        Err(error) => {
            // This build's candidate is owned here; always remove it.
            if let Err(cleanup) = smol::fs::remove_dir_all(candidate).await {
                return Err(eyre::eyre!(error).wrap_err(format!(
                    "cannot publish the WPE bridge build cache entry; cleaning its candidate also failed: {cleanup}"
                )));
            }
            // An existing complete entry means a concurrent identical build
            // won — validate it before use, never serve it unread.
            if matches!(
                error.kind(),
                std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::DirectoryNotEmpty
            ) {
                validate_existing().await?;
            } else {
                return Err(eyre::Report::new(error))
                    .wrap_err("cannot publish the WPE bridge build cache entry");
            }
        }
    }
    Ok(())
}

/// A cache entry is only usable when its recorded inputs match this build's
/// and every recorded output still hashes to what the build produced.
async fn validate_cached(prefix: &Path, input: &BuildInput) -> eyre::Result<()> {
    let manifest: NativeBuildManifest = serde_json::from_slice(
        &smol::fs::read(prefix.join(MANIFEST))
            .await
            .wrap_err("the cached WPE bridge build manifest is unreadable")?,
    )
    .wrap_err("the cached WPE bridge build manifest is malformed")?;
    if manifest.inputs != *input {
        eyre::bail!("the cached WPE bridge build was recorded for different inputs");
    }
    let actual = verify_outputs(prefix)
        .await
        .wrap_err("the cached WPE bridge outputs are incomplete")?;
    if actual != manifest.outputs {
        eyre::bail!("the cached WPE bridge outputs no longer match their record");
    }
    Ok(())
}

/// The SDK must carry the pkg-config descriptors the source build resolves —
/// without them `find_package`/`pkg_check_modules` could bind a system WPE.
async fn require_sdk_modules(sdk_root: &Path) -> eyre::Result<()> {
    let check = |module: &'static str| async move {
        match smol::fs::metadata(sdk_root.join(module)).await {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eyre::bail!("the WPE SDK is missing {module}; cannot build the bridge against it")
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        Ok::<(), eyre::Report>(())
    };
    futures_util::try_join!(
        check(REQUIRED_SDK_MODULES[0]),
        check(REQUIRED_SDK_MODULES[1]),
        check(REQUIRED_SDK_MODULES[2])
    )?;
    Ok(())
}

/// The compiler/toolchain variables that change what the build emits.
fn compiler_environment(host: &Host) -> BTreeMap<String, String> {
    COMPILER_CONFIG
        .iter()
        .filter_map(|name| {
            host.env_string(name)
                .map(|value| ((*name).to_owned(), value))
        })
        .collect()
}

/// `pkg-config` resolves the SDK descriptors first, through `--define-prefix`
/// so the relocated prefix works; the host's own `PKG_CONFIG_PATH` tail
/// stays for tools the SDK does not carry.
fn scoped_pkg_config_path(host: &Host, sdk_root: &Path) -> eyre::Result<std::ffi::OsString> {
    let mut paths = vec![
        sdk_root.join("lib/pkgconfig"),
        sdk_root.join("share/pkgconfig"),
    ];
    if let Some(existing) = host.env("PKG_CONFIG_PATH") {
        paths.extend(std::env::split_paths(existing));
    }
    Ok(std::env::join_paths(paths)?)
}

/// Configure, build and install into `candidate`, sharing one time budget
/// across the three invocations.
async fn cmake_build(
    host: &Host,
    source: &ResolvedWpeRuntime,
    sdk_root: &Path,
    snapshot: &Path,
    build_dir: &Path,
    candidate: &Path,
) -> eyre::Result<()> {
    let cmake = crate::toolchain::cmake::Cmake::default();
    if let Err(error) = cmake.check(host).await {
        eyre::bail!(
            "{}",
            crate::toolchain_checks::toolchain_check_message("CMake", &error)
        );
    }
    let cmake = cmake
        .path(host)
        .await
        .wrap_err("CMake passed its toolchain check but has no resolvable path")?;
    let host = host.with_env("PKG_CONFIG_PATH", scoped_pkg_config_path(host, sdk_root)?);
    let started = Instant::now();
    let expected_abi = format!("-DWATER_WPE_EXPECTED_ABI_VERSION={}", source.bridge_abi());
    run_cmake(
        &host,
        &cmake,
        [
            "-S".into(),
            snapshot.as_os_str().to_owned(),
            "-B".into(),
            build_dir.as_os_str().to_owned(),
            "-DCMAKE_BUILD_TYPE=Release".into(),
            "-DPKG_CONFIG_ARGN=--dont-define-prefix".into(),
            expected_abi.into(),
        ],
        "configure",
        started,
    )
    .await?;
    run_cmake(
        &host,
        &cmake,
        ["--build".into(), build_dir.as_os_str().to_owned()],
        "build",
        started,
    )
    .await?;
    run_cmake(
        &host,
        &cmake,
        [
            "--install".into(),
            build_dir.as_os_str().to_owned(),
            "--prefix".into(),
            candidate.as_os_str().to_owned(),
        ],
        "install",
        started,
    )
    .await?;
    Ok(())
}

/// One cmake invocation against the shared budget; stdout and stderr both
/// land in the failure text.
async fn run_cmake(
    host: &Host,
    cmake: &Path,
    args: impl IntoIterator<Item = std::ffi::OsString>,
    step: &str,
    started: Instant,
) -> eyre::Result<()> {
    let remaining = BUILD_BUDGET.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        eyre::bail!("the WPE bridge build exceeded its 60 second budget before {step} ran");
    }
    let output = host
        .output_within(cmake, args, remaining)
        .await
        .map_err(|error| eyre::eyre!("cmake {step} could not run: {error}"))?;
    if !output.status.success() {
        eyre::bail!(
            "cmake {step} failed for the WPE bridge:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Both installed outputs must exist and hash — the bridge alone never
/// passes.
async fn verify_outputs(prefix: &Path) -> eyre::Result<NativeBuildOutputs> {
    let bridge = prefix.join(REQUIRED_OUTPUTS[0]);
    let extension = prefix.join(REQUIRED_OUTPUTS[1]);
    let (bridge_sha, extension_sha) = try_join(
        async {
            if smol::fs::metadata(&bridge).await.is_err() {
                eyre::bail!("the WPE bridge build produced no {}", REQUIRED_OUTPUTS[0]);
            }
            crate::utils::file_sha256(&bridge).await.map_err(Into::into)
        },
        async {
            if smol::fs::metadata(&extension).await.is_err() {
                eyre::bail!("the WPE bridge build produced no {}", REQUIRED_OUTPUTS[1]);
            }
            crate::utils::file_sha256(&extension)
                .await
                .map_err(Into::into)
        },
    )
    .await?;
    Ok(NativeBuildOutputs {
        bridge_sha256: bridge_sha,
        extension_sha256: extension_sha,
    })
}

/// The published marker — written only after both outputs verify, so its
/// presence claims a complete build.
async fn write_manifest(
    prefix: &Path,
    input: &BuildInput,
    outputs: &NativeBuildOutputs,
) -> eyre::Result<()> {
    let bytes = serde_json::to_vec_pretty(&NativeBuildManifest {
        inputs: input.clone(),
        outputs: outputs.clone(),
    })?;
    smol::fs::write(prefix.join(MANIFEST), bytes).await?;
    Ok(())
}

/// Copies the engine-runtime files of an extracted SDK into the application
/// staging directory — `lib/` shared objects and helpers, `libexec/`, engine
/// resources under `share/`, locales, `licenses/`, `runtime.json` and the
/// SBOM — while build inputs (`include/`, `pkgconfig/`, `CMake` packages,
/// static archives, docs) stay behind in the build-host cache.
pub(super) async fn stage_engine_runtime(sdk_root: &Path, destination: &Path) -> eyre::Result<()> {
    let sdk_root = sdk_root.to_path_buf();
    let destination = destination.to_path_buf();
    smol::unblock(move || {
        for entry in walkdir::WalkDir::new(&sdk_root) {
            let entry = entry?;
            let path = entry.path();
            let relative = path.strip_prefix(&sdk_root)?;
            // The host-cache marker is never an application file.
            if relative == Path::new(".extracted") {
                continue;
            }
            // Filter on path components, not a separator-spelling prefix.
            let mut components = relative.components();
            let build_only = matches!(
                (components.next(), components.next()),
                (Some(std::path::Component::Normal(top)), Some(std::path::Component::Normal(next)))
                if (top == "include")
                    || (top == "share"
                        && matches!(next.to_str(), Some("pkgconfig" | "doc")))
                    || (top == "lib"
                        && matches!(next.to_str(), Some("pkgconfig" | "cmake")))
            );
            if build_only
                || relative
                    .extension()
                    .is_some_and(|extension| extension == "a")
            {
                continue;
            }
            let target = destination.join(relative);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&target)?;
                continue;
            }
            std::fs::create_dir_all(target.parent().expect("a staged file has a parent"))?;
            if entry.file_type().is_symlink() {
                // A SONAME alias stays a link — copying it would duplicate the
                // payload it names.
                let link_target = std::fs::read_link(path)?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(&link_target, &target)?;
                #[cfg(windows)]
                if path.is_dir() {
                    std::os::windows::fs::symlink_dir(&link_target, &target)?;
                } else {
                    std::os::windows::fs::symlink_file(&link_target, &target)?;
                }
            } else {
                std::fs::copy(path, &target)?;
            }
        }
        Ok::<_, eyre::Report>(())
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Snapshot) {
        let directory = tempfile::tempdir().expect("snapshot source directory");
        for (relative, bytes) in files {
            let path = directory.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let snapshot = Snapshot::read(directory.path()).expect("snapshot must read");
        (directory, snapshot)
    }

    fn input(source: &str) -> BuildInput {
        BuildInput {
            native_source_sha256: source.to_owned(),
            sdk_sha256: "sdk".to_owned(),
            target: "aarch64-unknown-linux-gnu".to_owned(),
            bridge_abi: 4,
            sdk_pkg_config_sha256: "pc".to_owned(),
            compiler: BTreeMap::new(),
        }
    }

    #[test]
    fn a_snapshot_digests_exactly_the_bytes_it_writes() {
        let (source, snapshot) =
            snapshot(&[("waterui_wpe.c", b"original"), ("CMakeLists.txt", b"cmake")]);
        // Mutating the checkout after the read cannot drift what the build
        // compiles or what the digest reported.
        std::fs::write(source.path().join("waterui_wpe.c"), b"edited").unwrap();
        let digest = snapshot.sha256();
        let staged = tempfile::tempdir().expect("staged snapshot");
        snapshot.write_to(staged.path()).unwrap();
        let reread = Snapshot::read(staged.path()).unwrap();
        assert_eq!(reread.sha256(), digest);
        assert_eq!(
            std::fs::read(staged.path().join("waterui_wpe.c")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn the_digest_frames_path_and_content_boundaries() {
        // Unframed concatenation collides on a+bc == ab+c: the same bytes
        // split across a path/content boundary must not digest alike.
        let (_a, one_file) = snapshot(&[("a", b"bc")]);
        let (_b, two_files) = snapshot(&[("a", b"b"), ("c", b"")]);
        assert_ne!(one_file.sha256(), two_files.sha256());
        // And the path itself must be covered, not only the content.
        let (_c, renamed) = snapshot(&[("ab", b"bc")]);
        assert_ne!(one_file.sha256(), renamed.sha256());
    }

    #[test]
    fn cache_keys_are_hex_safe_against_compiler_values() {
        let mut spaced = input("src");
        spaced
            .compiler
            .insert("CC".to_owned(), "/opt/tool chains/clang".to_owned());
        spaced
            .compiler
            .insert("CFLAGS".to_owned(), "-I/mnt/my sdk/include -O2".to_owned());
        let key = cache_key(&spaced).unwrap();
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(key.len(), 64);
        // A changed compiler input produces a different key.
        let mut other = input("src");
        other.compiler.insert("CC".to_owned(), "clang".to_owned());
        assert_ne!(key, cache_key(&other).unwrap());
    }

    #[test]
    fn inputs_change_the_key() {
        assert_ne!(
            cache_key(&input("a")).unwrap(),
            cache_key(&input("b")).unwrap()
        );
        let mut sdk = input("a");
        sdk.sdk_sha256 = "other-sdk".to_owned();
        assert_ne!(cache_key(&input("a")).unwrap(), cache_key(&sdk).unwrap());
        let mut abi = input("a");
        abi.bridge_abi = 5;
        assert_ne!(cache_key(&input("a")).unwrap(), cache_key(&abi).unwrap());
        let mut target = input("a");
        target.target = "x86_64-unknown-linux-gnu".to_owned();
        assert_ne!(cache_key(&input("a")).unwrap(), cache_key(&target).unwrap());
    }

    #[test]
    fn a_partial_install_fails_output_verification() {
        let prefix = tempfile::tempdir().expect("install prefix");
        std::fs::create_dir_all(prefix.path().join("lib")).unwrap();
        std::fs::write(prefix.path().join("lib/libwaterui_wpe.so"), b"bridge").unwrap();
        // The extension missing must fail even though the bridge exists.
        assert!(smol::block_on(verify_outputs(prefix.path())).is_err());
        std::fs::create_dir_all(prefix.path().join("lib/waterui-wpe/extensions")).unwrap();
        std::fs::write(
            prefix
                .path()
                .join("lib/waterui-wpe/extensions/waterui_wpe_extension.so"),
            b"extension",
        )
        .unwrap();
        smol::block_on(verify_outputs(prefix.path())).expect("both outputs satisfy the check");
    }

    #[test]
    fn a_cached_entry_must_match_inputs_and_output_checksums() {
        let prefix = tempfile::tempdir().expect("cache prefix");
        std::fs::create_dir_all(prefix.path().join("lib/waterui-wpe/extensions")).unwrap();
        std::fs::write(prefix.path().join("lib/libwaterui_wpe.so"), b"bridge").unwrap();
        std::fs::write(
            prefix
                .path()
                .join("lib/waterui-wpe/extensions/waterui_wpe_extension.so"),
            b"extension",
        )
        .unwrap();
        let base = input("src");
        let outputs = smol::block_on(verify_outputs(prefix.path())).unwrap();
        smol::block_on(write_manifest(prefix.path(), &base, &outputs)).unwrap();
        smol::block_on(validate_cached(prefix.path(), &base))
            .expect("an untampered entry validates");

        // A tampered output must fail validation, not serve stale bytes.
        std::fs::write(prefix.path().join("lib/libwaterui_wpe.so"), b"tampered").unwrap();
        assert!(smol::block_on(validate_cached(prefix.path(), &base)).is_err());
        std::fs::write(prefix.path().join("lib/libwaterui_wpe.so"), b"bridge").unwrap();

        // Different recorded inputs must fail too.
        let mut other = input("src");
        other.bridge_abi = 5;
        assert!(smol::block_on(validate_cached(prefix.path(), &other)).is_err());

        // And a missing manifest is not a cache hit at all.
        std::fs::remove_file(prefix.path().join(MANIFEST)).unwrap();
        assert!(!prefix.path().join(MANIFEST).exists());
    }

    #[test]
    fn only_engine_runtime_files_stage_to_the_application() {
        let sdk = tempfile::tempdir().expect("sdk root");
        let staged = tempfile::tempdir().expect("staged runtime");
        for (entry, bytes) in [
            ("lib/libwpewebkit-2.0.so", b"engine" as &[u8]),
            ("lib/pkgconfig/wpe-webkit-2.0.pc", b"build input"),
            ("lib/cmake/wpe/config.cmake", b"build input"),
            ("lib/libwpewebkit-2.0.a", b"static build input"),
            ("include/wpe/webkit.h", b"build input"),
            ("share/doc/wpe/readme.txt", b"build input"),
            ("libexec/WPEWebProcess", b"helper"),
            ("share/locale/en.po", b"locale"),
            ("licenses/lgpl.txt", b"license"),
            ("runtime.json", b"{}"),
            ("sbom.spdx.json", b"{}"),
        ] {
            let path = sdk.path().join(entry);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        smol::block_on(stage_engine_runtime(sdk.path(), staged.path())).unwrap();
        for staged_entry in [
            "lib/libwpewebkit-2.0.so",
            "libexec/WPEWebProcess",
            "share/locale/en.po",
            "licenses/lgpl.txt",
            "runtime.json",
            "sbom.spdx.json",
        ] {
            assert!(
                staged.path().join(staged_entry).is_file(),
                "{staged_entry} must reach the application"
            );
        }
        for excluded in [
            "lib/pkgconfig/wpe-webkit-2.0.pc",
            "lib/cmake/wpe/config.cmake",
            "lib/libwpewebkit-2.0.a",
            "include/wpe/webkit.h",
            "share/doc/wpe/readme.txt",
        ] {
            assert!(
                !staged.path().join(excluded).exists(),
                "build input {excluded} must not ship"
            );
        }
    }

    #[cfg(unix)]
    fn resolved(manifest_directory: &Path, native_source: &Path) -> ResolvedWpeRuntime {
        ResolvedWpeRuntime::testing(
            manifest_directory.to_path_buf(),
            native_source.to_path_buf(),
            "x86_64-unknown-linux-gnu".parse().unwrap(),
        )
    }

    /// A fake `cmake` whose `--install --prefix` materializes the two
    /// outputs; `WATERUI_FAKE_CMAKE_FAIL` forces every step to fail and
    /// `WATERUI_FAKE_CMAKE_OMIT_EXTENSION` skips the extension output.
    #[cfg(unix)]
    fn machine_with_cmake() -> crate::toolchain::testing::TestMachine {
        let machine = crate::toolchain::testing::TestMachine::new();
        machine.install("cmake");
        machine
    }

    #[cfg(unix)]
    fn sdk_with_modules(machine: &crate::toolchain::testing::TestMachine) -> PathBuf {
        let sdk = machine.dir("sdk");
        for module in [
            "wpe-webkit-2.0.pc",
            "wpe-platform-2.0.pc",
            "wpe-web-process-extension-2.0.pc",
        ] {
            machine.file(format!("sdk/lib/pkgconfig/{module}"), "fake pc");
        }
        sdk
    }

    #[cfg(unix)]
    #[test]
    fn a_build_installs_fresh_outputs_and_reuses_the_cache() {
        let machine = machine_with_cmake();
        let log = machine.root().join("commands.log");
        let host = machine.host([("WATERUI_FAKE_LOG", log.as_os_str())]);
        let sdk = sdk_with_modules(&machine);
        let source_dir = machine.dir("package/native");
        machine.file("package/native/waterui_wpe.c", "int main(void){return 0;}");
        let source = resolved(&machine.root().join("package"), &source_dir);

        let first = smol::block_on(build(&host, &source, &sdk, "sdksha"))
            .expect("a cold build must succeed");
        assert!(first.prefix.join("lib/libwaterui_wpe.so").is_file());
        assert!(
            first
                .prefix
                .join("lib/waterui-wpe/extensions/waterui_wpe_extension.so")
                .is_file()
        );

        // The fake saw the exact configure flags, with the SDK's pkgconfig
        // directories ahead of any host tail.
        let log_text = std::fs::read_to_string(&log).unwrap();
        assert!(log_text.contains("-DWATER_WPE_EXPECTED_ABI_VERSION=4"));
        assert!(log_text.contains("-DPKG_CONFIG_ARGN=--dont-define-prefix"));
        assert!(log_text.contains(&sdk.join("lib/pkgconfig").display().to_string()));

        // An unchanged input set serves the cache without running cmake again.
        let lines = log_text.lines().count();
        let second = smol::block_on(build(&host, &source, &sdk, "sdksha")).unwrap();
        assert_eq!(first.prefix, second.prefix);
        assert_eq!(
            std::fs::read_to_string(&log).unwrap().lines().count(),
            lines,
            "a cache hit must not run cmake"
        );

        // A source edit changes the digest and rebuilds into a new entry.
        machine.file("package/native/waterui_wpe.c", "int main(void){return 1;}");
        let third = smol::block_on(build(&host, &source, &sdk, "sdksha")).unwrap();
        assert_ne!(first.prefix, third.prefix);

        // Editing a normalized SDK descriptor changes the build inputs and
        // rebuilds: cmake runs again, into a different entry.
        let lines = std::fs::read_to_string(&log).unwrap().lines().count();
        machine.file("sdk/lib/pkgconfig/wpe-webkit-2.0.pc", "edited pc");
        let fourth = smol::block_on(build(&host, &source, &sdk, "sdksha")).unwrap();
        assert_ne!(third.prefix, fourth.prefix);
        assert!(
            std::fs::read_to_string(&log).unwrap().lines().count() > lines,
            "an edited descriptor must run cmake again"
        );
    }

    #[cfg(unix)]
    /// The real `pkg-config` end-to-end check the `--define-prefix` breakage
    /// demanded: a normalized SDK descriptor resolves to the staged root,
    /// while the host's multiarch descriptor keeps its real `/usr` prefix —
    /// no mangled `lib/include`, no stale absolute SDK path.
    #[test]
    fn normalized_sdk_descriptors_resolve_with_plain_pkg_config() {
        const BUDGET: Duration = Duration::from_secs(5);
        let fixture = tempfile::tempdir().expect("the pkg-config fixture");
        let sdk = fixture.path().join("sdk with spaces");
        let host_root = fixture.path().join("host");
        let sdk_pc = sdk.join("lib/pkgconfig");
        let host_pc = host_root.join("lib/x86_64-linux-gnu/pkgconfig");
        std::fs::create_dir_all(&sdk_pc).unwrap();
        std::fs::create_dir_all(&host_pc).unwrap();
        // The absolute `libdir` sits BEFORE the consumed `prefix` line on
        // purpose: a `${prefix}` reference must still resolve because the
        // normalized descriptor emits its intrinsic prefix first.
        std::fs::write(
            sdk_pc.join("sdk.pc"),
            "libdir=/old/wpe-sdk/lib\nprefix=/old/wpe-sdk\nincludedir=${prefix}/include\nName: sdk\nDescription: SDK\nVersion: 1.0\nRequires: host\nLibs: -L${libdir} -lsdk\nCflags: -I${includedir}/sdk\n",
        )
        .unwrap();
        std::fs::write(
            host_pc.join("host.pc"),
            "prefix=/usr\nlibdir=${prefix}/lib/x86_64-linux-gnu\nincludedir=${prefix}/include\nName: host\nDescription: Host\nVersion: 1.0\nLibs: -L${libdir} -lhost\nCflags: -I${includedir}/host\n",
        )
        .unwrap();
        smol::block_on(normalize_sdk_descriptors(sdk.clone())).unwrap();

        let path = std::env::join_paths([sdk_pc, host_pc]).unwrap();
        let host = Host::current()
            .with_env("PKG_CONFIG_PATH", &path)
            .with_env("PKG_CONFIG_LIBDIR", "")
            .with_env("PKG_CONFIG_SYSROOT_DIR", "");
        let query = |args: &'static [&'static str]| {
            let host = host.clone();
            async move {
                let output = host
                    .output_within("pkg-config", args, BUDGET)
                    .await
                    .expect("pkg-config runs");
                assert!(
                    output.status.success(),
                    "pkg-config {args:?} failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                String::from_utf8(output.stdout).unwrap().trim().to_owned()
            }
        };
        let (prefix_host, prefix_sdk, libdir_sdk, flags) = smol::block_on(async {
            futures_util::join!(
                query(&["--dont-define-prefix", "--variable=prefix", "host"]),
                query(&["--dont-define-prefix", "--variable=prefix", "sdk"]),
                query(&["--dont-define-prefix", "--variable=libdir", "sdk"]),
                query(&[
                    "--dont-define-prefix",
                    "--keep-system-libs",
                    "--keep-system-cflags",
                    "--cflags",
                    "--libs",
                    "sdk",
                ]),
            )
        });
        // `--dont-define-prefix` preserves the host's real prefix; the SDK
        // resolves its own staged root through `${pcfiledir}`.
        assert_eq!(prefix_host, "/usr");
        // `${pcfiledir}` resolves literally — `..` segments and escaped
        // spaces included — so canonicalize before comparing.
        let resolved = |raw: &str| {
            std::path::PathBuf::from(raw.replace("\\ ", " "))
                .canonicalize()
                .unwrap()
        };
        assert_eq!(resolved(&prefix_sdk), sdk.canonicalize().unwrap());
        assert_eq!(
            resolved(&libdir_sdk),
            sdk.canonicalize().unwrap().join("lib")
        );
        assert!(flags.contains("/usr/include/host"), "{flags}");
        assert!(flags.contains("/usr/lib/x86_64-linux-gnu"), "{flags}");
        assert!(!flags.contains("/old/wpe-sdk"), "{flags}");
        assert!(!flags.contains("lib/include"), "{flags}");
    }

    #[test]
    fn a_descriptor_without_prefix_is_rejected() {
        let sdk = tempfile::tempdir().expect("sdk candidate");
        let dir = sdk.path().join("lib/pkgconfig");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.pc"), "Name: a\nVersion: 1\n").unwrap();
        std::fs::write(dir.join("b.pc"), "prefix=\nVersion: 1\n").unwrap();
        for file in ["a.pc", "b.pc"] {
            let body = std::fs::read_to_string(dir.join(file)).unwrap();
            assert!(
                normalize_pc_source(sdk.path(), &dir.join(file), &body).is_err(),
                "{file} must be rejected"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_descriptor_is_rejected() {
        let sdk = tempfile::tempdir().expect("sdk candidate");
        let dir = sdk.path().join("lib/pkgconfig");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", dir.join("hostile.pc")).unwrap();
        assert!(smol::block_on(normalize_sdk_descriptors(sdk.path().to_path_buf())).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_build_publishes_nothing() {
        let machine = machine_with_cmake();
        let host = machine.host([("WATERUI_FAKE_CMAKE_FAIL", "1")]);
        let sdk = sdk_with_modules(&machine);
        let source_dir = machine.dir("package/native");
        machine.file("package/native/waterui_wpe.c", "int main(void){return 0;}");
        let source = resolved(&machine.root().join("package"), &source_dir);
        assert!(smol::block_on(build(&host, &source, &sdk, "sdksha")).is_err());
        // No ready marker and no bridge output may exist after failure.
        let cache_root = machine
            .home()
            .join("Library/Caches/waterui/browser-runtimes/wpe-source-build");
        if cache_root.exists() {
            assert!(
                std::fs::read_dir(&cache_root).unwrap().all(|entry| !entry
                    .unwrap()
                    .path()
                    .join("native_bridge.json")
                    .exists())
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_build_requiring_both_outputs_rejects_a_missing_extension() {
        let machine = machine_with_cmake();
        let host = machine.host([("WATERUI_FAKE_CMAKE_OMIT_EXTENSION", "1")]);
        let sdk = sdk_with_modules(&machine);
        let source_dir = machine.dir("package/native");
        machine.file("package/native/waterui_wpe.c", "int main(void){return 0;}");
        let source = resolved(&machine.root().join("package"), &source_dir);
        assert!(smol::block_on(build(&host, &source, &sdk, "sdksha")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_tampered_cache_entry_is_rejected_not_served() {
        let machine = machine_with_cmake();
        let host = machine.host(std::iter::empty::<(&str, &str)>());
        let sdk = sdk_with_modules(&machine);
        let source_dir = machine.dir("package/native");
        machine.file("package/native/waterui_wpe.c", "int main(void){return 0;}");
        let source = resolved(&machine.root().join("package"), &source_dir);
        let built = smol::block_on(build(&host, &source, &sdk, "sdksha")).unwrap();
        std::fs::write(built.prefix.join("lib/libwaterui_wpe.so"), b"tampered").unwrap();
        assert!(smol::block_on(build(&host, &source, &sdk, "sdksha")).is_err());
    }

    #[test]
    fn an_empty_or_partial_outputs_record_never_validates() {
        let prefix = tempfile::tempdir().expect("cache prefix");
        std::fs::create_dir_all(prefix.path().join("lib/waterui-wpe/extensions")).unwrap();
        std::fs::write(prefix.path().join("lib/libwaterui_wpe.so"), b"bridge").unwrap();
        std::fs::write(
            prefix
                .path()
                .join("lib/waterui-wpe/extensions/waterui_wpe_extension.so"),
            b"extension",
        )
        .unwrap();
        // An outputs:{} manifest cannot satisfy validation.
        std::fs::write(
            prefix.path().join(MANIFEST),
            serde_json::to_vec(&serde_json::json!({
                "inputs": input("src"),
                "outputs": {}
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(smol::block_on(validate_cached(prefix.path(), &input("src"))).is_err());
        // And a manifest recorded before the extension lands fails too.
        let base = input("src");
        let bridge_only = serde_json::to_vec(&serde_json::json!({
            "inputs": base,
            "outputs": { "bridge_sha256": "x" }
        }))
        .unwrap();
        std::fs::write(prefix.path().join(MANIFEST), bridge_only).unwrap();
        assert!(smol::block_on(validate_cached(prefix.path(), &base)).is_err());
    }

    #[test]
    fn sdk_descriptors_normalize_to_pcfiledir_relative() {
        let sdk = tempfile::tempdir().expect("sdk candidate");
        let dir = sdk.path().join("lib/pkgconfig");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("wpe-webkit-2.0.pc"),
            "prefix=/old/wpe-sdk\nlibdir=/old/wpe-sdk/lib\nincludedir=${prefix}/include\nRequires: other\nLibs: -L${libdir} -lwpe\n# keep me\n",
        )
        .unwrap();
        smol::block_on(normalize_sdk_descriptors(sdk.path().to_path_buf())).unwrap();
        let text = std::fs::read_to_string(dir.join("wpe-webkit-2.0.pc")).unwrap();
        assert!(text.starts_with("prefix=${pcfiledir}/../..\n"), "{text}");
        assert!(text.contains("libdir=${prefix}/lib\n"), "{text}");
        assert!(text.contains("includedir=${prefix}/include"), "{text}");
        assert!(text.contains("Requires: other"), "{text}");
        assert!(text.contains("# keep me"), "{text}");
        // Idempotent: a second pass changes nothing.
        smol::block_on(normalize_sdk_descriptors(sdk.path().to_path_buf())).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("wpe-webkit-2.0.pc")).unwrap(),
            text
        );
    }

    #[test]
    fn sdk_descriptor_edges_and_errors() {
        let sdk = tempfile::tempdir().expect("sdk candidate");
        let dir = sdk.path().join("share/pkgconfig/deeper");
        std::fs::create_dir_all(&dir).unwrap();
        // Root-sibling path is not rebased; deeper nesting gets more parents.
        std::fs::write(
            dir.join("deep.pc"),
            "Name: deep\nprefix=/old/sdk\nfoo=/old/sdk-sibling/x /old/sdk/y\nVersion: 1\n",
        )
        .unwrap();
        smol::block_on(normalize_sdk_descriptors(sdk.path().to_path_buf())).unwrap();
        let text = std::fs::read_to_string(dir.join("deep.pc")).unwrap();
        assert!(text.starts_with("prefix=${pcfiledir}/../../..\n"), "{text}");
        assert!(text.contains("/old/sdk-sibling/x"), "{text}");
        assert!(text.contains("${prefix}/y"), "{text}");
        // No prefix -> named error.
        std::fs::write(dir.join("bad.pc"), "Name: bad\nVersion: 1\n").unwrap();
        assert!(smol::block_on(normalize_sdk_descriptors(sdk.path().to_path_buf())).is_err());
    }

    #[test]
    fn an_sdk_without_descriptors_cannot_build() {
        let sdk = tempfile::tempdir().expect("sdk root");
        assert!(smol::block_on(require_sdk_modules(sdk.path())).is_err());
        std::fs::create_dir_all(sdk.path().join("lib/pkgconfig")).unwrap();
        for module in [
            "wpe-webkit-2.0.pc",
            "wpe-platform-2.0.pc",
            "wpe-web-process-extension-2.0.pc",
        ] {
            std::fs::write(sdk.path().join("lib/pkgconfig").join(module), b"pc").unwrap();
        }
        smol::block_on(require_sdk_modules(sdk.path())).expect("a complete SDK validates");
    }
}
