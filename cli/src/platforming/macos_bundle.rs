//! Helpers for packaging native binaries into macOS `.app` bundles.

#[cfg(target_os = "macos")]
use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use askama::Template;
#[cfg(target_os = "macos")]
use eyre::Context as _;
use eyre::bail;
use fs_extra::dir::CopyOptions;
use smol::fs;
#[cfg(target_os = "macos")]
use smol::stream::StreamExt as _;

// `copy_file` is used by `package_binary_as_app`, which compiles on every
// host; only the codesign helpers below are macOS-gated.
use crate::project_types::AppleBundleIdentifier;
#[cfg(target_os = "macos")]
use crate::toolchain::Host;
use crate::utils::copy_file;

#[cfg(target_os = "macos")]
const CEF_HELPER_VARIANTS: [(&str, &str); 5] = [
    ("", ""),
    (" (Alerts)", ".alerts"),
    (" (GPU)", ".gpu"),
    (" (Plugin)", ".plugin"),
    (" (Renderer)", ".renderer"),
];

#[derive(Template)]
#[template(path = "macos/Info.plist.tpl", escape = "none")]
struct InfoPlistTemplate<'a> {
    bundle_identifier: &'a str,
    app_name: &'a str,
    executable_name: &'a str,
    usage_descriptions: &'a [MacOsUsageDescription],
}

#[cfg(target_os = "macos")]
#[derive(Template)]
#[template(path = "macos/CefHelperInfo.plist.tpl", escape = "none")]
struct CefHelperInfoPlistTemplate<'a> {
    bundle_identifier: &'a str,
    helper_name: &'a str,
    product_name: &'a str,
}

/// Apple Info.plist usage-description entry for a macOS app bundle.
#[derive(Debug, Clone)]
pub struct MacOsUsageDescription {
    /// Raw Info.plist key such as `NSCameraUsageDescription`.
    pub plist_key: &'static str,
    /// User-facing reason declared in `Water.toml`.
    pub description: String,
}

/// The two names a macOS `.app` bundle carries.
#[derive(Debug, Clone, Copy)]
pub struct MacOsAppNames<'a> {
    /// Human-readable bundle name — `<app_name>.app` and `CFBundleName`.
    pub app_name: &'a str,
    /// Shipped `Contents/MacOS` and `CFBundleExecutable` name — the product
    /// name, not the artifact's file name, which may carry a build-internal
    /// tag.
    pub executable_name: &'a str,
}

/// Package a compiled binary as a macOS `.app` bundle.
///
/// `resources_dir` is optional and copied to `Contents/Resources` when
/// present. `icns` is the encoded app-icon family, written as
/// `Contents/Resources/AppIcon.icns` and referenced from `Info.plist`.
///
/// # Errors
/// Returns an error if the binary is missing, template rendering fails, or bundle files cannot be created.
pub async fn package_binary_as_app(
    binary_path: &Path,
    bundle_id: &AppleBundleIdentifier,
    names: MacOsAppNames<'_>,
    usage_descriptions: &[MacOsUsageDescription],
    resources_dir: Option<&Path>,
    icns: &[u8],
    output_root: &Path,
) -> eyre::Result<PathBuf> {
    if !binary_path.exists() {
        bail!(
            "Binary not found at {}. Build must succeed before packaging.",
            binary_path.display()
        );
    }

    let app_dir = output_root.join(format!("{}.app", names.app_name));
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let bundle_resources_dir = contents_dir.join("Resources");
    if app_dir.exists() {
        fs::remove_dir_all(&app_dir).await?;
    }
    fs::create_dir_all(&macos_dir).await?;
    fs::create_dir_all(&bundle_resources_dir).await?;

    let executable_dest = macos_dir.join(names.executable_name);
    copy_file(binary_path, &executable_dest).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&executable_dest).await?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&executable_dest, perms).await?;
    }

    if let Some(src_resources) = resources_dir
        && src_resources.exists()
    {
        copy_dir(src_resources, &bundle_resources_dir).await?;
    }

    fs::write(bundle_resources_dir.join("AppIcon.icns"), icns).await?;

    let plist = InfoPlistTemplate {
        bundle_identifier: bundle_id.as_str(),
        app_name: names.app_name,
        executable_name: names.executable_name,
        usage_descriptions,
    }
    .render()
    .map_err(|error| eyre::eyre!("Failed to render Info.plist template: {error}"))?;
    fs::write(contents_dir.join("Info.plist"), plist).await?;

    Ok(app_dir)
}

/// How a packaged macOS `.app` is code-signed.
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub enum MacOsSigning {
    /// A development signature: ad hoc, or the first installed development
    /// identity when the app needs a stable one.
    Development {
        /// Whether the app declares protected resources and therefore needs a
        /// stable identity so macOS can persist privacy grants across rebuilds.
        requires_stable_identity: bool,
    },
    /// A distribution signature for delivery outside the developer's
    /// machine: Developer ID, hardened runtime, secure timestamp, then
    /// notarization and stapling. It never degrades to an ad hoc signature.
    Distribution(DistributionSigning),
}

/// What a macOS distribution package signs and notarizes with.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct DistributionSigning {
    /// The Apple Developer team the package signs for — the certificate
    /// subject OU the Developer ID Application identity must carry.
    pub team_id: String,
    /// The `xcrun notarytool` keychain profile the submission authenticates
    /// with.
    pub notary_profile: String,
    /// The entitlements applied to the outermost bundle, when the project
    /// declares any.
    pub entitlements: Option<PathBuf>,
}

#[cfg(target_os = "macos")]
impl DistributionSigning {
    /// The distribution signing configuration the project's `Water.toml`
    /// declares in `[signing.macos]`.
    ///
    /// `notary_profile` names the `notarytool` keychain profile the
    /// submission authenticates with and `team_id` the team whose Developer
    /// ID Application certificate signs.
    ///
    /// # Errors
    /// Returns an error naming the `[signing.macos]` section to add — and the
    /// `xcrun notarytool store-credentials` command that creates the keychain
    /// profile it references — when the manifest carries no such table.
    pub fn from_manifest(
        config: Option<&crate::project::MacosSigningConfig>,
        entitlements: Option<PathBuf>,
    ) -> eyre::Result<Self> {
        let config = config.ok_or_else(|| {
            eyre::eyre!(
                "macOS distribution packaging is configured per project in Water.toml:\n  \
                 [signing.macos]\n  \
                 team_id = \"<your-team-id>\"\n  \
                 notary_profile = \"<profile-name>\"\n\
                 `notary_profile` names the keychain profile holding the App \
                 Store Connect credentials `notarytool` authenticates with; \
                 create it with\n  \
                 xcrun notarytool store-credentials \"<profile-name>\" \\\n                     --apple-id <apple-id> --team-id <team-id> --password <app-specific-password>"
            )
        })?;
        Ok(Self {
            team_id: config.team_id.clone(),
            notary_profile: config.notary_profile.clone(),
            entitlements,
        })
    }
}

