//! The inspector client a project selects — built from its own `WaterUI`
//! checkout, or fetched from the release that checkout came from.
//!
//! The expected identity is read from the project's resolved `waterui` and
//! `waterui-inspector-protocol` packages; every produced binary answers
//! `--identity` and must equal it before it may launch — a cached or
//! downloaded artifact with a foreign identity is never run.

use std::ffi::OsStr;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cargo_metadata::{Metadata, Package};
use eyre::{Context as _, Result, bail, eyre};
use futures_util::future::{Either, select};
use target_lexicon::Triple;
use tracing::info;
use waterui_inspector_protocol::{InspectorBuildIdentity, InspectorProtocolInfo};
use zenwave::ResponseExt as _;

use crate::build::BuildProgress;
use crate::device::Artifact;
use crate::platform::NativeOs;
use crate::project_model::{assets, water_dir};
use crate::toolchain::Host;
use crate::workflows::build::RustBuild;

/// The inspector binary name — `Local` runs it as a plain executable.
const CLIENT_BINARY: &str = "waterui-inspector";
/// How long a `--identity` probe may take before the binary is unusable.
const IDENTITY_TIMEOUT: Duration = Duration::from_secs(10);
/// Release metadata is small; anything beyond this is a stuck transport.
const RELEASE_METADATA_TIMEOUT: Duration = Duration::from_secs(30);
/// The archive can be tens of megabytes — long enough for slow mirrors,
/// short enough to fail a stuck transfer.
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(120);
/// The directory inside the archive/cache that holds the client package.
pub(super) const CLIENT_PACKAGE: &str = "waterui-inspector";
/// The receipt a published cache package carries — the verified binary's
/// hash, checked before a cached binary ever runs.
const CACHE_RECEIPT: &str = ".cache.json";

/// The project's selected `waterui` + `waterui-inspector-protocol` —
/// the identity every client binary must answer.
#[derive(Debug)]
struct SelectedPackages {
    /// `waterui`'s resolved version — the client's framework version.
    framework_version: String,
    /// `waterui-inspector-protocol`'s resolved version.
    protocol_version: String,
    /// Where the facade comes from: a checkout of this repository, or the
    /// registry.
    source: FacadeSource,
}

/// How the inspected project resolves `waterui`.
#[derive(Debug)]
enum FacadeSource {
    /// A `path` or `git` dependency on this repository — the client is built
    /// from that checkout's `apps/inspector`.
    Checkout {
        /// The checkout's workspace root (the facade manifest's directory).
        root: PathBuf,
    },
    /// A crates.io version — the client is the published binary release.
    Registry,
}

/// `future` with a deadline — a call that never completes fails clearly
/// rather than waiting forever.
async fn bounded<T>(
    future: impl Future<Output = Result<T>>,
    timeout: Duration,
    what: &str,
) -> Result<T> {
    let future = std::pin::pin!(future);
    let timer = std::pin::pin!(smol::Timer::after(timeout));
    match select(future, timer).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => bail!("{what} did not complete within {timeout:?}"),
    }
}

/// Read the project's resolved `waterui` and `waterui-inspector-protocol`,
/// failing on an ambiguous or absent selection rather than guessing.
async fn resolve_packages(host: &Host, project_path: &Path) -> Result<SelectedPackages> {
    let mut command = cargo_metadata::MetadataCommand::new();
    command
        .current_dir(project_path)
        .other_options(vec![String::from("--locked")]);
    let metadata = host
        .cargo_metadata(&command)
        .await
        .wrap_err("failed to resolve the project's Cargo metadata")?;

    let waterui = select_unique(&metadata, "waterui")?;
    let protocol = select_unique(&metadata, "waterui-inspector-protocol")?;
    let source = select_source(host, waterui, protocol).await?;

    Ok(SelectedPackages {
        framework_version: waterui.version.to_string(),
        protocol_version: protocol.version.to_string(),
        source,
    })
}

fn select_unique<'a>(metadata: &'a Metadata, name: &str) -> Result<&'a Package> {
    let mut matches = metadata.packages.iter().filter(|p| p.name == name);
    let first = matches
        .next()
        .ok_or_else(|| eyre!("could not resolve a `{name}` package from the project"))?;
    if matches.next().is_some() {
        bail!("multiple `{name}` packages resolved; the inspector needs exactly one");
    }
    Ok(first)
}

/// Whether a resolved package comes from a checkout (`path`/`git`, or no
/// source at all — a manifest directory in the same graph) or a registry.
fn is_checkout(package: &Package) -> bool {
    package.source.as_ref().is_none_or(|source| {
        let source = source.to_string();
        source.starts_with("path+") || source.starts_with("git+")
    })
}

/// Pick the client source from where the project resolves *both* packages.
///
/// The identity the client carries mixes the facade's framework version with
/// the protocol's build — the two packages must come from one source class:
/// both from the registry, or both from checkouts that resolve the same
/// revision. A protocol manifest inside the facade's own tree shares its
/// HEAD; one checked out elsewhere must still resolve the same commit.
async fn select_source(host: &Host, waterui: &Package, protocol: &Package) -> Result<FacadeSource> {
    match (is_checkout(waterui), is_checkout(protocol)) {
        (false, false) => Ok(FacadeSource::Registry),
        (true, false) => bail!(
            "the project resolves `waterui` from a checkout but \
             `waterui-inspector-protocol` from a registry — the inspector \
             cannot carry one coherent identity from two sources"
        ),
        (false, true) => bail!(
            "the project resolves `waterui` from a registry but \
             `waterui-inspector-protocol` from a checkout — the inspector \
             cannot carry one coherent identity from two sources"
        ),
        (true, true) => {
            // The facade is the repository's root crate: its manifest's
            // directory is the workspace root the client builds inside.
            let root = waterui
                .manifest_path
                .as_std_path()
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| eyre!("the waterui manifest has no parent directory"))?;
            let app_manifest = root.join("apps/inspector/Cargo.toml");
            eyre::ensure!(
                app_manifest.is_file(),
                "the resolved waterui checkout at {} does not contain \
                 the inspector application at apps/inspector — a checkout of a \
                 WaterUI release predating the in-tree client cannot build one",
                root.display()
            );
            if !protocol.manifest_path.as_std_path().starts_with(&root) {
                // A foreign checkout of the protocol is only coherent when
                // it resolves the facade checkout's own revision.
                let protocol_dir = protocol
                    .manifest_path
                    .as_std_path()
                    .parent()
                    .ok_or_else(|| eyre!("the protocol manifest has no parent directory"))?;
                let facade_head = checkout_head(host, &root).await?;
                let protocol_head = checkout_head(host, protocol_dir).await?;
                eyre::ensure!(
                    facade_head == protocol_head,
                    "the project resolves `waterui` and \
                     `waterui-inspector-protocol` from checkouts at different \
                     revisions ({facade_head} vs {protocol_head}) — the \
                     inspector cannot carry one coherent identity"
                );
            }
            Ok(FacadeSource::Checkout { root })
        }
    }
}

/// `git rev-parse HEAD` for the repository containing `dir` — the protocol
/// build's stamp.
async fn checkout_head(host: &Host, dir: &Path) -> Result<String> {
    let head = host
        .run(
            OsStr::new("git"),
            [
                OsStr::new("-C"),
                dir.as_os_str(),
                OsStr::new("rev-parse"),
                OsStr::new("HEAD"),
            ],
        )
        .await
        .wrap_err_with(|| {
            format!(
                "could not resolve HEAD of the checkout at {}",
                dir.display()
            )
        })?;
    let head = head.trim();
    eyre::ensure!(
        head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit()),
        "git rev-parse HEAD returned {head:?}, not a 40-hex revision"
    );
    Ok(head.to_string())
}

/// Resolve, build or fetch the client binary for this project.
///
/// The running endpoint's identity is the source of truth: an endpoint
/// whose protocol carries no source revision is a released build, so its
/// matching released client is fetched — the project's manifest never
/// enters that decision. An endpoint reporting a checkout revision needs a
/// client built from that checkout; the project's resolution must name the
/// same source and revision before any compile or download starts.
///
/// # Errors
/// Returns an error when the project's `WaterUI` source cannot be resolved,
/// the build fails, the fetched artifact cannot be verified, or the
/// project and the endpoint disagree about versions or revisions.
pub(super) async fn prepare_client(
    host: &Host,
    project_path: &Path,
    identity: &InspectorBuildIdentity,
    progress: Option<BuildProgress>,
) -> Result<Artifact> {
    let semver = |field: &str, version: &str| -> Result<semver::Version> {
        version.parse().wrap_err_with(|| {
            format!("the endpoint reports a malformed {field} version {version:?}")
        })
    };
    semver("framework", &identity.framework_version)?;
    semver("protocol", &identity.protocol.version)?;

    let Some(commit) = &identity.protocol.build_commit else {
        // A released endpoint: fetch the release of the identity it
        // reported — Cargo metadata is never consulted.
        let selected = SelectedPackages {
            framework_version: identity.framework_version.clone(),
            protocol_version: identity.protocol.version.clone(),
            source: FacadeSource::Registry,
        };
        return fetch_release(host, &selected, identity).await;
    };
    eyre::ensure!(
        commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "the endpoint reports a malformed source revision {commit:?}"
    );

    let selected = resolve_packages(host, project_path).await?;
    let FacadeSource::Checkout { root } = &selected.source else {
        bail!(
            "the endpoint is an unreleased checkout ({}) but the project \
             resolves `waterui` from a registry — rebuild the target against \
             a released `waterui` or select a matching checkout",
            commit
        );
    };
    let head = checkout_head(host, root).await?;
    eyre::ensure!(
        head == *commit,
        "the endpoint was built at {commit} but the project's checkout is \
         at {head} — rebuild the target or select a matching checkout"
    );
    eyre::ensure!(
        selected.framework_version == identity.framework_version
            && selected.protocol_version == identity.protocol.version,
        "the endpoint reports waterui {} / protocol {} but the project \
         resolves {} / {} — select a matching release or checkout",
        identity.framework_version,
        identity.protocol.version,
        selected.framework_version,
        selected.protocol_version
    );
    build_from_checkout(host, root, identity, progress).await
}