/// Signs a macOS app bundle for development or distribution.
///
/// Development keeps its local behavior: an ad hoc signature, or the first
/// installed development identity for an app declaring protected resources so
/// macOS can persist privacy grants across rebuilds (TCC records grants
/// against the code signature; an ad hoc one changes every build).
///
/// A distribution package is signed inside out — every nested framework and
/// helper before the bundle itself — with the configured team's Developer ID
/// Application certificate, the hardened runtime and a secure timestamp, then
/// notarized with `notarytool` and stapled. A missing identity or keychain
/// profile fails with the setup step it needs; a distribution build never
/// falls back to an ad hoc signature.
///
/// # Errors
///
/// Returns an error when the required identity or notarization credentials
/// are missing, when `security`/`codesign` cannot inspect or sign the
/// bundle, or when notarization rejects the app.
#[cfg(target_os = "macos")]
pub async fn sign_macos_app(
    host: &crate::toolchain::Host,
    app_path: &Path,
    bundle_id: &AppleBundleIdentifier,
    signing: &MacOsSigning,
) -> eyre::Result<()> {
    match signing {
        MacOsSigning::Development {
            requires_stable_identity,
        } => {
            let identities = host
                .run("security", ["find-identity", "-v", "-p", "codesigning"])
                .await?;
            let identity = if *requires_stable_identity {
                first_codesigning_identity(&identities).map_or_else(
                    || {
                        tracing::warn!(
                            "no code-signing identity installed; signing ad hoc — \
                             privacy grants will be requested again after every rebuild"
                        );
                        "-"
                    },
                    |identity| identity,
                )
            } else {
                "-"
            };
            let plan = codesign_plan(host, app_path, identity, bundle_id.as_str(), None).await?;
            run_sign_plan(host, plan).await
        }
        MacOsSigning::Distribution(distribution) => {
            let identity = developer_id_identity(host, &distribution.team_id).await?;
            let plan = codesign_plan(
                host,
                app_path,
                &identity,
                bundle_id.as_str(),
                Some(distribution),
            )
            .await?;
            run_sign_plan(host, plan).await?;
            notarize_app(host, app_path, &distribution.notary_profile).await?;
            staple_app(host, app_path).await
        }
    }
}

/// Signs the libraries staged into a device bundle's `Frameworks/` directory.
///
/// `xcodebuild` signs what it embeds; `water package` copies the Rust dylibs
/// into `Frameworks/` afterwards, so they reach a device with no signature at
/// all and `dyld` refuses them. Each staged file is re-signed with the leaf
/// identity Xcode used for the app itself, read back from the app's
/// `Authority` chain — the generated project cannot know which of the
/// keychain's development certificates automatic signing resolved.
///
/// # Errors
///
/// Returns an error if `codesign` cannot read the app's signature or sign a
/// staged library.
#[cfg(target_os = "macos")]
pub async fn sign_staged_device_libraries(
    host: &crate::toolchain::Host,
    app_path: &Path,
    frameworks_dir: &Path,
) -> eyre::Result<()> {
    if !frameworks_dir.exists() {
        return Ok(());
    }

    // `codesign` reports the signature on stderr, and the `Authority=` chain
    // only appears at `-vvv`; its first line is the leaf certificate that
    // signed the app.
    let output = host
        .output(
            "codesign",
            [std::ffi::OsStr::new("-dvvv"), app_path.as_os_str()],
        )
        .await?;
    let info = String::from_utf8_lossy(&output.stderr);
    let identity = info
        .lines()
        .find_map(|line| line.strip_prefix("Authority="))
        .ok_or_else(|| {
            eyre::eyre!(
                "codesign reported no signing authority for {}",
                app_path.display()
            )
        })?;

    let mut staged_paths = Vec::new();
    let mut entries = fs::read_dir(frameworks_dir).await?;
    while let Some(entry) = entries.next().await {
        let path = entry?.path();
        if path.is_file()
            || matches!(
                path.extension().and_then(std::ffi::OsStr::to_str),
                Some("app" | "framework")
            )
        {
            staged_paths.push(path);
        }
    }
    staged_paths.sort();
    for staged in staged_paths {
        host.run(
            "codesign",
            codesign_arguments(&staged, identity, Seal::Development, None, None),
        )
        .await?;
    }
    Ok(())
}

/// The seal `codesign` applies: a development signature carries no timestamp,
/// a distribution signature enables the hardened runtime and requests a
/// secure timestamp — both are what the notarization service requires of
/// every signed item in the bundle.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seal {
    /// `--timestamp=none`: a local signature that never leaves the machine.
    Development,
    /// `--options runtime --timestamp`: Developer ID sealing.
    Distribution,
}

/// The entitlements a nested-code item's signature declares on top of the
/// hardened-runtime seal. Requirements are modeled per item: only the CEF
/// helper variants whose process type needs V8 carry an entitlement, and
/// nothing else inherits the app's entitlements.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NestedEntitlements {
    /// `com.apple.security.cs.allow-jit` on the `<app> Helper (GPU).app` and
    /// `<app> Helper (Renderer).app` bundles CEF requires: V8 compiles
    /// JavaScript in those two process types. Upstream Chromium grants them
    /// exactly this one entitlement (`chrome/app/helper-gpu-entitlements.plist`
    /// and `helper-renderer-entitlements.plist`, mirrored by the pinned CEF
    /// release); the default, plugin and alerts helpers carry none.
    CefJit,
}

/// The entitlement kind `path` is, when it is one of the CEF helper variant
/// bundles `package_cef_helper_app` creates under `Contents/Frameworks`.
#[cfg(target_os = "macos")]
fn nested_entitlements(path: &Path) -> Option<NestedEntitlements> {
    let name = path.file_name().and_then(OsStr::to_str)?;
    (name.ends_with(" Helper (GPU).app") || name.ends_with(" Helper (Renderer).app"))
        .then_some(NestedEntitlements::CefJit)
}

/// The helper-variant entitlement plists generated for one sign run, keeping
/// their temporary directory alive until every `codesign` call has run.
#[cfg(target_os = "macos")]
#[derive(Debug)]
struct HelperEntitlements {
    _scratch: tempfile::TempDir,
    cef_jit: PathBuf,
}

#[cfg(target_os = "macos")]
impl HelperEntitlements {
    fn new(host: &crate::toolchain::Host) -> eyre::Result<Self> {
        let scratch = tempfile::tempdir_in(host.temp_dir())
            .wrap_err("a scratch directory for helper entitlements must be writable")?;
        let cef_jit = scratch.path().join("cef-helper.entitlements");
        plist::Value::Dictionary(plist::Dictionary::from_iter([(
            String::from("com.apple.security.cs.allow-jit"),
            plist::Value::Boolean(true),
        )]))
        .to_file_xml(&cef_jit)
        .wrap_err_with(|| format!("failed to write {}", cef_jit.display()))?;
        Ok(Self {
            _scratch: scratch,
            cef_jit,
        })
    }
}

/// The built `codesign` invocation list plus the generated entitlements it
/// references.
#[cfg(target_os = "macos")]
#[derive(Debug)]
struct CodesignPlan {
    invocations: Vec<Vec<OsString>>,
    helpers: Option<HelperEntitlements>,
}