/// The package directory a source build publishes into — named by the
/// binary's own content, so a rebuild that changed the artifact lands beside
/// the previous one instead of overwriting a client another session holds.
fn package_name(binary_sha256: &str) -> String {
    format!("{CLIENT_PACKAGE}-{binary_sha256}")
}

/// The protocol build script's identity mode — `checkout` stamps the
/// source revision into the protocol identity, `release` stamps none.
const BUILD_MODE_ENV: &str = "WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE";

/// The `RustBuild` that produces the inspector client: the workspace's own
/// `apps/inspector` crate for this machine's triple, its identity stamped
/// for the build's profile — `checkout` for a development build's source
/// revision, `release` for a package whose identity carries none.
fn client_build(
    host: &Host,
    root: &Path,
    release: bool,
    progress: Option<BuildProgress>,
) -> RustBuild {
    let mut build = RustBuild::new(
        &host.clone().with_cwd(root),
        root.join("apps/inspector"),
        Triple::host(),
    )
    .with_feature("desktop")
    .with_env(BUILD_MODE_ENV, if release { "release" } else { "checkout" });
    if let Some(progress) = progress {
        build = build.with_progress(progress);
    }
    build
}

/// Build the client and snapshot its immutable artifact — still under the
/// held artifact lock — into a staging directory beside the build's own
/// profile directory. Returns the staged package body, the profile
/// directory, and the snapshot binary's sha256.
///
/// Cargo always decides whether to rebuild; the hash only names a package,
/// never gates one.
async fn build_and_stage(
    host: &Host,
    root: &Path,
    release: bool,
    progress: Option<BuildProgress>,
) -> Result<(tempfile::TempDir, PathBuf, String)> {
    info!("Building the inspector client from the WaterUI checkout...");
    let built_client = client_build(host, root, release, progress)
        .build_binary(CLIENT_BINARY, release)
        .await
        .map_err(|e| eyre!("failed to build the inspector client: {e}"))?;

    // Snapshot the artifact while `built_client` still holds the build's
    // artifact lock — the `deps/<name>-<marker>` path is options-stable, so
    // a rebuild rewrites the bytes under it; copying under the held lock is
    // what makes the snapshot immutable.
    let stage_parent = built_client.profile_dir.clone();
    let stage = smol::unblock(move || tempfile::TempDir::with_prefix_in("client-", &stage_parent))
        .await
        .wrap_err("could not stage the inspector package")?;
    smol::fs::copy(&built_client.artifact, stage.path().join(binary_name()))
        .await
        .wrap_err("failed to stage the inspector binary")?;
    let binary_sha256 = crate::runtime::utils::file_sha256(&stage.path().join(binary_name()))
        .await
        .wrap_err("could not hash the staged inspector binary")?;
    drop(built_client); // the lock was held until the snapshot existed
    let profile_dir = stage
        .path()
        .parent()
        .expect("the staging directory has a parent")
        .to_path_buf();
    Ok((stage, profile_dir, binary_sha256))
}

/// Build `apps/inspector` from the project's own checkout and publish the
/// immutable build artifact with its resources under the build's profile
/// directory — named by the binary's own content, so a rebuild that changed
/// the artifact lands beside the previous one instead of overwriting a
/// client another session holds.
async fn build_from_checkout(
    host: &Host,
    root: &Path,
    expected: &InspectorBuildIdentity,
    progress: Option<BuildProgress>,
) -> Result<Artifact> {
    let (stage, profile_dir, binary_sha256) = build_and_stage(host, root, false, progress).await?;
    let package_dir = profile_dir.join(package_name(&binary_sha256));
    let binary = package_dir.join(binary_name());
    if package_dir.is_dir() {
        // Identical bytes published before: the existing package still
        // proves itself rather than being re-staged.
        verify_identity(host, &binary, expected, IDENTITY_TIMEOUT).await?;
        return Ok(Artifact::new("dev.waterui.inspector", binary));
    }

    let app_manifest = root.join("apps/inspector/Cargo.toml");
    stage_resources(host, &app_manifest, stage.path()).await?;
    verify_identity(
        host,
        &stage.path().join(binary_name()),
        expected,
        IDENTITY_TIMEOUT,
    )
    .await?;
    smol::fs::rename(stage.path(), &package_dir)
        .await
        .wrap_err_with(|| {
            format!(
                "could not publish the inspector package at {}",
                package_dir.display()
            )
        })?;
    Ok(Artifact::new("dev.waterui.inspector", binary))
}

/// Package the inspector's release archive from a `WaterUI` checkout.
///
/// `workspace` is the `WaterUI` checkout whose `apps/inspector` builds the
/// client — its resolved `waterui`/`waterui-inspector-protocol` versions
/// name the release identity (a released package carries no source
/// revision). The archive lands at
/// `output_dir/waterui-inspector-<host triple>.zip`, verified against the
/// staged binary's `--identity` answer before any archive exists; an
/// existing file is never truncated.
///
/// Producing the archive implies nothing about publication — it is one
/// artifact on disk.
///
/// # Errors
/// Fails when `workspace` does not resolve `waterui` from a checkout, the
/// build or the identity probe fails, the archive cannot be written, or
/// the destination already exists.
pub async fn package_release_client(
    host: &Host,
    workspace: &Path,
    output_dir: &Path,
) -> Result<PathBuf> {
    let selected = resolve_packages(host, workspace).await?;
    let FacadeSource::Checkout { root } = &selected.source else {
        bail!(
            "a release package builds from a WaterUI checkout — {} resolves \
             `waterui` from a registry",
            workspace.display()
        );
    };
    // A released client's identity carries the resolved versions and no
    // source revision — the same shape the endpoint answers `Identify`
    // with after a release-mode protocol stamp.
    let expected = InspectorBuildIdentity {
        framework_version: selected.framework_version.clone(),
        protocol: InspectorProtocolInfo {
            version: selected.protocol_version.clone(),
            build_commit: None,
        },
    };

    let (stage, _profile_dir, _sha) = build_and_stage(host, root, true, None).await?;
    let app_manifest = root.join("apps/inspector/Cargo.toml");
    stage_resources(host, &app_manifest, stage.path()).await?;
    package_staged_client(host, stage.path(), &expected, output_dir).await
}

/// Verify the staged package body's binary, then archive it.
///
/// Split so a failed probe never writes an archive — and so the caller's
/// cancellation between the two cannot publish a staged package.
async fn package_staged_client(
    host: &Host,
    package_root: &Path,
    expected: &InspectorBuildIdentity,
    output_dir: &Path,
) -> Result<PathBuf> {
    verify_identity(
        host,
        &package_root.join(binary_name()),
        expected,
        IDENTITY_TIMEOUT,
    )
    .await?;
    write_release_archive(package_root, output_dir).await
}

/// The release archive's file name for this host.
fn release_archive_name() -> String {
    format!("{CLIENT_BINARY}-{}.zip", Triple::host())
}

/// Write `package_root`'s contents as `waterui-inspector-*.zip` under
/// `output_dir` — every entry rooted at `waterui-inspector/` with `/`
/// separators, deflate compressed, file modes preserved, files sorted for
/// a stable archive.
///
/// The zip is written into a temporary file beside the destination and
/// published with `persist_noclobber`: an existing destination is never
/// truncated, and a failed write leaves no partial archive. Cancellation
/// cannot interrupt the blocking worker; it may finish publishing a complete
/// archive, but never exposes its temporary output at the destination.
async fn write_release_archive(package_root: &Path, output_dir: &Path) -> Result<PathBuf> {
    smol::fs::create_dir_all(output_dir)
        .await
        .wrap_err_with(|| format!("could not create {}", output_dir.display()))?;
    let destination = output_dir.join(release_archive_name());
    let package_root = package_root.to_path_buf();
    let dest = destination.clone();
    let out_dir = output_dir.to_path_buf();
    smol::unblock(move || -> Result<()> {
        let mut staged = tempfile::NamedTempFile::new_in(&out_dir)
            .wrap_err("could not stage the release archive")?;
        {
            let mut zip = zip::ZipWriter::new(staged.as_file_mut());
            let mut entries = Vec::new();
            for entry in walkdir::WalkDir::new(&package_root).sort_by_file_name() {
                entries.push(entry?.into_path());
            }
            entries.sort();
            for path in entries {
                let relative = path
                    .strip_prefix(&package_root)
                    .expect("entries stay under the package root");
                if relative.as_os_str().is_empty() {
                    continue;
                }
                // The cache receipt is a local-cache artifact, never release
                // content.
                if relative == Path::new(CACHE_RECEIPT) {
                    continue;
                }
                // '/' separators on every platform — ZIP names are
                // component lists, not host path strings.
                let mut name = String::from(CLIENT_PACKAGE);
                for component in relative {
                    name.push('/');
                    name.push_str(
                        component.to_str().ok_or_else(|| {
                            eyre!("non-UTF-8 package path {}", relative.display())
                        })?,
                    );
                }
                let metadata = path.metadata()?;
                if metadata.is_dir() {
                    zip.add_directory(name, zip::write::SimpleFileOptions::default())?;
                    continue;
                }
                let mut options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    options = options.unix_permissions(metadata.permissions().mode() & 0o777);
                }
                zip.start_file(name, options)?;
                let mut source = std::fs::File::open(&path)?;
                std::io::copy(&mut source, &mut zip)?;
            }
            zip.finish()
                .wrap_err("failed to finalize the release archive")?;
        }
        staged
            .persist_noclobber(&dest)
            .map_err(|error| error.error)
            .wrap_err_with(|| {
                format!(
                    "refusing to overwrite an existing release archive at {}",
                    dest.display()
                )
            })?;
        Ok(())
    })
    .await
    .wrap_err("failed to write the release archive")?;
    Ok(destination)
}

pub(super) fn binary_name() -> String {
    if cfg!(windows) {
        format!("{CLIENT_BINARY}.exe")
    } else {
        CLIENT_BINARY.to_string()
    }
}

/// Lay out the app's declared fonts and asset directory under `dest`'s
/// `resources/` so `ResourceContext::application` resolves them beside the
/// binary the caller already placed there.
async fn stage_resources(host: &Host, app_manifest: &Path, dest: &Path) -> Result<()> {
    smol::fs::create_dir_all(dest).await?;

    let resources = dest.join("resources");
    // Created even when no dependency declares fonts: `ResourceContext`
    // selects the adjacent `resources/waterui_assets` as the app's root.
    smol::fs::create_dir_all(resources.join("waterui_assets")).await?;

    let metadata = assets::crate_metadata(host, app_manifest, &[String::from("desktop")])
        .await
        .wrap_err("failed to resolve the inspector's declared fonts")?;
    let platform = NativeOs::running_host().target_platform().font_platform()?;
    let declarations: Vec<_> = waterui_assets_planner::dependency_font_declarations(
        &metadata,
        waterui_assets_planner::GraphScope::Build,
    )
    .wrap_err("invalid font declarations in the inspector's graph")?
    .into_iter()
    .filter(|declaration| declaration.bundled_on(platform))
    .collect();
    let fonts = assets::resolve_fonts(host, declarations).await?;
    assets::copy_fonts(&fonts, &resources.join("fonts")).await?;

    if cfg!(windows) {
        crate::workflows::build::stage_dxc_runtime(host, dest).await?;
    }
    Ok(())
}

/// The receipt a published cache package carries — the hash of the binary
/// the verified archive produced, checked before that binary ever runs.
/// This protects the executable's integrity; a resource file a user edited
/// deliberately is outside this check's scope.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedClient {
    binary_sha256: String,
}

/// Fetch the published `waterui-inspector-<triple>.zip` for the project's
/// released `waterui` version, or serve the verified cache.
async fn fetch_release(
    host: &Host,
    selected: &SelectedPackages,
    expected: &InspectorBuildIdentity,
) -> Result<Artifact> {
    let triple = Triple::host().to_string();
    let cache_dir = cache_root(host)?
        .join(&selected.framework_version)
        .join(&triple);
    let package = cache_dir.join(CLIENT_PACKAGE);
    let binary = package.join(binary_name());

    if package.exists() {
        serve_cached(host, &package, &binary, expected).await?;
        info!("Using the cached inspector at {}", package.display());
        return Ok(Artifact::new("dev.waterui.inspector", binary));
    }

    let asset = release_asset(host, selected, &triple).await?;
    download_and_publish(host, &asset, expected, &cache_dir).await?;
    Ok(Artifact::new("dev.waterui.inspector", binary))
}

/// A cached package must be complete: receipt, binary, matching hash, and
/// finally the `--identity` answer. Anything else fails explicitly — a
/// broken package is never re-downloaded silently and a hash mismatch never
/// executes the binary.
async fn serve_cached(
    host: &Host,
    package: &Path,
    binary: &Path,
    expected: &InspectorBuildIdentity,
) -> Result<()> {
    let receipt_path = package.join(CACHE_RECEIPT);
    let text = smol::fs::read_to_string(&receipt_path)
        .await
        .map_err(|error| {
            eyre!(
                "the cached inspector at {} has no usable receipt ({error}); \
             remove {} and run `water inspector` again",
                package.display(),
                package.display()
            )
        })?;
    let receipt: CachedClient =
        serde_json::from_str(&text).wrap_err("the cached inspector's receipt did not parse")?;
    eyre::ensure!(
        binary.is_file(),
        "the cached inspector at {} has no {} — remove it and run \
         `water inspector` again",
        package.display(),
        binary_name()
    );
    let hash_of = binary.to_path_buf();
    let actual = smol::unblock(move || -> Result<String> {
        use sha2::Digest as _;
        Ok(hex::encode(sha2::Sha256::digest(std::fs::read(&hash_of)?)))
    })
    .await
    .wrap_err("could not hash the cached inspector binary")?;
    eyre::ensure!(
        actual.eq_ignore_ascii_case(&receipt.binary_sha256),
        "the cached inspector binary at {} does not match its receipt — \
         remove {} and run `water inspector` again",
        binary.display(),
        package.display()
    );
    verify_identity(host, binary, expected, IDENTITY_TIMEOUT).await
}

/// The one release asset named `waterui-inspector-<triple>.zip`, carrying its
/// published sha256 digest and a download URL inside this release.
async fn release_asset(
    host: &Host,
    selected: &SelectedPackages,
    triple: &str,
) -> Result<ReleaseAsset> {
    let api_url = format!(
        "https://api.github.com/repos/water-rs/waterui/releases/tags/v{}",
        selected.framework_version
    );
    let release: GitHubRelease = bounded(
        async {
            let response = host
                .http_get(
                    &api_url,
                    &[
                        ("Accept", "application/vnd.github+json"),
                        ("User-Agent", "waterui-cli"),
                    ],
                )
                .await
                .wrap_err_with(|| format!("could not read {api_url}"))?;
            let status = response.status();
            eyre::ensure!(status.is_success(), "{api_url} answered {status}");
            response
                .into_json()
                .await
                .wrap_err("the release record did not parse as JSON")
        },
        RELEASE_METADATA_TIMEOUT,
        "the release record request",
    )
    .await?;

    let asset_name = format!("{CLIENT_BINARY}-{triple}.zip");
    let mut candidates = release.assets.iter().filter(|a| a.name == asset_name);
    let asset = candidates.next().ok_or_else(|| {
        eyre!(
            "release v{} publishes no {asset_name}",
            selected.framework_version
        )
    })?;
    if candidates.next().is_some() {
        bail!(
            "release v{} publishes duplicate {asset_name} assets",
            selected.framework_version
        );
    }

    let digest = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .ok_or_else(|| eyre!("{asset_name} publishes no sha256 digest"))?;
    eyre::ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "{asset_name}'s digest {digest:?} is not a 64-hex sha256"
    );
    let download_prefix = format!(
        "https://github.com/water-rs/waterui/releases/download/v{}/",
        selected.framework_version
    );
    eyre::ensure!(
        asset.browser_download_url.starts_with(&download_prefix),
        "{asset_name}'s URL {} does not live under {download_prefix}",
        asset.browser_download_url
    );
    Ok(ReleaseAsset {
        url: asset.browser_download_url.clone(),
        digest: digest.to_ascii_lowercase(),
        name: asset_name,
    })
}

/// The selected release asset, verified.
struct ReleaseAsset {
    url: String,
    digest: String,
    name: String,
}

/// Download `asset`, verify its hash, extract to a staging directory, and
/// publish under `cache_dir` only once the binary's `--identity` matches — a
/// failed fetch or validation leaves the cache untouched.
async fn download_and_publish(
    host: &Host,
    asset: &ReleaseAsset,
    expected: &InspectorBuildIdentity,
    cache_dir: &Path,
) -> Result<()> {
    // Stage beside the cache so the validated directory lands atomically.
    let stage_parent = cache_dir.parent().expect("the cache dir has a parent");
    smol::fs::create_dir_all(stage_parent).await?;
    let stage_dir = stage_parent.to_path_buf();
    let stage = smol::unblock(move || tempfile::TempDir::with_prefix_in("stage-", &stage_dir))
        .await
        .wrap_err("could not stage the inspector release")?;

    let archive = bounded(
        async {
            let response = host
                .http_get(&asset.url, &[("User-Agent", "waterui-cli")])
                .await
                .wrap_err_with(|| format!("could not download {}", asset.name))?;
            let status = response.status();
            eyre::ensure!(status.is_success(), "{} answered {status}", asset.url);
            response
                .into_bytes()
                .await
                .wrap_err_with(|| format!("could not read {}", asset.name))
        },
        ARCHIVE_TIMEOUT,
        "the inspector archive download",
    )
    .await?;

    // Hashing and extraction are blocking work; they run off the async
    // executor, and the receipt the package carries is written here so the
    // published directory is complete in one move. The worker owns `stage`
    // until it finishes — a cancelled caller cannot drop the directory out
    // from under in-flight extraction and resurrect it entry by entry.
    let name = asset.name.clone();
    let digest = asset.digest.clone();
    let stage = smol::unblock(move || -> Result<tempfile::TempDir> {
        use sha2::Digest as _;
        let actual = hex::encode(sha2::Sha256::digest(&archive));
        eyre::ensure!(
            actual.eq_ignore_ascii_case(&digest),
            "{name} hashed {actual}, expected {digest}"
        );
        extract_zip(&archive, stage.path())?;

        let package = stage.path().join(CLIENT_PACKAGE);
        let binary = package.join(binary_name());
        eyre::ensure!(
            binary.is_file(),
            "{name} contains no {CLIENT_PACKAGE}/{} entry",
            binary_name()
        );
        let receipt = CachedClient {
            binary_sha256: hex::encode(sha2::Sha256::digest(std::fs::read(&binary)?)),
        };
        std::fs::write(package.join(CACHE_RECEIPT), serde_json::to_vec(&receipt)?)?;
        Ok(stage)
    })
    .await?;

    verify_identity(
        host,
        &stage.path().join(CLIENT_PACKAGE).join(binary_name()),
        expected,
        IDENTITY_TIMEOUT,
    )
    .await?;

    smol::fs::create_dir_all(cache_dir).await?;
    smol::fs::rename(
        stage.path().join(CLIENT_PACKAGE),
        cache_dir.join(CLIENT_PACKAGE),
    )
    .await
    .wrap_err_with(|| {
        format!(
            "could not publish the inspector cache at {}",
            cache_dir.display()
        )
    })?;
    Ok(())
}