/// The `codesign` invocations sealing `app_path` inside out: every nested
/// code path first — each bundle after the code it itself carries — and the
/// app last, with its identifier and, for distribution, its entitlements.
///
/// The discovery walk is blocking filesystem work, so the whole plan builds
/// on `smol::unblock` rather than the async executor.
#[cfg(target_os = "macos")]
async fn codesign_plan(
    host: &Host,
    app_path: &Path,
    identity: &str,
    bundle_id: &str,
    distribution: Option<&DistributionSigning>,
) -> eyre::Result<CodesignPlan> {
    let app_path = app_path.to_path_buf();
    let identity = identity.to_owned();
    let bundle_id = bundle_id.to_owned();
    let seal = if distribution.is_some() {
        Seal::Distribution
    } else {
        Seal::Development
    };
    let app_entitlements = distribution.and_then(|d| d.entitlements.clone());
    let host = host.clone();
    smol::unblock(move || {
        let mut invocations = Vec::new();
        let mut helpers: Option<HelperEntitlements> = None;
        for nested in nested_code_paths(&app_path)? {
            let entitlements = match nested_entitlements(&nested) {
                Some(NestedEntitlements::CefJit) => {
                    if helpers.is_none() {
                        helpers = Some(HelperEntitlements::new(&host)?);
                    }
                    helpers.as_ref().map(|h| h.cef_jit.as_path())
                }
                None => None,
            };
            invocations.push(codesign_arguments(
                &nested,
                &identity,
                seal,
                entitlements,
                None,
            ));
        }
        invocations.push(codesign_arguments(
            &app_path,
            &identity,
            seal,
            app_entitlements.as_deref(),
            Some(&bundle_id),
        ));
        Ok(CodesignPlan {
            invocations,
            helpers,
        })
    })
    .await
}

/// One `codesign` invocation.
#[cfg(target_os = "macos")]
fn codesign_arguments(
    path: &Path,
    identity: &str,
    flavor: Seal,
    entitlements: Option<&Path>,
    identifier: Option<&str>,
) -> Vec<OsString> {
    let mut arguments = vec![
        OsString::from("--force"),
        OsString::from("--sign"),
        OsString::from(identity),
    ];
    match flavor {
        Seal::Development => arguments.push(OsString::from("--timestamp=none")),
        Seal::Distribution => {
            arguments.push(OsString::from("--options"));
            arguments.push(OsString::from("runtime"));
            arguments.push(OsString::from("--timestamp"));
        }
    }
    if let Some(entitlements) = entitlements {
        arguments.push(OsString::from("--entitlements"));
        arguments.push(entitlements.as_os_str().to_owned());
    }
    if let Some(identifier) = identifier {
        arguments.push(OsString::from("--identifier"));
        arguments.push(OsString::from(identifier));
    }
    arguments.push(path.as_os_str().to_owned());
    arguments
}

/// The directories inside `Contents/` that can carry nested code.
#[cfg(target_os = "macos")]
const NESTED_CODE_DIRS: [&str; 5] = [
    "Frameworks",
    "PlugIns",
    "XPCServices",
    "Helpers",
    "Library/LoginItems",
];

/// The nested code paths `app_path` carries, in inside-out order: every item
/// in each standard nested-code location, with each bundle listed after the
/// code inside it, plus any Mach-O executables in `Contents/MacOS` beyond
/// the bundle's own `CFBundleExecutable`.
///
/// A location that does not exist is simply absent; any other read failure
/// (a listing or metadata error) propagates — a half-sealed bundle must not
/// leave the build as though it were complete.
#[cfg(target_os = "macos")]
fn nested_code_paths(app_path: &Path) -> eyre::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_bundle_nested_code(app_path, &mut paths)?;
    Ok(paths)
}

/// The nested code `bundle` carries inside its `Contents`, deepest first.
#[cfg(target_os = "macos")]
fn collect_bundle_nested_code(bundle: &Path, paths: &mut Vec<PathBuf>) -> eyre::Result<()> {
    let contents = bundle.join("Contents");
    for location in NESTED_CODE_DIRS {
        collect_dir_code(&contents.join(location), paths, None)?;
    }
    // The bundle's own `CFBundleExecutable` seals with the bundle, so
    // `Contents/MacOS` contributes every Mach-O file but that one — and the
    // non-bundle directories the flat CEF runtime stages next to it
    // (`swiftshader/`, `locales/`) get the same treatment.
    collect_dir_code(
        &contents.join("MacOS"),
        paths,
        bundle_executable_name(&contents)?
            .as_deref()
            .map(OsStr::new),
    )
}

/// `dir`'s children, sorted so the walk is deterministic. A missing
/// directory yields no children; any other listing failure propagates.
#[cfg(target_os = "macos")]
fn sorted_children(dir: &Path) -> eyre::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("failed to list {}", dir.display()));
        }
    };
    let mut children = Vec::new();
    for entry in entries {
        children.push(
            entry
                .wrap_err_with(|| format!("failed to list {}", dir.display()))?
                .path(),
        );
    }
    children.sort();
    Ok(children)
}

/// The signable code `dir` holds, recursively and inside out: a nested
/// `.app`-style bundle follows the code inside it, a `.framework` follows
/// its own internals, any other directory recurses, and a file signs only
/// when it is a real Mach-O binary — plain resources seal with the bundle
/// that carries them and are never signed alone. `excluded` names a file
/// directly inside `dir` that seals with its owning bundle instead (a
/// `CFBundleExecutable`). Symlinks are skipped: the walk signs each real
/// file once, at the location where it actually lives — a helper bundle's
/// linked libraries seal in the outer `Frameworks`, and a versioned
/// framework's `Current` link never shadows `Versions/A`.
#[cfg(target_os = "macos")]
fn collect_dir_code(
    dir: &Path,
    paths: &mut Vec<PathBuf>,
    excluded: Option<&OsStr>,
) -> eyre::Result<()> {
    for path in sorted_children(dir)? {
        let metadata = path
            .symlink_metadata()
            .wrap_err_with(|| format!("failed to inspect {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            match path.extension().and_then(OsStr::to_str) {
                Some("app" | "xpc" | "appex" | "plugin" | "bundle") => {
                    collect_bundle_nested_code(&path, paths)?;
                    paths.push(path);
                }
                Some("framework") => {
                    collect_framework_code(&path, paths)?;
                    paths.push(path);
                }
                _ => collect_dir_code(&path, paths, None)?,
            }
        } else if metadata.is_file() && path.file_name() != excluded && is_mach_o(&path)? {
            paths.push(path);
        }
    }
    Ok(())
}

/// The code a `.framework` carries, signed before the framework itself.
///
/// A versioned layout (`Foo.framework/Versions/{A,…}` with `Current` and the
/// top-level entries as symlinks) is walked inside each real version
/// directory — `Libraries/*.dylib`, helper binaries, nested frameworks — so
/// every Mach-O inner binary signs before the enclosing framework does; the
/// framework's own `CFBundleExecutable` seals with it. A flat layout (a
/// `Foo.framework` with no `Versions/`) is walked the same way from the
/// framework root.
#[cfg(target_os = "macos")]
fn collect_framework_code(framework: &Path, paths: &mut Vec<PathBuf>) -> eyre::Result<()> {
    // The executable a framework seals around is its CFBundleExecutable from
    // `Resources/Info.plist` (flat layouts may carry `Info.plist` at the
    // root); absent either, convention has it named after the framework.
    let excluded = framework_executable_name(framework)?.or_else(|| {
        framework
            .file_stem()
            .and_then(OsStr::to_str)
            .map(str::to_owned)
    });
    let versions = framework.join("Versions");
    if versions.is_dir() {
        for version in sorted_children(&versions)? {
            let metadata = version
                .symlink_metadata()
                .wrap_err_with(|| format!("failed to inspect {}", version.display()))?;
            // `Versions/Current` is a symlink to the real version directory;
            // walking real directories only keeps every file signed once.
            if metadata.is_dir() {
                collect_dir_code(&version, paths, excluded.as_deref().map(OsStr::new))?;
            }
        }
    } else {
        collect_dir_code(framework, paths, excluded.as_deref().map(OsStr::new))?;
    }
    Ok(())
}

/// The `CFBundleExecutable` a framework's `Info.plist` declares, looking in
/// `Resources/` first and the framework root for flat layouts.
#[cfg(target_os = "macos")]
fn framework_executable_name(framework: &Path) -> eyre::Result<Option<String>> {
    for location in ["Resources/Info.plist", "Info.plist"] {
        let info_plist = framework.join(location);
        if info_plist.exists() {
            return plist_executable_name(&info_plist);
        }
    }
    Ok(None)
}

/// The bundle's `CFBundleExecutable` name from its `Info.plist`, when the
/// bundle declares one.
#[cfg(target_os = "macos")]
fn bundle_executable_name(contents: &Path) -> eyre::Result<Option<String>> {
    plist_executable_name(&contents.join("Info.plist"))
}

/// `CFBundleExecutable` in `info_plist`, or `None` when the file is absent.
#[cfg(target_os = "macos")]
fn plist_executable_name(info_plist: &Path) -> eyre::Result<Option<String>> {
    if !info_plist.exists() {
        return Ok(None);
    }
    let plist::Value::Dictionary(root) = plist::Value::from_file(info_plist)
        .wrap_err_with(|| format!("failed to read {}", info_plist.display()))?
    else {
        bail!("{} is not a plist dictionary", info_plist.display());
    };
    Ok(root
        .get("CFBundleExecutable")
        .and_then(plist::Value::as_string)
        .map(str::to_owned))
}

/// Whether `path` opens with a Mach-O magic number — the signature of a
/// Mach-O binary, in each bit width and byte order plus the fat (universal)
/// wrappers.
#[cfg(target_os = "macos")]
fn is_mach_o(path: &Path) -> eyre::Result<bool> {
    use std::io::Read as _;

    let mut file =
        std::fs::File::open(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
    let mut magic = [0u8; 4];
    match file.read_exact(&mut magic) {
        Ok(()) => {}
        // Shorter than a magic number: not a Mach-O executable.
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("failed to read {}", path.display()));
        }
    }
    Ok(matches!(
        magic,
        // MH_MAGIC / MH_MAGIC_64 and their byte-swapped CIGAM forms.
        [0xfe, 0xed, 0xfa, 0xce | 0xcf]
            | [0xce | 0xcf, 0xfa, 0xed, 0xfe]
            // FAT_MAGIC / FAT_MAGIC_64 and their byte-swapped CIGAM forms.
            | [0xca, 0xfe, 0xba, 0xbe | 0xbf]
            | [0xbe | 0xbf, 0xba, 0xfe, 0xca]
    ))
}

#[cfg(target_os = "macos")]
async fn run_sign_plan(host: &Host, plan: CodesignPlan) -> eyre::Result<()> {
    for arguments in &plan.invocations {
        host.run("codesign", arguments).await?;
    }
    // The plan owns the generated helper entitlements; dropping it here keeps
    // the scratch directory alive for the whole signing run.
    drop(plan.helpers);
    Ok(())
}

/// Why no Developer ID identity can sign for a team.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeveloperIdIdentityMissing {
    /// No `Developer ID Application` certificate for the team is installed
    /// at all.
    NoCertificate,
    /// A matching certificate is installed but `security find-identity -v -p
    /// codesigning` does not offer it — no usable private key pairs with it
    /// (a `.cer` import drops the key; an expired certificate loses it too).
    NoUsableIdentity,
}

/// The SHA-1 fingerprint `codesign` knows the Developer ID Application
/// identity issued to `team_id` by.
///
/// The identity is chosen on the certificate itself — `security`'s identity
/// display orders certificates arbitrarily and the name alone says nothing
/// about which team issued them — so each keychain certificate's subject is
/// parsed and matched on the `Developer ID Application` common name plus the
/// team ID as organizational unit. The match is then intersected with the
/// identities `security find-identity -v -p codesigning` reports as valid:
/// a certificate without its private key (or past its validity) must not
/// shadow a usable identity for the same team.
///
/// # Errors
/// Returns an error when `security` cannot list identities or certificates,
/// when no matching certificate is installed, or when the installed one
/// carries no usable private key — each message names the setup step it
/// needs.
#[cfg(target_os = "macos")]
async fn developer_id_identity(host: &Host, team_id: &str) -> eyre::Result<String> {
    let identities_output = host
        .run("security", ["find-identity", "-v", "-p", "codesigning"])
        .await?;
    let usable: BTreeSet<String> = valid_codesigning_identities(&identities_output)
        .map(str::to_owned)
        .collect();
    let certificates = crate::apple::toolchain::keychain_certificates(host).await?;
    developer_id_identity_in(&certificates, &usable, team_id).map_err(|missing| match missing {
        DeveloperIdIdentityMissing::NoCertificate => eyre::eyre!(
            "macOS distribution packaging needs a \"Developer ID Application\" \
                 certificate for team {team_id}, and none is installed in the \
                 keychain. Create one in Xcode → Settings → Accounts → Manage \
                 Certificates → \"+\" → \"Developer ID Application\", then re-run \
                 `water package`."
        ),
        DeveloperIdIdentityMissing::NoUsableIdentity => eyre::eyre!(
            "macOS distribution packaging needs a \"Developer ID Application\" \
                 certificate for team {team_id}; one is installed in the keychain, \
                 but `security find-identity -v -p codesigning` does not offer it — \
                 no usable private key pairs with it. Import the certificate's \
                 `.p12` (the private key travels with it) or create a new identity \
                 in Xcode → Settings → Accounts → Manage Certificates, then re-run \
                 `water package`."
        ),
    })
}

/// The SHA-1 of the certificate in `certificates` whose subject is a
/// `Developer ID Application:` common name for `team_id`, when the
/// certificate has one — regardless of whether the keychain can sign with
/// it.
#[cfg(target_os = "macos")]
fn developer_id_certificate_sha1(der: &[u8], team_id: &str) -> Option<String> {
    use x509_parser::prelude::FromDer as _;

    let (_, certificate) = x509_parser::certificate::X509Certificate::from_der(der).ok()?;
    let subject = certificate.subject();
    let developer_id_application = subject.iter_common_name().any(|name| {
        name.as_str()
            .is_ok_and(|common| common.starts_with("Developer ID Application:"))
    });
    let team = subject
        .iter_organizational_unit()
        .any(|unit| unit.as_str().is_ok_and(|unit| unit == team_id));
    (developer_id_application && team).then(|| crate::apple::toolchain::certificate_sha1_hex(der))
}

/// The Developer ID identity to sign with: the certificate matching
/// `team_id` that `find-identity` reports as usable. A matching certificate
/// that cannot sign is remembered so the caller can tell "not installed"
/// from "installed but keyless".
#[cfg(target_os = "macos")]
fn developer_id_identity_in(
    certificates: &[Vec<u8>],
    usable_identities: &BTreeSet<String>,
    team_id: &str,
) -> Result<String, DeveloperIdIdentityMissing> {
    let mut keyless = false;
    for der in certificates {
        let Some(sha1) = developer_id_certificate_sha1(der, team_id) else {
            continue;
        };
        if usable_identities.contains(&sha1) {
            return Ok(sha1);
        }
        keyless = true;
    }
    Err(if keyless {
        DeveloperIdIdentityMissing::NoUsableIdentity
    } else {
        DeveloperIdIdentityMissing::NoCertificate
    })
}

/// The JSON result `notarytool submit --output-format json` reports.
#[cfg(target_os = "macos")]
#[derive(Debug, serde::Deserialize)]
struct NotarySubmission {
    status: String,
    id: Option<String>,
    message: Option<String>,
}