/// `~/.water/inspector/releases` — published clients live under
/// `<framework-version>/<triple>/`.
fn cache_root(host: &Host) -> Result<PathBuf> {
    Ok(water_dir::water_home_dir(host)
        .wrap_err("the inspector cache needs a water home")?
        .join("inspector")
        .join("releases"))
}

#[derive(serde::Deserialize)]
struct GitHubRelease {
    assets: Vec<GitHubAsset>,
}

#[derive(serde::Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

/// Extract `archive` under `dest`, rejecting entries that would escape or
/// are symlinks — a release archive names only ordinary files.
fn extract_zip(archive: &[u8], dest: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .wrap_err("the inspector release is not a zip archive")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry
            .enclosed_name()
            .ok_or_else(|| eyre!("archive entry {:?} escapes the destination", entry.name()))?;
        eyre::ensure!(!entry.is_symlink(), "archive entry {name:?} is a symlink");
        let out = dest.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut file)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

/// The binary must answer `--identity` with the identity the project's
/// resolution computed — a built, cached or downloaded client that disagrees
/// is never launched.
///
/// The probe strips the session's inspector credentials from the child
/// environment — a binary that only prints its identity needs none — and a
/// binary that does not answer within `timeout` is killed and reaped when
/// the bounded future drops it, never left running.
async fn verify_identity(
    host: &Host,
    binary: &Path,
    expected: &InspectorBuildIdentity,
    timeout: Duration,
) -> Result<()> {
    let mut command = host.command(binary);
    command
        .arg("--identity")
        .env_remove("WATERUI_INSPECTOR_TOKEN")
        .env_remove("WATERUI_INSPECTOR_TARGET_ADDR")
        .kill_on_drop(true);
    let probe = std::pin::pin!(command.output());
    let timer = std::pin::pin!(smol::Timer::after(timeout));
    let output = match select(probe, timer).await {
        Either::Left((result, _)) => {
            result.wrap_err_with(|| format!("could not run {} --identity", binary.display()))?
        }
        Either::Right(_) => {
            bail!(
                "{} --identity did not answer within {timeout:?}",
                binary.display()
            )
        }
    };
    eyre::ensure!(
        output.status.success(),
        "{} --identity exited {status}: {stderr}",
        binary.display(),
        status = output.status,
        stderr = String::from_utf8_lossy(&output.stderr).trim_end()
    );
    let actual: InspectorBuildIdentity = serde_json::from_slice(&output.stdout)
        .wrap_err("the inspector's --identity output did not parse")?;
    eyre::ensure!(
        actual == *expected,
        "the inspector at {} reports {actual}, expected {expected}",
        binary.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use sha2::Digest as _;
    use waterui_inspector_protocol::InspectorProtocolInfo;

    use super::*;
    use crate::toolchain::testing::TestMachine;

    /// The smallest `cargo metadata` package entry the fields this module
    /// reads need — anything else defaults.
    fn package_json(
        name: &str,
        version: &str,
        source: Option<&str>,
        manifest: &Path,
    ) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "version": version,
            "id": format!("registry+{name}#{version}"),
            "license": null,
            "license_file": null,
            "description": null,
            "source": source,
            "dependencies": [],
            "manifest_path": manifest,
            "categories": [],
            "keywords": [],
            "readme": null,
            "repository": null,
            "homepage": null,
            "documentation": null,
            "metadata": {},
            "links": null,
            "publish": null,
            "authors": [],
            "edition": "2024",
            "rust_version": null,
            "features": {},
            "targets": [{
                "kind": ["lib"],
                "name": name,
                "src_path": manifest.parent().unwrap().join("src/lib.rs"),
                "edition": "2024",
                "doctest": true,
                "test": true,
                "doc": true,
                "crate_types": ["lib"],
                "required-features": []
            }],
            "default_run": null,
            "default_features": true,
        })
    }

    fn metadata_json(packages: serde_json::Value) -> Metadata {
        let mut root = serde_json::json!({
            "workspace_members": [],
            "resolve": null,
            "target_directory": "/tmp/fixture/target",
            "version": 0,
            "workspace_root": "/tmp/fixture",
            "metadata": {},
        });
        root["packages"] = packages;
        serde_json::from_value(root).expect("fixture metadata parses")
    }

    fn parse_package(value: serde_json::Value) -> Package {
        serde_json::from_value(value).expect("package parses")
    }

    fn expected_identity() -> InspectorBuildIdentity {
        InspectorBuildIdentity {
            framework_version: String::from("0.5.2"),
            protocol: InspectorProtocolInfo {
                version: String::from("0.5.1"),
                build_commit: None,
            },
        }
    }

    fn registry_selected() -> SelectedPackages {
        SelectedPackages {
            framework_version: String::from("0.5.2"),
            protocol_version: String::from("0.5.1"),
            source: FacadeSource::Registry,
        }
    }

    /// The API URL `fetch_release` queries for this selected version.
    fn api_url(selected: &SelectedPackages) -> String {
        format!(
            "https://api.github.com/repos/water-rs/waterui/releases/tags/v{}",
            selected.framework_version
        )
    }

    /// A JSON release record with the given assets.
    fn release_json(assets: serde_json::Value) -> zenwave::Response {
        let mut record = serde_json::json!({ "tag_name": "v0.5.2" });
        record["assets"] = assets;
        zenwave::Response::new(zenwave::Body::from(record.to_string()))
    }

    fn asset_json(name: &str, url: &str, digest: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "name": name,
            "browser_download_url": url,
            "digest": digest,
            "size": 0,
        })
    }

    /// A zip archive carrying the fixture `waterui-inspector` script as the
    /// package's binary.
    fn fixture_zip() -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            zip.add_directory::<_, ()>(
                "waterui-inspector/",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("dir entry");
            let options = {
                #[allow(unused_mut)]
                let mut options = zip::write::SimpleFileOptions::default();
                #[cfg(unix)]
                {
                    options = options.unix_permissions(0o755);
                }
                options
            };
            zip.start_file::<_, ()>("waterui-inspector/waterui-inspector", options)
                .expect("file entry");
            zip.write_all(include_bytes!("testdata/fake_client.sh"))
                .expect("bytes");
            zip.finish().expect("seal");
        }
        cursor.into_inner()
    }

    /// Where the fixture host's cache lands — `HOME` is the machine's home.
    fn cached_package(machine: &TestMachine, selected: &SelectedPackages) -> PathBuf {
        machine
            .home()
            .join(".water/inspector/releases")
            .join(&selected.framework_version)
            .join(Triple::host().to_string())
            .join(CLIENT_PACKAGE)
    }

    /// A host whose fixture binary answers `--identity` with `identity`.
    fn identity_host(
        machine: &TestMachine,
        identity: &InspectorBuildIdentity,
        extra: impl IntoIterator<Item = (&'static str, std::ffi::OsString)>,
    ) -> Host {
        let identity_file = machine.file(
            "responses/identity.json",
            &serde_json::to_string(identity).expect("json"),
        );
        let mut vars: Vec<(String, std::ffi::OsString)> = vec![(
            String::from("WATERUI_FAKE_IDENTITY"),
            identity_file.as_os_str().to_os_string(),
        )];
        vars.extend(extra.into_iter().map(|(k, v)| (k.to_string(), v)));
        machine.host(vars)
    }

    #[test]
    fn a_project_with_no_waterui_fails_selection() {
        let metadata = metadata_json(serde_json::json!([package_json(
            "app",
            "0.1.0",
            None,
            Path::new("/tmp/fixture/Cargo.toml")
        )]));
        let error = select_unique(&metadata, "waterui").expect_err("no waterui");
        assert!(error.to_string().contains("waterui"), "{error}");
    }

    #[test]
    fn duplicate_waterui_packages_fail_selection() {
        let manifest = Path::new("/tmp/fixture/Cargo.toml");
        let metadata = metadata_json(serde_json::json!([
            package_json(
                "waterui",
                "0.5.2",
                Some("registry+https://github.com/rust-lang/crates.io-index"),
                manifest
            ),
            package_json(
                "waterui",
                "0.5.1",
                Some("registry+https://github.com/rust-lang/crates.io-index"),
                manifest
            ),
        ]));
        let error = select_unique(&metadata, "waterui").expect_err("two facades");
        assert!(error.to_string().contains("multiple"), "{error}");
    }

    #[test]
    fn a_checkout_facade_with_a_registry_protocol_is_rejected() {
        let facade = package_json("waterui", "0.5.2", None, Path::new("/checkout/Cargo.toml"));
        let protocol = package_json(
            "waterui-inspector-protocol",
            "0.5.1",
            Some("registry+https://github.com/rust-lang/crates.io-index"),
            Path::new("/tmp/protocol/Cargo.toml"),
        );
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let error = smol::block_on(select_source(
            &host,
            &parse_package(facade),
            &parse_package(protocol),
        ))
        .expect_err("mixed sources");
        assert!(error.to_string().contains("two sources"), "{error}");
    }

    #[test]
    fn a_registry_facade_with_a_checkout_protocol_is_rejected() {
        let facade = package_json(
            "waterui",
            "0.5.2",
            Some("registry+https://github.com/rust-lang/crates.io-index"),
            Path::new("/tmp/facade/Cargo.toml"),
        );
        let protocol = package_json(
            "waterui-inspector-protocol",
            "0.5.1",
            None,
            Path::new("/checkout/protocol/Cargo.toml"),
        );
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let error = smol::block_on(select_source(
            &host,
            &parse_package(facade),
            &parse_package(protocol),
        ))
        .expect_err("mixed sources");
        assert!(error.to_string().contains("two sources"), "{error}");
    }

    /// A path-resolved facade whose checkout lacks `apps/inspector` cannot
    /// build a client — the failure names the missing crate.
    #[test]
    fn a_checkout_without_the_app_fails() {
        let temp = tempfile::tempdir().expect("scratch");
        let manifest = temp.path().join("Cargo.toml");
        std::fs::write(&manifest, "[package]\nname = \"waterui\"\n").expect("manifest");
        let facade = parse_package(package_json("waterui", "0.5.2", None, &manifest));
        let protocol = parse_package(package_json(
            "waterui-inspector-protocol",
            "0.5.1",
            None,
            &manifest,
        ));
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let error = smol::block_on(select_source(&host, &facade, &protocol))
            .expect_err("no apps/inspector in the fixture");
        assert!(error.to_string().contains("apps/inspector"), "{error}");
    }

    /// Two checkouts at different revisions cannot form one identity.
    #[test]
    fn checkouts_at_different_revisions_are_rejected() {
        // Two real git checkouts at unrelated commits.
        let a = tempfile::tempdir().expect("checkout a");
        let b = tempfile::tempdir().expect("checkout b");
        for (index, dir) in [a.path(), b.path()].iter().enumerate() {
            // Distinct contents — identical empty commits hash identically.
            std::fs::write(
                dir.join("Cargo.toml"),
                format!("[package]\nname = \"checkout-{index}\"\n"),
            )
            .expect("manifest");
            std::process::Command::new("git")
                .args(["-C", dir.to_str().unwrap(), "init", "-q"])
                .status()
                .expect("git init");
            std::fs::create_dir_all(dir.join("apps/inspector")).expect("app dir");
            std::fs::write(dir.join("apps/inspector/Cargo.toml"), "x").expect("app manifest");
            std::process::Command::new("git")
                .args(["-C", dir.to_str().unwrap(), "add", "-A"])
                .status()
                .expect("git add");
            std::process::Command::new("git")
                .args([
                    "-C",
                    dir.to_str().unwrap(),
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "user.name=t",
                    "commit",
                    "-qm",
                    "initial",
                ])
                .status()
                .expect("commit");
        }
        let facade = parse_package(package_json(
            "waterui",
            "0.5.2",
            None,
            &a.path().join("Cargo.toml"),
        ));
        let protocol = parse_package(package_json(
            "waterui-inspector-protocol",
            "0.5.1",
            None,
            &b.path().join("Cargo.toml"),
        ));
        let error = smol::block_on(select_source(&Host::current(), &facade, &protocol))
            .expect_err("different revisions");
        assert!(error.to_string().contains("different"), "{error}");
    }

    /// An archive entry outside the destination must be rejected, not
    /// silently skipped.
    #[test]
    fn an_archive_entry_escaping_the_destination_is_rejected() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            zip.start_file::<_, ()>("../escape.txt", zip::write::SimpleFileOptions::default())
                .expect("write entry");
            zip.write_all(b"escape").expect("bytes");
            zip.finish().expect("seal");
        }
        let dest = tempfile::tempdir().expect("scratch");
        let error = extract_zip(&cursor.into_inner(), dest.path()).expect_err("traversal rejected");
        assert!(error.to_string().contains("escape"), "{error}");
    }

    /// A symlink entry must be rejected — extracting it would let the
    /// archive alias paths outside its own tree.
    #[cfg(unix)]
    #[test]
    fn a_symlink_entry_is_rejected() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default().unix_permissions(0o120_777);
            zip.add_symlink("link", "target", options)
                .expect("symlink entry");
            zip.finish().expect("seal");
        }
        let dest = tempfile::tempdir().expect("scratch");
        let error = extract_zip(&cursor.into_inner(), dest.path()).expect_err("symlink rejected");
        assert!(error.to_string().contains("symlink"), "{error}");
    }

    /// A well-formed archive extracts under the destination.
    #[test]
    fn a_release_archive_extracts() {
        let archive = fixture_zip();
        let dest = tempfile::tempdir().expect("scratch");
        extract_zip(&archive, dest.path()).expect("valid archive extracts");
        let binary = dest.path().join("waterui-inspector/waterui-inspector");
        assert!(binary.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                binary.metadata().unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
    }

    /// A staged "binary" that answers `--identity` with fixed JSON — the
    /// shape the real client prints.
    #[cfg(unix)]
    fn fake_client(machine: &TestMachine, identity: &InspectorBuildIdentity) -> (PathBuf, Host) {
        use std::os::unix::fs::PermissionsExt as _;
        let binary = machine.file(
            "bin/waterui-inspector",
            include_str!("testdata/fake_client.sh"),
        );
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let host = identity_host(machine, identity, []);
        (binary, host)
    }

    #[cfg(unix)]
    #[test]
    fn a_binary_reporting_the_expected_identity_passes() {
        let machine = TestMachine::new();
        let expected = expected_identity();
        let (binary, host) = fake_client(&machine, &expected);
        smol::block_on(verify_identity(&host, &binary, &expected, IDENTITY_TIMEOUT))
            .expect("matching identity verifies");
    }

    /// A cached or downloaded binary with a foreign identity is never
    /// launched — the probe fails preparation instead.
    #[cfg(unix)]
    #[test]
    fn a_foreign_identity_is_never_launched() {
        let machine = TestMachine::new();
        let foreign = InspectorBuildIdentity {
            framework_version: String::from("0.0.2"),
            ..expected_identity()
        };
        let (binary, host) = fake_client(&machine, &foreign);
        let error = smol::block_on(verify_identity(
            &host,
            &binary,
            &expected_identity(),
            IDENTITY_TIMEOUT,
        ))
        .expect_err("foreign identity rejected");
        assert!(error.to_string().contains("reports"), "{error}");
    }

    /// The probe runs without the session's credentials — the fixture exits
    /// nonzero if either leaks into its environment.
    #[cfg(unix)]
    #[test]
    fn the_probe_carries_no_credentials() {
        let machine = TestMachine::new();
        let expected = expected_identity();
        let (binary, _default_host) = fake_client(&machine, &expected);
        let host = identity_host(
            &machine,
            &expected,
            [
                ("WATERUI_FAKE_TOKEN_PROBE", std::ffi::OsString::from("1")),
                (
                    "WATERUI_INSPECTOR_TOKEN",
                    std::ffi::OsString::from("session-secret"),
                ),
                (
                    "WATERUI_INSPECTOR_TARGET_ADDR",
                    std::ffi::OsString::from("127.0.0.1:9"),
                ),
            ],
        );
        smol::block_on(verify_identity(&host, &binary, &expected, IDENTITY_TIMEOUT))
            .expect("stripped credentials verify");
    }

    /// A client that never answers `--identity` is killed when the deadline
    /// drops its child — the recorded pid is gone, not merely abandoned.
    #[cfg(unix)]
    #[test]
    fn a_hanging_identity_probe_is_killed() {
        let machine = TestMachine::new();
        let pid_file = machine.root().join("probe.pid");
        let (binary, _default_host) = fake_client(&machine, &expected_identity());
        let host = identity_host(
            &machine,
            &expected_identity(),
            [
                ("WATERUI_FAKE_HANG", std::ffi::OsString::from("1")),
                ("WATERUI_FAKE_PID_FILE", pid_file.as_os_str().to_os_string()),
            ],
        );
        let error = smol::block_on(verify_identity(
            &host,
            &binary,
            &expected_identity(),
            Duration::from_millis(200),
        ))
        .expect_err("the hung probe times out");
        assert!(error.to_string().contains("did not answer"), "{error}");
        // The child may not have started before the deadline — wait for the
        // fixture to have recorded its pid, then prove it is gone.
        let pid = smol::block_on(async {
            for _ in 0..100 {
                if pid_file.is_file() {
                    break;
                }
                smol::Timer::after(Duration::from_millis(10)).await;
            }
            std::fs::read_to_string(&pid_file).expect("the fixture recorded its pid")
        });
        // The dropped probe killed the child; reap_on_drop frees it, so a
        // bounded wait proves it is actually gone.
        smol::block_on(async {
            for _ in 0..50 {
                // `host.run` under the declared environment — no raw spawn.
                let gone = host
                    .run(
                        OsStr::new("/bin/kill"),
                        [OsStr::new("-0"), OsStr::new(pid.trim())],
                    )
                    .await
                    .is_err();
                if gone {
                    return;
                }
                smol::Timer::after(Duration::from_millis(20)).await;
            }
            panic!("the hung probe process {pid} was still alive after the timeout");
        });
    }

    // ----- release fetch + cache -----

    /// A fixture release that publishes the archive at `url` with the
    /// archive's real digest.
    fn valid_release_fixture(selected: &SelectedPackages, archive: &[u8]) -> (String, String) {
        let triple = Triple::host().to_string();
        let name = format!("{CLIENT_BINARY}-{triple}.zip");
        let url = format!(
            "https://github.com/water-rs/waterui/releases/download/v{}/{name}",
            selected.framework_version
        );
        let digest = hex::encode(sha2::Sha256::digest(archive));
        (url, digest)
    }

    /// One full fetch: fixture release record, archive, then a warm call —
    /// the host has no fixture left, so any second HTTP request panics.
    #[cfg(unix)]
    #[test]
    fn a_release_download_publishes_a_verified_cache_then_serves_it() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let archive = fixture_zip();
        let (url, digest) = valid_release_fixture(&selected, &archive);
        let name = format!("{CLIENT_BINARY}-{}.zip", Triple::host());

        let identity = expected_identity();
        let host = identity_host(&machine, &identity, [])
            .with_http_response(
                &api_url(&selected),
                release_json(serde_json::json!([asset_json(
                    &name,
                    &url,
                    Some(&format!("sha256:{digest}")),
                )])),
            )
            .with_http_response(&url, zenwave::Response::new(zenwave::Body::from(archive)));

        let artifact = smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect("first fetch");
        let package = cached_package(&machine, &selected);
        assert_eq!(artifact.path(), &package.join(binary_name()));
        assert!(package.join(CACHE_RECEIPT).is_file());

        // Warm cache: no HTTP fixture remains — a request would panic.
        smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect("warm cache serves");
    }

    #[test]
    fn a_missing_release_asset_fails_clearly() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let host = identity_host(&machine, &expected_identity(), []).with_http_response(
            &api_url(&selected),
            release_json(serde_json::json!([asset_json(
                "other.zip",
                "https://x",
                None
            )])),
        );
        let error = smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect_err("no asset");
        assert!(error.to_string().contains("publishes no"), "{error}");
    }

    #[test]
    fn duplicate_release_assets_fail() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let name = format!("{CLIENT_BINARY}-{}.zip", Triple::host());
        let host = identity_host(&machine, &expected_identity(), []).with_http_response(
            &api_url(&selected),
            release_json(serde_json::json!([
                asset_json(&name, "https://x/a.zip", None),
                asset_json(&name, "https://x/b.zip", None),
            ])),
        );
        let error = smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect_err("duplicate");
        assert!(error.to_string().contains("duplicate"), "{error}");
    }

    #[test]
    fn a_release_without_a_digest_is_rejected() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let name = format!("{CLIENT_BINARY}-{}.zip", Triple::host());
        let (url, _) = valid_release_fixture(&selected, b"");
        let host = identity_host(&machine, &expected_identity(), []).with_http_response(
            &api_url(&selected),
            release_json(serde_json::json!([asset_json(&name, &url, None)])),
        );
        let error = smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect_err("no digest");
        assert!(error.to_string().contains("sha256"), "{error}");
    }

    #[test]
    fn a_404_release_record_is_actionable() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let mut response = release_json(serde_json::json!([]));
        *response.status_mut() = zenwave::StatusCode::NOT_FOUND;
        let host = identity_host(&machine, &expected_identity(), [])
            .with_http_response(&api_url(&selected), response);
        let error =
            smol::block_on(fetch_release(&host, &selected, &expected_identity())).expect_err("404");
        assert!(error.to_string().contains("404"), "{error}");
    }

    #[test]
    fn a_hash_mismatch_never_extracts_or_publishes() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let name = format!("{CLIENT_BINARY}-{}.zip", Triple::host());
        let (url, _digest) = valid_release_fixture(&selected, b"");
        let wrong = "0".repeat(64);
        let host = identity_host(&machine, &expected_identity(), [])
            .with_http_response(
                &api_url(&selected),
                release_json(serde_json::json!([asset_json(
                    &name,
                    &url,
                    Some(&format!("sha256:{wrong}")),
                )])),
            )
            .with_http_response(
                &url,
                zenwave::Response::new(zenwave::Body::from(fixture_zip())),
            );
        let error = smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect_err("hash mismatch");
        assert!(error.to_string().contains("hashed"), "{error}");
        // Nothing published — not even an empty package dir.
        let cache = cached_package(&machine, &selected);
        assert!(!cache.exists(), "a failed fetch left {}", cache.display());
    }

    /// A binary whose `--identity` fails — or answers malformed JSON — is
    /// never published.
    #[cfg(unix)]
    #[test]
    fn a_wrong_identity_never_publishes() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let archive = fixture_zip();
        let (url, digest) = valid_release_fixture(&selected, &archive);
        let name = format!("{CLIENT_BINARY}-{}.zip", Triple::host());
        // The fixture binary reports a foreign identity.
        let foreign = InspectorBuildIdentity {
            framework_version: String::from("0.0.2"),
            ..expected_identity()
        };
        let host = identity_host(&machine, &foreign, [])
            .with_http_response(
                &api_url(&selected),
                release_json(serde_json::json!([asset_json(
                    &name,
                    &url,
                    Some(&format!("sha256:{digest}")),
                )])),
            )
            .with_http_response(&url, zenwave::Response::new(zenwave::Body::from(archive)));
        let error = smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect_err("foreign");
        assert!(error.to_string().contains("reports"), "{error}");
        assert!(!cached_package(&machine, &selected).exists());
    }

    #[test]
    fn a_cached_package_without_a_receipt_fails() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let package = cached_package(&machine, &selected);
        std::fs::create_dir_all(&package).expect("package dir");
        let host = identity_host(&machine, &expected_identity(), []);
        let error = smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect_err("no receipt");
        assert!(error.to_string().contains("receipt"), "{error}");
    }

    #[test]
    fn a_cached_package_without_a_binary_fails() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let package = cached_package(&machine, &selected);
        std::fs::create_dir_all(&package).expect("package dir");
        let receipt = CachedClient {
            binary_sha256: "0".repeat(64),
        };
        std::fs::write(
            package.join(CACHE_RECEIPT),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .expect("receipt");
        let host = identity_host(&machine, &expected_identity(), []);
        let error = smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect_err("no binary");
        assert!(error.to_string().contains("no"), "{error}");
    }

    /// A cached binary whose hash does not match the receipt is never run.
    #[cfg(unix)]
    #[test]
    fn a_cached_binary_with_a_wrong_hash_never_executes() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let package = cached_package(&machine, &selected);
        std::fs::create_dir_all(&package).expect("package dir");
        let binary = package.join(binary_name());
        std::fs::write(&binary, b"tampered").expect("binary");
        let receipt = CachedClient {
            binary_sha256: "0".repeat(64),
        };
        std::fs::write(
            package.join(CACHE_RECEIPT),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .expect("receipt");
        let host = identity_host(&machine, &expected_identity(), []);
        let error = smol::block_on(fetch_release(&host, &selected, &expected_identity()))
            .expect_err("tampered");
        assert!(error.to_string().contains("does not match"), "{error}");
    }

    /// Dropping the fetch mid-flight — here, while the staged binary's
    /// hanging `--identity` probe suspends the pipeline — removes the
    /// staging directory and publishes nothing.
    #[cfg(unix)]
    #[test]
    fn an_abandoned_fetch_publishes_nothing() {
        let machine = TestMachine::new();
        let selected = registry_selected();
        let archive = fixture_zip();
        let (url, digest) = valid_release_fixture(&selected, &archive);
        let name = format!("{CLIENT_BINARY}-{}.zip", Triple::host());
        let pid_file = machine.root().join("probe.pid");
        let host = identity_host(
            &machine,
            &expected_identity(),
            [
                ("WATERUI_FAKE_HANG", std::ffi::OsString::from("1")),
                ("WATERUI_FAKE_PID_FILE", pid_file.as_os_str().to_os_string()),
            ],
        )
        .with_http_response(
            &api_url(&selected),
            release_json(serde_json::json!([asset_json(
                &name,
                &url,
                Some(&format!("sha256:{digest}")),
            )])),
        )
        .with_http_response(&url, zenwave::Response::new(zenwave::Body::from(archive)));

        let stage_parent = cached_package(&machine, &selected)
            .parent()
            .expect("stage parent")
            .to_path_buf();
        smol::block_on(async {
            let expected = expected_identity();
            let mut fetch = std::pin::pin!(fetch_release(&host, &selected, &expected));
            // Drive the fetch until the staged fixture binary's hanging
            // probe records its pid, then drop the fetch mid-verification.
            for _ in 0..250 {
                if pid_file.is_file() {
                    break;
                }
                select(
                    fetch.as_mut(),
                    std::pin::pin!(smol::Timer::after(Duration::from_millis(20))),
                )
                .await;
            }
            // The pinned fetch drops when this block ends — its stage
            // directory is removed by the TempDir it still holds.
        });
        assert!(pid_file.is_file(), "the probe never started");
        assert!(
            !cached_package(&machine, &selected).exists(),
            "an abandoned fetch left a package"
        );
        let leftovers: Vec<std::fs::DirEntry> = std::fs::read_dir(&stage_parent)
            .map(|entries| entries.flatten().collect())
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "stage dirs leaked: {leftovers:?}");
    }

    /// `resolve_packages` reads what the project actually resolved — a
    /// `waterui = "0.5"` declaration whose lockfile carries `0.5.2` selects
    /// release `v0.5.2`, not a tag derived from the declared range.
    #[test]
    fn the_resolved_version_selects_the_release() {
        let machine = TestMachine::new();
        machine.install("cargo");
        let dir = machine.dir("project");
        machine.respond(
            "CARGO_METADATA",
            &serde_json::json!({
                "packages": [
                    package_json("the-app", "0.1.0", None, &dir.join("Cargo.toml")),
                    package_json("waterui", "0.5.2", Some("registry+https://github.com/rust-lang/crates.io-index"), &dir.join("x/Cargo.toml")),
                    package_json("waterui-inspector-protocol", "0.5.1", Some("registry+https://github.com/rust-lang/crates.io-index"), &dir.join("y/Cargo.toml")),
                ],
                "workspace_members": ["path+file://app#0.1.0"],
                "resolve": null,
                "target_directory": "/tmp/fixture/target",
                "version": 0,
                "workspace_root": dir,
                "metadata": {},
            })
            .to_string(),
        );
        let host = machine.host(Vec::<(String, String)>::new());
        let selected = smol::block_on(resolve_packages(&host, &dir)).expect("resolved");
        assert_eq!(selected.framework_version, "0.5.2");
        assert_eq!(selected.protocol_version, "0.5.1");
        assert!(api_url(&selected).ends_with("/v0.5.2"));
        assert!(matches!(selected.source, FacadeSource::Registry));
    }

    /// Building the real `apps/inspector` from this checkout — an
    /// end-to-end preparation check run manually.
    ///
    /// ```console
    /// cargo nextest run -p waterui-cli --run-ignored ignored-only \
    ///   -E 'test(preparing_from_this_checkout)'
    /// ```
    #[test]
    #[ignore = "builds the framework's inspector source"]
    fn preparing_from_this_checkout_builds_and_verifies_the_client() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("cli/ has a workspace root")
            .to_path_buf();
        let host = Host::current();
        // The endpoint this client would serve reports this checkout's own
        // revision and the resolved package versions — resolved through the
        // same production path the launcher takes.
        let selected = smol::block_on(resolve_packages(&host, &workspace))
            .expect("the enclosing workspace resolves waterui");
        let head = smol::block_on(checkout_head(&host, &workspace))
            .expect("the enclosing checkout has a HEAD");
        let expected = InspectorBuildIdentity {
            framework_version: selected.framework_version,
            protocol: InspectorProtocolInfo {
                version: selected.protocol_version,
                build_commit: Some(head),
            },
        };
        let artifact = smol::block_on(prepare_client(&host, &workspace, &expected, None))
            .expect("prepare_client on this checkout");
        assert!(artifact.path().is_file());
        let package_dir = artifact.path().parent().expect("package dir");
        assert!(
            package_dir.join("resources/waterui_assets").is_dir(),
            "the packaged client carries its asset directory"
        );
        assert!(
            package_dir.join("resources/fonts").is_dir(),
            "the packaged client carries its declared fonts"
        );
        // A second preparation of the same bytes reuses the published
        // package — same path, same binary.
        let second = smol::block_on(prepare_client(&host, &workspace, &expected, None))
            .expect("the warm preparation serves");
        assert_eq!(artifact.path(), second.path());
    }

    /// An endpoint built at `HEAD_B` rejects a checkout sitting at `HEAD_A` —
    /// before any build runs.
    #[test]
    fn an_unreleased_endpoint_rejects_a_different_checkout_revision() {
        let machine = TestMachine::new();
        machine.install("cargo");
        machine.install("git");
        let root = machine.dir("checkout");
        machine.file(
            "checkout/apps/inspector/Cargo.toml",
            "[package]\nname = \"waterui-inspector-app\"\n",
        );
        let project = machine.dir("project");
        machine.respond(
            "CARGO_METADATA",
            &serde_json::json!({
                "packages": [
                    package_json("the-app", "0.1.0", None, &project.join("Cargo.toml")),
                    package_json("waterui", "0.5.2", None, &root.join("Cargo.toml")),
                    package_json(
                        "waterui-inspector-protocol",
                        "0.5.1",
                        None,
                        &root.join("components/devtools/inspector/protocol/Cargo.toml")
                    ),
                ],
                "workspace_members": ["path+file://app#0.1.0"],
                "resolve": null,
                "target_directory": "/tmp/fixture/target",
                "version": 0,
                "workspace_root": project,
                "metadata": {},
            })
            .to_string(),
        );
        // The checkout sits at a different revision than the endpoint's.
        machine.respond("GIT_HEAD", &"a".repeat(40));
        let host = machine.host(Vec::<(String, String)>::new());
        let identity = InspectorBuildIdentity {
            framework_version: String::from("0.5.2"),
            protocol: InspectorProtocolInfo {
                version: String::from("0.5.1"),
                build_commit: Some("b".repeat(40)),
            },
        };
        let error = smol::block_on(prepare_client(&host, &project, &identity, None))
            .expect_err("a different revision is rejected before the build");
        assert!(error.to_string().contains("was built at"), "{error}");
    }

    /// Same checkout, same revision — but a different resolved version
    /// tuple fails the endpoint's identity just as early.
    #[test]
    fn an_unreleased_endpoint_rejects_a_version_tuple_mismatch() {
        let machine = TestMachine::new();
        machine.install("cargo");
        machine.install("git");
        let root = machine.dir("checkout");
        machine.file(
            "checkout/apps/inspector/Cargo.toml",
            "[package]\nname = \"waterui-inspector-app\"\n",
        );
        let project = machine.dir("project");
        machine.respond(
            "CARGO_METADATA",
            &serde_json::json!({
                "packages": [
                    package_json("the-app", "0.1.0", None, &project.join("Cargo.toml")),
                    package_json("waterui", "0.9.9", None, &root.join("Cargo.toml")),
                    package_json(
                        "waterui-inspector-protocol",
                        "0.5.1",
                        None,
                        &root.join("components/devtools/inspector/protocol/Cargo.toml")
                    ),
                ],
                "workspace_members": ["path+file://app#0.1.0"],
                "resolve": null,
                "target_directory": "/tmp/fixture/target",
                "version": 0,
                "workspace_root": project,
                "metadata": {},
            })
            .to_string(),
        );
        let head = "a".repeat(40);
        machine.respond("GIT_HEAD", &head);
        let host = machine.host(Vec::<(String, String)>::new());
        let identity = InspectorBuildIdentity {
            framework_version: String::from("0.5.2"),
            protocol: InspectorProtocolInfo {
                version: String::from("0.5.1"),
                build_commit: Some(head),
            },
        };
        let error = smol::block_on(prepare_client(&host, &project, &identity, None))
            .expect_err("a version mismatch is rejected before the build");
        assert!(error.to_string().contains("select a matching"), "{error}");
    }
    /// A released endpoint — `build_commit: None` — is served from the
    /// release its identity names; the project's manifest is never read,
    /// and no `cargo` or `git` exists on this host to answer if it were.
    #[cfg(unix)]
    #[test]
    fn a_released_endpoint_never_reads_the_project() {
        let machine = TestMachine::new();
        let identity = expected_identity();
        let selected = registry_selected();
        let archive = fixture_zip();
        let (url, digest) = valid_release_fixture(&selected, &archive);
        let name = format!("{CLIENT_BINARY}-{}.zip", Triple::host());
        let host = identity_host(&machine, &identity, [])
            .with_http_response(
                &api_url(&selected),
                release_json(serde_json::json!([asset_json(
                    &name,
                    &url,
                    Some(&format!("sha256:{digest}")),
                )])),
            )
            .with_http_response(&url, zenwave::Response::new(zenwave::Body::from(archive)));

        // An empty directory stands in for the project — a metadata call
        // would fail on spawn, so a returned artifact proves it never ran.
        let empty = machine.dir("no-project");
        let artifact = smol::block_on(prepare_client(&host, &empty, &identity, None))
            .expect("a released endpoint needs no project");
        assert!(artifact.path().is_file());
    }

    /// An endpoint reporting a checkout revision must not be served by a
    /// registry `waterui` — the mismatch fails before any build or fetch.
    #[test]
    fn an_unreleased_endpoint_rejects_a_registry_project() {
        let machine = TestMachine::new();
        machine.install("cargo");
        let dir = machine.dir("project");
        machine.respond(
            "CARGO_METADATA",
            &serde_json::json!({
                "packages": [
                    package_json("the-app", "0.1.0", None, &dir.join("Cargo.toml")),
                    package_json("waterui", "0.5.2", Some("registry+https://github.com/rust-lang/crates.io-index"), &dir.join("x/Cargo.toml")),
                    package_json("waterui-inspector-protocol", "0.5.1", Some("registry+https://github.com/rust-lang/crates.io-index"), &dir.join("y/Cargo.toml")),
                ],
                "workspace_members": ["path+file://app#0.1.0"],
                "resolve": null,
                "target_directory": "/tmp/fixture/target",
                "version": 0,
                "workspace_root": dir,
                "metadata": {},
            })
            .to_string(),
        );
        let host = machine.host(Vec::<(String, String)>::new());
        let identity = InspectorBuildIdentity {
            framework_version: String::from("0.5.2"),
            protocol: InspectorProtocolInfo {
                version: String::from("0.5.1"),
                build_commit: Some("a".repeat(40)),
            },
        };
        let error = smol::block_on(prepare_client(&host, &dir, &identity, None))
            .expect_err("a registry project cannot serve a checkout endpoint");
        assert!(error.to_string().contains("registry"), "{error}");
    }

    // ----- release packaging -----

    /// A staged package body: binary, a declared font, the asset dir, a
    /// runtime sibling, and a deliberate cache receipt the exporter must
    /// not carry into the archive.
    fn staged_package(machine: &TestMachine) -> PathBuf {
        let package = machine.dir("package");
        std::fs::create_dir_all(package.join("resources/fonts")).unwrap();
        std::fs::create_dir_all(package.join("resources/waterui_assets")).unwrap();
        std::fs::write(package.join(binary_name()), b"#!binary").unwrap();
        std::fs::write(package.join("resources/fonts/F.ttf"), b"font bytes").unwrap();
        std::fs::write(package.join("resources/waterui_assets/index"), b"assets").unwrap();
        std::fs::write(package.join("dxc.dll"), b"runtime sibling").unwrap();
        std::fs::write(package.join(CACHE_RECEIPT), br#"{"binary_sha256":"x"}"#).unwrap();
        package
    }

    /// The entry names inside an archive, in the order they were written.
    fn archive_names(archive: &Path) -> Vec<String> {
        zip::ZipArchive::new(std::fs::File::open(archive).unwrap())
            .unwrap()
            .file_names()
            .map(str::to_owned)
            .collect()
    }

    /// The archive holds every package file under `waterui-inspector/` with
    /// '/' separators, byte-identical, and carries no cache receipt.
    #[test]
    fn a_release_archive_roundtrips_the_package() {
        let machine = TestMachine::new();
        let package = staged_package(&machine);
        let out = machine.dir("out");
        let archive =
            smol::block_on(write_release_archive(&package, &out)).expect("archive written");
        assert_eq!(
            archive.file_name().unwrap(),
            release_archive_name().as_str(),
            "the contract name waterui-inspector-<triple>.zip"
        );

        let names = archive_names(&archive);
        assert!(
            names
                .iter()
                .all(|name| name.starts_with("waterui-inspector/")),
            "every entry lives under the package root: {names:?}"
        );
        assert!(
            names.iter().all(|name| !name.contains('\\')),
            "entry names use '/' separators: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.ends_with(CACHE_RECEIPT)),
            "the receipt never enters a release archive: {names:?}"
        );

        let extracted = machine.dir("extracted");
        extract_zip(&std::fs::read(&archive).unwrap(), &extracted).expect("the archive extracts");
        let root = extracted.join(CLIENT_PACKAGE);
        assert_eq!(
            std::fs::read(root.join(binary_name())).unwrap(),
            b"#!binary"
        );
        assert_eq!(
            std::fs::read(root.join("resources/fonts/F.ttf")).unwrap(),
            b"font bytes"
        );
        assert_eq!(
            std::fs::read(root.join("resources/waterui_assets/index")).unwrap(),
            b"assets"
        );
        assert_eq!(
            std::fs::read(root.join("dxc.dll")).unwrap(),
            b"runtime sibling"
        );
    }

    /// The executable mode survives the archive on platforms that carry
    /// one.
    #[cfg(unix)]
    #[test]
    fn a_release_archive_preserves_the_executable_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let machine = TestMachine::new();
        let package = staged_package(&machine);
        std::fs::set_permissions(
            package.join(binary_name()),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let out = machine.dir("out");
        let archive =
            smol::block_on(write_release_archive(&package, &out)).expect("archive written");

        let extracted = machine.dir("extracted");
        extract_zip(&std::fs::read(&archive).unwrap(), &extracted).expect("the archive extracts");
        assert_eq!(
            extracted
                .join(CLIENT_PACKAGE)
                .join(binary_name())
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "the executable bit survives the archive"
        );
    }

    /// A missing output directory is created so the archive lands.
    #[test]
    fn a_fresh_output_directory_is_created() {
        let machine = TestMachine::new();
        let package = staged_package(&machine);
        let out = machine.root().join("nested/out");
        let archive = smol::block_on(write_release_archive(&package, &out))
            .expect("the output directory is created");
        assert!(archive.is_file(), "the archive exists at {archive:?}");
    }

    /// An existing destination is never truncated — the error carries the
    /// refusal and the file keeps its bytes.
    #[test]
    fn a_release_archive_refuses_to_overwrite() {
        let machine = TestMachine::new();
        let package = staged_package(&machine);
        let out = machine.dir("out");
        let existing = out.join(release_archive_name());
        std::fs::write(&existing, b"previous release bytes").unwrap();

        let error = smol::block_on(write_release_archive(&package, &out))
            .expect_err("an existing archive refuses");
        assert!(
            format!("{error:#}").contains("refusing to overwrite"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&existing).unwrap(),
            b"previous release bytes",
            "the existing archive is untouched"
        );
        // And no staged leftovers sit beside it.
        let leftovers: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "only the preexisting archive remains");
    }

    /// A source that fails mid-archive leaves no final file — the staged
    /// temporary is the worker's and is dropped with it.
    #[cfg(unix)]
    #[test]
    fn a_broken_source_never_exposes_a_partial_archive() {
        let machine = TestMachine::new();
        let package = staged_package(&machine);
        // A broken symlink's metadata read fails inside the writer.
        std::os::unix::fs::symlink(package.join("missing-target"), package.join("dangling"))
            .unwrap();
        let out = machine.dir("out");
        let _ = smol::block_on(write_release_archive(&package, &out))
            .expect_err("a broken source fails the archive");
        assert!(
            !out.join(release_archive_name()).exists(),
            "no partial archive was published"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&out).unwrap().collect();
        assert!(leftovers.is_empty(), "no staged leftovers either");
    }

    /// A staged binary that reports a foreign identity refuses before the
    /// archive exists — verification gates publication.
    #[cfg(unix)]
    #[test]
    fn a_wrong_identity_never_archives() {
        use std::os::unix::fs::PermissionsExt as _;
        let machine = TestMachine::new();
        let package = staged_package(&machine);
        let binary = package.join(binary_name());
        std::fs::write(&binary, include_bytes!("testdata/fake_client.sh")).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = machine.dir("out");
        let mut foreign = expected_identity();
        foreign.framework_version = String::from("9.9.9");
        let host = identity_host(&machine, &foreign, []);
        let error = smol::block_on(package_staged_client(
            &host,
            &package,
            &expected_identity(),
            &out,
        ))
        .expect_err("a foreign binary refuses");
        assert!(format!("{error:#}").contains("reports"), "{error:#}");
        assert!(
            !out.join(release_archive_name()).exists(),
            "no archive was written"
        );
    }

    /// The release build asks cargo for `--release` and stamps the release
    /// identity; a debug build passes neither — and a mode the caller's
    /// environment happened to set is overridden, never inherited.
    #[test]
    fn the_release_build_runs_cargo_in_release_mode() {
        let machine = TestMachine::new();
        machine.install("cargo");
        let log = machine.root().join("cargo.log");
        let root = machine.dir("checkout");
        std::fs::create_dir_all(root.join("apps/inspector")).unwrap();
        // Enough metadata for the build's own target-directory resolution.
        machine.respond(
            "CARGO_METADATA",
            &serde_json::json!({
                "packages": [],
                "workspace_members": [],
                "workspace_default_members": [],
                "resolve": null,
                "workspace_root": root,
                "workspace_metadata": null,
                "target_directory": machine.dir("target"),
                "version": 1,
            })
            .to_string(),
        );

        // `rustc` fails on the missing artifact — the argv/env log it left
        // is the assertion.
        let host = machine.host([
            (
                "WATERUI_FAKE_CARGO_LOG".to_string(),
                log.display().to_string(),
            ),
            // A hostile inherited mode: the release stamp must override it.
            (
                "WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE".to_string(),
                "checkout".to_string(),
            ),
        ]);
        let _ = smol::block_on(build_and_stage(&host, &root, true, None))
            .expect_err("the fake cargo emits no artifact");
        let logged = std::fs::read_to_string(&log).unwrap();
        assert!(
            logged.contains("--release"),
            "the release build passes --release: {logged}"
        );
        assert!(
            logged.contains("mode: release"),
            "the release stamp overrides the host's checkout mode: {logged}"
        );

        std::fs::remove_file(&log).unwrap();
        let host = machine.host([(
            "WATERUI_FAKE_CARGO_LOG".to_string(),
            log.display().to_string(),
        )]);
        let _ = smol::block_on(build_and_stage(&host, &root, false, None))
            .expect_err("the fake cargo emits no artifact");
        let logged = std::fs::read_to_string(&log).unwrap();
        let rustc_invocations: Vec<&str> = logged
            .lines()
            .filter(|line| line.starts_with("argv: rustc"))
            .collect();
        assert!(
            !rustc_invocations.is_empty()
                && rustc_invocations
                    .iter()
                    .all(|line| !line.contains("--release")),
            "the checkout build stays debug: {logged}"
        );
        assert!(
            logged.contains("mode: checkout"),
            "debug stamps checkout: {logged}"
        );
    }
}