/// The disposition `notarytool submit --wait` reports for a submission.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotaryDisposition {
    /// The submission passed; the ticket can be stapled onto the app.
    Accepted,
    /// The service rejected the submission; its log says why.
    Invalid,
    /// `--wait` returned before the service finished assessing the app.
    InProgress,
    /// A disposition this CLI does not know; the raw output says more.
    Unknown,
}

#[cfg(target_os = "macos")]
impl NotarySubmission {
    fn disposition(&self) -> NotaryDisposition {
        match self.status.as_str() {
            "Accepted" => NotaryDisposition::Accepted,
            "Invalid" => NotaryDisposition::Invalid,
            "In Progress" => NotaryDisposition::InProgress,
            _ => NotaryDisposition::Unknown,
        }
    }
}

#[cfg(target_os = "macos")]
fn parse_notary_submission(output: &str) -> eyre::Result<NotarySubmission> {
    serde_json::from_str(output)
        .map_err(|error| eyre::eyre!("notarytool did not report JSON: {error}\n{output}"))
}

/// Submit `app_path` for notarization under `keychain_profile` and wait for
/// the verdict.
///
/// `notarytool` only accepts an upload format it can inspect — a zip, pkg or
/// dmg — so the signed bundle travels as `<app>.zip`, built with `ditto` so
/// the signatures on the sealed items survive the round trip. The ticket the
/// service returns is stapled to the bundle itself, so the archive stays
/// behind only as the artifact to ship or discard.
///
/// # Errors
/// Returns an error when the submission itself fails — with the
/// `store-credentials` setup command — or when the service does not accept
/// the app; a rejection includes the notary log that explains it.
#[cfg(target_os = "macos")]
async fn notarize_app(host: &Host, app_path: &Path, keychain_profile: &str) -> eyre::Result<()> {
    let archive = app_path.with_extension("zip");
    host.run(
        "ditto",
        [
            OsStr::new("-c"),
            OsStr::new("-k"),
            OsStr::new("--sequesterRsrc"),
            OsStr::new("--keepParent"),
            app_path.as_os_str(),
            archive.as_os_str(),
        ],
    )
    .await?;
    let output = host
        .output(
            "xcrun",
            [
                OsStr::new("notarytool"),
                OsStr::new("submit"),
                archive.as_os_str(),
                OsStr::new("--keychain-profile"),
                OsStr::new(keychain_profile),
                OsStr::new("--wait"),
                OsStr::new("--output-format"),
                OsStr::new("json"),
            ],
        )
        .await?;
    if !output.status.success() {
        bail!(
            "notarytool could not submit {} for notarization:\n{}\n\
             Store the App Store Connect credentials it authenticates with:\n  \
             xcrun notarytool store-credentials \"{keychain_profile}\" \\\n                 --apple-id <apple-id> --team-id <team-id> --password <app-specific-password>",
            app_path.display(),
            String::from_utf8_lossy(&output.stderr).trim_end(),
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let submission = parse_notary_submission(&stdout)?;
    match submission.disposition() {
        NotaryDisposition::Accepted => Ok(()),
        NotaryDisposition::Invalid => {
            let reason = submission
                .message
                .as_deref()
                .unwrap_or("no reason reported");
            let log = notary_log(host, submission.id.as_deref(), keychain_profile)
                .await
                .wrap_err_with(|| {
                    format!("Notarization rejected {} ({reason})", app_path.display())
                })?;
            bail!(
                "Notarization rejected {} ({reason}):\n{}",
                app_path.display(),
                log.trim_end()
            );
        }
        _ => bail!(
            "Notarization of {} did not complete (status {:?}):\n{}",
            app_path.display(),
            submission.status,
            stdout.trim_end()
        ),
    }
}

/// The notarization log for a rejected submission.
///
/// # Errors
/// Returns an error when the submission reported no id to fetch a log for or
/// `notarytool log` itself fails — the rejection then propagates with this
/// failure in its chain.
#[cfg(target_os = "macos")]
async fn notary_log(
    host: &Host,
    submission_id: Option<&str>,
    keychain_profile: &str,
) -> eyre::Result<String> {
    let submission_id = submission_id
        .ok_or_else(|| eyre::eyre!("the submission reported no id to fetch a log for"))?;
    let output = host
        .output(
            "xcrun",
            [
                OsStr::new("notarytool"),
                OsStr::new("log"),
                OsStr::new(submission_id),
                OsStr::new("--keychain-profile"),
                OsStr::new(keychain_profile),
            ],
        )
        .await
        .wrap_err("failed to run `xcrun notarytool log`")?;
    if !output.status.success() {
        bail!(
            "`notarytool log` for submission {submission_id} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim_end(),
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Staple the notarization ticket onto `app_path`.
///
/// # Errors
/// Returns an error when `stapler` cannot attach the ticket.
#[cfg(target_os = "macos")]
async fn staple_app(host: &Host, app_path: &Path) -> eyre::Result<()> {
    host.run(
        "xcrun",
        [
            OsStr::new("stapler"),
            OsStr::new("staple"),
            app_path.as_os_str(),
        ],
    )
    .await?;
    Ok(())
}

/// The identity SHA-1s `security find-identity -v -p codesigning` reports —
/// the certificates that pair with a usable private key, in listing order.
/// Lines read `  N) <sha1> "<name>"`; the trailing `N valid identities
/// found` summary line does not match the shape.
#[cfg(target_os = "macos")]
fn valid_codesigning_identities(output: &str) -> impl Iterator<Item = &str> {
    output.lines().filter_map(|line| {
        let (_, identity_and_name) = line.split_once(')')?;
        let identity = identity_and_name.split_whitespace().next()?;
        (identity.len() == 40 && identity.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then_some(identity)
    })
}

#[cfg(target_os = "macos")]
fn first_codesigning_identity(output: &str) -> Option<&str> {
    valid_codesigning_identities(output).next()
}

/// Packages the current application executable as the invisible helper variants
/// required by CEF on macOS.
///
/// # Errors
/// Returns an error if the main bundle layout is malformed or the helper cannot
/// be copied and described.
#[cfg(target_os = "macos")]
pub async fn package_cef_helper_app(
    app_dir: &Path,
    main_binary_path: &Path,
    helper_binary_path: &Path,
    bundle_identifier: &AppleBundleIdentifier,
) -> eyre::Result<Vec<PathBuf>> {
    let executable_name = main_binary_path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| eyre::eyre!("main application has no valid executable name"))?;
    if !helper_binary_path.is_file() {
        bail!(
            "CEF helper executable is missing at {}",
            helper_binary_path.display()
        );
    }
    let main_frameworks_dir = app_dir.join("Contents/Frameworks");
    let mut dynamic_libraries = Vec::new();
    let mut frameworks = fs::read_dir(&main_frameworks_dir).await?;
    while let Some(entry) = frameworks.next().await {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("dylib") {
            dynamic_libraries.push(entry.file_name());
        }
    }

    let mut helper_dirs = Vec::with_capacity(CEF_HELPER_VARIANTS.len());
    for (name_suffix, identifier_suffix) in CEF_HELPER_VARIANTS {
        let helper_name = format!("{executable_name} Helper{name_suffix}");
        let helper_dir = main_frameworks_dir.join(format!("{helper_name}.app"));
        if helper_dir.exists() {
            fs::remove_dir_all(&helper_dir).await?;
        }

        let helper_contents_dir = helper_dir.join("Contents");
        let helper_macos_dir = helper_contents_dir.join("MacOS");
        let helper_frameworks_dir = helper_contents_dir.join("Frameworks");
        fs::create_dir_all(&helper_macos_dir).await?;
        fs::create_dir_all(&helper_frameworks_dir).await?;

        let helper_executable = helper_macos_dir.join(&helper_name);
        copy_file(helper_binary_path, &helper_executable).await?;
        {
            use std::os::unix::fs::PermissionsExt as _;

            let mut permissions = fs::metadata(&helper_executable).await?.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&helper_executable, permissions).await?;
        }
        for name in &dynamic_libraries {
            std::os::unix::fs::symlink(
                Path::new("../../..").join(name),
                helper_frameworks_dir.join(name),
            )?;
        }

        let helper_bundle_identifier = format!("{bundle_identifier}.helper{identifier_suffix}");
        let plist = CefHelperInfoPlistTemplate {
            bundle_identifier: &helper_bundle_identifier,
            helper_name: &helper_name,
            product_name: executable_name,
        }
        .render()
        .map_err(|error| eyre::eyre!("Failed to render CEF helper Info.plist: {error}"))?;
        fs::write(helper_contents_dir.join("Info.plist"), plist).await?;
        fs::write(helper_contents_dir.join("PkgInfo"), b"APPL????").await?;
        helper_dirs.push(helper_dir);
    }

    Ok(helper_dirs)
}

/// Removes CEF helper applications added after a previous macOS build.
///
/// # Errors
///
/// Returns an error when an existing helper application cannot be removed.
#[cfg(target_os = "macos")]
pub async fn remove_cef_helper_apps(app_dir: &Path, executable_name: &str) -> eyre::Result<()> {
    let frameworks_dir = app_dir.join("Contents/Frameworks");
    for (name_suffix, _) in CEF_HELPER_VARIANTS {
        let helper_name = format!("{executable_name} Helper{name_suffix}.app");
        let helper_dir = frameworks_dir.join(helper_name);
        if helper_dir.exists() {
            fs::remove_dir_all(helper_dir).await?;
        }
    }
    Ok(())
}

async fn copy_dir(from: &Path, to: &Path) -> eyre::Result<()> {
    let source = from.to_path_buf();
    let destination = to.to_path_buf();
    smol::unblock(move || {
        let mut options = CopyOptions::new();
        options.copy_inside = true;
        options.overwrite = true;
        fs_extra::dir::copy(&source, &destination, &options)
            .map(|_| ())
            .map_err(|error| {
                eyre::eyre!(
                    "Failed to copy resources from {} to {}: {error}",
                    source.display(),
                    destination.display()
                )
            })
    })
    .await
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::collections::BTreeSet;
    use std::os::unix::fs::PermissionsExt as _;

    use super::{first_codesigning_identity, package_cef_helper_app, remove_cef_helper_apps};

    /// A Mach-O 64-bit magic number, enough of a header for `is_mach_o`.
    const MACH_O_64: [u8; 4] = [0xcf, 0xfa, 0xed, 0xfe];

    /// A self-signed certificate whose subject carries the common name and
    /// organizational unit identity selection reads. Generated at test time
    /// so no key material is committed.
    fn certificate_der(common_name: &str, organizational_unit: &str) -> Vec<u8> {
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, common_name);
        params
            .distinguished_name
            .push(rcgen::DnType::OrganizationalUnitName, organizational_unit);
        let key = rcgen::KeyPair::generate().expect("a key pair must generate");
        params
            .self_signed(&key)
            .expect("a self-signed certificate must generate")
            .der()
            .to_vec()
    }

    /// The SHA-1 `codesign` names a certificate by.
    fn sha1(der: &[u8]) -> String {
        crate::apple::toolchain::certificate_sha1_hex(der)
    }

    #[test]
    fn parses_first_valid_codesigning_identity() {
        let output = "  1) 645DCB18E20044A687FFE48B0E62D31BF9F6A443 \"Apple Development\"\n     1 valid identities found\n";
        assert_eq!(
            first_codesigning_identity(output),
            Some("645DCB18E20044A687FFE48B0E62D31BF9F6A443")
        );
    }

    #[test]
    fn lists_every_valid_codesigning_identity() {
        let output = "  1) 645DCB18E20044A687FFE48B0E62D31BF9F6A443 \"Apple Development\"\n  2) 251D99B01777B98B2CAE0F4EACF99BCB65CDD14E \"Developer ID Application: Devin Test\"\n     2 valid identities found\n";
        assert_eq!(
            super::valid_codesigning_identities(output).collect::<Vec<_>>(),
            [
                "645DCB18E20044A687FFE48B0E62D31BF9F6A443",
                "251D99B01777B98B2CAE0F4EACF99BCB65CDD14E"
            ]
        );
    }

    #[test]
    fn reports_no_codesigning_identity() {
        assert_eq!(
            first_codesigning_identity("     0 valid identities found\n"),
            None
        );
    }

    #[test]
    fn packaged_app_carries_icon_and_plist_references() {
        smol::block_on(async {
            let temporary = tempfile::tempdir().expect("temporary directory must be available");
            // The built binary carries the generated crate's project-root
            // tag; the bundle ships it under the product name.
            let binary = temporary.path().join("demo-hydrolysis-deadbeef");
            std::fs::write(&binary, b"demo").expect("fake executable must be written");
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
                .expect("fake executable must be executable");

            let app = super::package_binary_as_app(
                &binary,
                &crate::project_types::AppleBundleIdentifier::try_from("dev.waterui.demo")
                    .expect("bundle identifier"),
                super::MacOsAppNames {
                    app_name: "Demo",
                    executable_name: "demo-hydrolysis",
                },
                &[],
                None,
                b"fake-icns-bytes",
                temporary.path(),
            )
            .await
            .expect("bundle must package");

            assert_eq!(
                std::fs::read(app.join("Contents/Resources/AppIcon.icns"))
                    .expect("bundle must contain the icon family"),
                b"fake-icns-bytes"
            );
            assert!(app.join("Contents/MacOS/demo-hydrolysis").is_file());
            assert!(!app.join("Contents/MacOS/demo-hydrolysis-deadbeef").exists());
            let plist = std::fs::read_to_string(app.join("Contents/Info.plist"))
                .expect("bundle plist must be readable");
            assert!(plist.contains("<key>CFBundleIconFile</key>"));
            assert!(plist.contains("<key>CFBundleIconName</key>"));
            assert!(plist.contains("<string>demo-hydrolysis</string>"));
            assert!(!plist.contains("deadbeef"));
        });
    }

    #[test]
    fn cef_helpers_are_invisible_variant_bundles_with_shared_runtime_links() {
        smol::block_on(async {
            let temporary = tempfile::tempdir().expect("temporary directory must be available");
            let app = temporary.path().join("Browser.app");
            let frameworks = app.join("Contents/Frameworks");
            let binary = temporary.path().join("browser");
            let helper_binary = temporary.path().join("waterui-cef-helper");
            std::fs::create_dir_all(&frameworks).expect("frameworks directory must be created");
            std::fs::write(&binary, b"browser").expect("fake executable must be written");
            std::fs::write(&helper_binary, b"helper")
                .expect("fake helper executable must be written");
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
                .expect("fake executable must be executable");
            std::fs::set_permissions(&helper_binary, std::fs::Permissions::from_mode(0o755))
                .expect("fake helper executable must be executable");
            std::fs::write(frameworks.join("libwaterui.dylib"), b"runtime")
                .expect("fake runtime must be written");

            let helpers = package_cef_helper_app(
                &app,
                &binary,
                &helper_binary,
                &crate::project_types::AppleBundleIdentifier::try_from("dev.waterui.browser")
                    .expect("bundle identifier"),
            )
            .await
            .expect("CEF helper must package");
            assert_eq!(helpers.len(), 5);
            let helper = &helpers[0];
            let helper_name = "browser Helper";
            let plist = std::fs::read_to_string(helper.join("Contents/Info.plist"))
                .expect("helper plist must be readable");
            assert!(plist.contains("<key>LSUIElement</key>"));
            assert!(plist.contains("dev.waterui.browser.helper"));
            assert!(helper.join("Contents/MacOS").join(helper_name).is_file());
            assert_eq!(
                std::fs::read_link(helper.join("Contents/Frameworks/libwaterui.dylib"))
                    .expect("helper runtime must be linked"),
                std::path::PathBuf::from("../../../libwaterui.dylib")
            );
            let renderer = &helpers[4];
            let renderer_plist = std::fs::read_to_string(renderer.join("Contents/Info.plist"))
                .expect("renderer helper plist must be readable");
            assert!(renderer_plist.contains("browser Helper (Renderer)"));
            assert!(renderer_plist.contains("dev.waterui.browser.helper.renderer"));

            remove_cef_helper_apps(&app, "browser")
                .await
                .expect("CEF helpers must be removable before an incremental build");
            for helper in helpers {
                assert!(!helper.exists());
            }
        });
    }

    #[test]
    fn selects_the_usable_developer_id_identity_by_subject_ou() {
        let development = certificate_der("Apple Development: Devin Test", "TEAMID1234");
        // Listed before the usable identity — a certificate whose private key
        // never made it into the keychain must not shadow it.
        let keyless = certificate_der("Developer ID Application: Devin Test", "TEAMID1234");
        let developer_id = certificate_der("Developer ID Application: Devin Test", "TEAMID1234");
        let other_team = certificate_der("Developer ID Application: Devin Test", "OTHERTEAM9");
        let developer_id_sha1 = sha1(&developer_id);
        let usable = BTreeSet::from([developer_id_sha1.clone(), sha1(&other_team)]);
        let certificates = vec![development, keyless, developer_id, other_team];

        assert_eq!(
            super::developer_id_identity_in(&certificates, &usable, "TEAMID1234"),
            Ok(developer_id_sha1)
        );
    }

    #[test]
    fn a_keyless_developer_id_certificate_is_not_an_identity() {
        let keyless = certificate_der("Developer ID Application: Devin Test", "TEAMID1234");

        assert_eq!(
            super::developer_id_identity_in(&[keyless], &BTreeSet::new(), "TEAMID1234"),
            Err(super::DeveloperIdIdentityMissing::NoUsableIdentity)
        );
    }

    #[test]
    fn finds_no_identity_for_a_team_without_a_developer_id_certificate() {
        let certificates = vec![
            certificate_der("Apple Development: Devin Test", "TEAMID1234"),
            certificate_der("Developer ID Application: Devin Test", "OTHERTEAM9"),
        ];

        assert_eq!(
            super::developer_id_identity_in(&certificates, &BTreeSet::new(), "TEAMID1234"),
            Err(super::DeveloperIdIdentityMissing::NoCertificate)
        );
    }

    fn write_info_plist(contents: &std::path::Path, executable: &str) {
        plist::Value::Dictionary(plist::Dictionary::from_iter([(
            String::from("CFBundleExecutable"),
            plist::Value::String(String::from(executable)),
        )]))
        .to_file_xml(contents.join("Info.plist"))
        .expect("the fixture Info.plist must be written");
    }

    /// The versioned layout `browser_runtime.rs` stages verbatim from the
    /// CEF distribution: `Versions/A` holds the real files while `Current`
    /// and the top-level entries are symlinks.
    fn write_cef_framework(frameworks: &std::path::Path) -> std::path::PathBuf {
        let cef = frameworks.join("Chromium Embedded Framework.framework");
        let version = cef.join("Versions/A");
        for dir in [&version.join("Libraries"), &version.join("Resources")] {
            std::fs::create_dir_all(dir).expect("CEF layout must be created");
        }
        std::fs::write(version.join("Chromium Embedded Framework"), MACH_O_64)
            .expect("framework executable must be written");
        write_info_plist(&version.join("Resources"), "Chromium Embedded Framework");
        std::fs::write(version.join("Resources/cef.pak"), b"pak")
            .expect("resource must be written");
        for library in ["libEGL.dylib", "libGLESv2.dylib", "libvk_swiftshader.dylib"] {
            std::fs::write(version.join("Libraries").join(library), MACH_O_64)
                .expect("CEF library must be written");
        }
        std::os::unix::fs::symlink("A", cef.join("Versions/Current"))
            .expect("Current symlink must be created");
        for (link, target) in [
            (
                "Chromium Embedded Framework",
                "Versions/Current/Chromium Embedded Framework",
            ),
            ("Libraries", "Versions/Current/Libraries"),
            ("Resources", "Versions/Current/Resources"),
        ] {
            std::os::unix::fs::symlink(target, cef.join(link))
                .expect("top-level symlink must be created");
        }
        cef
    }

    /// A CEF helper bundle as `package_cef_helper_app` creates it.
    fn write_cef_helper(frameworks: &std::path::Path, name: &str) -> std::path::PathBuf {
        let helper = frameworks.join(format!("{name}.app"));
        let macos = helper.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).expect("helper bundle must be created");
        write_info_plist(&helper.join("Contents"), name);
        std::fs::write(macos.join(name), MACH_O_64).expect("helper executable must be written");
        helper
    }

    fn sign_args(
        path: &std::path::Path,
        entitlements: Option<&std::path::Path>,
    ) -> Vec<std::ffi::OsString> {
        let mut args: Vec<std::ffi::OsString> = vec![
            "--force".into(),
            "--sign".into(),
            "HASH".into(),
            "--options".into(),
            "runtime".into(),
            "--timestamp".into(),
        ];
        if let Some(entitlements) = entitlements {
            args.push("--entitlements".into());
            args.push(entitlements.as_os_str().to_owned());
        }
        args.push(path.as_os_str().to_owned());
        args
    }

    /// A declared host whose temporary directory is `temp`.
    fn host_with_temp_dir(temp: &std::path::Path) -> crate::toolchain::Host {
        crate::toolchain::Host::new(std::iter::empty::<&std::path::Path>(), [("TMPDIR", temp)])
    }

    #[test]
    fn codesign_plan_signs_nested_code_inside_out() {
        smol::block_on(async {
            let temporary = tempfile::tempdir().expect("temporary directory must be available");
            let app = temporary.path().join("Demo.app");
            let contents = app.join("Contents");
            let macos_dir = contents.join("MacOS");
            let frameworks = contents.join("Frameworks");
            let login_item = contents.join("Library/LoginItems/LoginItem.app");
            for dir in [
                &macos_dir,
                &macos_dir.join("swiftshader"),
                &contents.join("PlugIns"),
                &contents.join("XPCServices"),
                &contents.join("Helpers"),
                &login_item.join("Contents/MacOS"),
            ] {
                std::fs::create_dir_all(dir).expect("nested bundle directories must be created");
            }
            write_info_plist(&contents, "Demo");
            write_info_plist(&login_item.join("Contents"), "LoginItem");
            std::fs::write(macos_dir.join("notes.txt"), b"not code")
                .expect("data file must be written");
            let cef = write_cef_framework(&frameworks);
            let helper = write_cef_helper(&frameworks, "Demo Helper");
            let gpu_helper = write_cef_helper(&frameworks, "Demo Helper (GPU)");
            let renderer_helper = write_cef_helper(&frameworks, "Demo Helper (Renderer)");
            // The main and extra executables, a flat-runtime library, a
            // framework library, the login item's executable and a helper tool.
            for code in [
                macos_dir.join("Demo"),
                macos_dir.join("agent"),
                macos_dir.join("swiftshader/libvk_swiftshader.dylib"),
                frameworks.join("libfoo.dylib"),
                login_item.join("Contents/MacOS/LoginItem"),
                contents.join("Helpers/helper-tool"),
            ] {
                std::fs::write(code, MACH_O_64).expect("Mach-O fixture must be written");
            }
            std::os::unix::fs::symlink("libfoo.dylib", frameworks.join("liblink.dylib"))
                .expect("symlink must be created");
            std::fs::create_dir_all(contents.join("PlugIns/Share.appex"))
                .expect("plug-in bundle must be created");
            std::fs::create_dir_all(contents.join("XPCServices/Agent.xpc"))
                .expect("xpc bundle must be created");
            let entitlements = temporary.path().join("Demo.entitlements");
            let distribution = super::DistributionSigning {
                team_id: String::from("TEAMID1234"),
                notary_profile: String::from("fixture-profile"),
                entitlements: Some(entitlements.clone()),
            };

            let host = host_with_temp_dir(temporary.path());
            let plan =
                super::codesign_plan(&host, &app, "HASH", "dev.waterui.demo", Some(&distribution))
                    .await
                    .expect("the sign plan must build");

            let jit = plan
                .helpers
                .as_ref()
                .map(|helpers| helpers.cef_jit.as_path())
                .filter(|jit| jit.starts_with(temporary.path()))
                .expect(
                    "the CEF JIT helpers must carry entitlements generated in the host temp dir",
                );
            assert!(
                std::fs::read_to_string(jit)
                    .expect("the generated entitlements must be readable")
                    .contains("com.apple.security.cs.allow-jit"),
                "the CEF helper entitlement must grant V8 JIT"
            );
            let cef_libraries = cef.join("Versions/A/Libraries");
            let nested: Vec<(std::path::PathBuf, Option<std::path::PathBuf>)> = [
                (cef_libraries.join("libEGL.dylib"), None),
                (cef_libraries.join("libGLESv2.dylib"), None),
                (cef_libraries.join("libvk_swiftshader.dylib"), None),
                (cef.clone(), None),
                (gpu_helper, Some(jit.to_path_buf())),
                (renderer_helper, Some(jit.to_path_buf())),
                (helper, None),
                (frameworks.join("libfoo.dylib"), None),
                (contents.join("PlugIns/Share.appex"), None),
                (contents.join("XPCServices/Agent.xpc"), None),
                (contents.join("Helpers/helper-tool"), None),
                (login_item, None),
                (macos_dir.join("agent"), None),
                (macos_dir.join("swiftshader/libvk_swiftshader.dylib"), None),
            ]
            .into_iter()
            .collect();
            let expected: Vec<Vec<std::ffi::OsString>> = nested
                .iter()
                .map(|(path, helper_entitlements)| sign_args(path, helper_entitlements.as_deref()))
                .chain([{
                    let mut args = sign_args(&app, Some(&entitlements));
                    args.insert(args.len() - 1, "--identifier".into());
                    args.insert(args.len() - 1, "dev.waterui.demo".into());
                    args
                }])
                .collect();
            assert_eq!(plan.invocations, expected);
        });
    }

    #[test]
    fn nested_code_paths_reports_a_broken_location() {
        let temporary = tempfile::tempdir().expect("temporary directory must be available");
        let app = temporary.path().join("Demo.app");
        let frameworks = app.join("Contents/Frameworks");
        std::fs::create_dir_all(&frameworks).expect("frameworks directory must be created");
        std::fs::set_permissions(&frameworks, std::fs::Permissions::from_mode(0o000))
            .expect("permissions must be set");

        let result = super::nested_code_paths(&app);
        std::fs::set_permissions(&frameworks, std::fs::Permissions::from_mode(0o700))
            .expect("permissions must be restored for cleanup");

        assert!(
            result.is_err(),
            "an unreadable nested-code location must fail, not be skipped"
        );
    }

    #[test]
    fn codesign_plan_signs_development_bundles_without_a_timestamp() {
        smol::block_on(async {
            let temporary = tempfile::tempdir().expect("temporary directory must be available");
            let app = temporary.path().join("Demo.app");
            let frameworks = app.join("Contents/Frameworks");
            std::fs::create_dir_all(&frameworks).expect("frameworks directory must be created");
            std::fs::write(frameworks.join("libfoo.dylib"), MACH_O_64)
                .expect("library must be written");

            let host = host_with_temp_dir(temporary.path());
            let plan = super::codesign_plan(&host, &app, "-", "dev.waterui.demo", None)
                .await
                .expect("the sign plan must build");

            assert_eq!(
                plan.invocations,
                vec![
                    vec![
                        "--force".into(),
                        "--sign".into(),
                        "-".into(),
                        "--timestamp=none".into(),
                        frameworks.join("libfoo.dylib").as_os_str().to_owned(),
                    ],
                    vec![
                        "--force".into(),
                        "--sign".into(),
                        "-".into(),
                        "--timestamp=none".into(),
                        "--identifier".into(),
                        "dev.waterui.demo".into(),
                        app.as_os_str().to_owned(),
                    ],
                ]
            );
        });
    }

    #[test]
    fn distribution_signing_needs_the_manifest_section() {
        let error = super::DistributionSigning::from_manifest(None, None)
            .expect_err("a missing [signing.macos] section must fail")
            .to_string();

        assert!(error.contains("[signing.macos]"));
        assert!(error.contains("xcrun notarytool store-credentials"));
    }

    #[test]
    fn parses_an_accepted_notary_submission() {
        let submission = super::parse_notary_submission(
            r#"{"message":"Successfully received submission info.","id":"2efe2717-52ef-43a5-96dc-0797e4ca1041","status":"Accepted"}"#,
        )
        .expect("accepted submission must parse");

        assert_eq!(submission.disposition(), super::NotaryDisposition::Accepted);
    }

    #[test]
    fn parses_an_invalid_notary_submission() {
        let submission = super::parse_notary_submission(
            r#"{"id":"2efe2717-52ef-43a5-96dc-0797e4ca1041","message":"The signature of the binary is invalid.","status":"Invalid"}"#,
        )
        .expect("invalid submission must parse");

        assert_eq!(submission.disposition(), super::NotaryDisposition::Invalid);
    }

    #[test]
    fn parses_an_in_progress_notary_submission() {
        let submission = super::parse_notary_submission(
            r#"{"id":"2efe2717-52ef-43a5-96dc-0797e4ca1041","status":"In Progress"}"#,
        )
        .expect("in-progress submission must parse");

        assert_eq!(
            submission.disposition(),
            super::NotaryDisposition::InProgress
        );
    }

    #[test]
    fn a_non_json_notary_result_is_an_error() {
        let result = super::parse_notary_submission("Error: could not connect to the App Store");
        assert!(result.is_err());
    }
}
