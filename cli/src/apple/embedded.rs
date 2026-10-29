//! Embedded Apple artifacts assembled without a native host project.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use askama::Template;
use eyre::{Result, bail};
use smol::fs;
use target_lexicon::Architecture;

mod runtime;

use runtime::RuntimeClosure;

use crate::{
    apple::platform,
    artifact_symbols::ArtifactSymbols,
    assets,
    build::{BuildOptions, NativeLink},
    platform::TargetPlatform,
    project::Project,
    toolchain::Host,
    workflows::build::BuiltTarget,
};

/// Stable local Swift package location consumed by a native host.
#[derive(Debug)]
pub struct EmbeddedArtifact {
    /// Add this directory as a local package dependency in the native host.
    package_path: PathBuf,
    /// Header-free binary containing the application's native Rust archive.
    xcframework_path: PathBuf,
}

impl EmbeddedArtifact {
    /// An artifact whose Swift package lives at `package_path` and whose
    /// `XCFramework` lives at `xcframework_path`.
    #[must_use]
    pub const fn new(package_path: PathBuf, xcframework_path: PathBuf) -> Self {
        Self {
            package_path,
            xcframework_path,
        }
    }

    /// The directory a native host adds as a local package dependency.
    #[must_use]
    pub fn package_path(&self) -> &Path {
        &self.package_path
    }

    /// Header-free binary containing the application's native Rust archive.
    #[must_use]
    pub fn xcframework_path(&self) -> &Path {
        &self.xcframework_path
    }
}

#[derive(Template)]
#[template(path = "apple_embedded/Package.swift.tpl", escape = "none")]
struct PackageTemplate<'a> {
    name: &'a str,
    macos: &'a str,
    ios: &'a str,
    links: &'a [PlatformLinks],
}

struct PlatformLinks {
    platform: &'static str,
    links: Vec<NativeLink>,
}

/// `SwiftPM` platform conditions cannot distinguish device, simulator or architecture.
/// Share a list only when its complete ordered link contract is identical.
fn collect_platform_links(
    links: &mut Vec<PlatformLinks>,
    platform: &'static str,
    triple: &str,
    native_links: Vec<NativeLink>,
) -> Result<()> {
    if let Some(existing) = links.iter().find(|entry| entry.platform == platform) {
        if existing.links != native_links {
            bail!(
                "Cannot package static Apple libraries: {triple} requires {native_links:?}, \
                 but another {platform} slice requires {:?}. SwiftPM linker settings cannot \
                 select device/simulator or architecture-specific libraries. A single static \
                 package requires identical ordered native link requirements across those slices.",
                existing.links
            );
        }
    } else {
        links.push(PlatformLinks {
            platform,
            links: native_links,
        });
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct Slice {
    platform: TargetPlatform,
    name: &'static str,
    triple: &'static str,
    module_triple: &'static str,
}

fn slices(architecture: Option<Architecture>) -> Result<Vec<Slice>> {
    if let Some(architecture) = architecture {
        super::platform::validate_architecture(architecture)?;
    }
    let mut result = Vec::new();
    for (platform, name, triple, module_triple) in [
        (
            TargetPlatform::MacOS,
            "macos",
            "aarch64-apple-darwin",
            "arm64-apple-macos",
        ),
        (
            TargetPlatform::IOS,
            "ios",
            "aarch64-apple-ios",
            "arm64-apple-ios",
        ),
        (
            TargetPlatform::IOSSimulator,
            "ios-simulator",
            "aarch64-apple-ios-sim",
            "arm64-apple-ios-simulator",
        ),
    ] {
        result.push(Slice {
            platform,
            name,
            triple,
            module_triple,
        });
    }
    Ok(result)
}

/// Build static Rust slices, then export the resolved backend's thin Swift adapter
/// and compiler-discovered resources as a local Swift package.
///
/// Cargo caches stay outside the replaced package directory. Each invocation
/// assembles a fresh package before replacing the previous successful output.
/// No executable, native project, application entry, or signing identity is used.
///
/// # Errors
/// Returns errors from target selection, compilation, resource staging or Xcode.
pub async fn build_xcframework(
    project: &Project,
    options: &BuildOptions,
    architecture: Option<Architecture>,
) -> Result<EmbeddedArtifact> {
    // The package's dependency table is read from the generated Apple
    // manifest below — it must render before anything resolves it.
    project.scaffold_apple_companion().await?;
    let selected = slices(architecture)?;
    let host = project.host();
    check_toolchain(host, &selected).await?;
    let package_parent = project.root().join("target/package");
    fs::create_dir_all(&package_parent).await?;
    let temporary_parent = package_parent.clone();
    let temporary = smol::unblock(move || {
        tempfile::Builder::new()
            .prefix(".apple-embedded-")
            .tempdir_in(temporary_parent)
    })
    .await?;
    let package = temporary.path().join("package");
    let source = package.join("Sources/WaterUI");
    fs::create_dir_all(source.join("Resources/Notices")).await?;
    fs::write(
        source.join("Exports.swift"),
        include_str!("../templates/apple_embedded/Exports.swift"),
    )
    .await?;
    let (links, manifests) = assemble_slices(project, options, &selected, temporary.path()).await?;
    stage_resources(project, &source.join("Resources"), manifests).await?;
    write_package(project, &package, &links).await?;
    let destination = package_parent.join(format!("{}-apple", project.crate_name()));
    replace_package(&package, &destination).await?;
    smol::unblock(move || temporary.close()).await?;
    Ok(EmbeddedArtifact {
        xcframework_path: destination.join("WaterUINative.xcframework"),
        package_path: destination,
    })
}

async fn check_toolchain(host: &Host, selected: &[Slice]) -> Result<()> {
    futures_util::future::try_join_all(selected.iter().map(|slice| {
        let sdk = match slice.platform {
            TargetPlatform::MacOS => crate::apple::toolchain::AppleSdk::Macos,
            TargetPlatform::IOS => crate::apple::toolchain::AppleSdk::Ios,
            TargetPlatform::IOSSimulator => crate::apple::toolchain::AppleSdk::IosSimulator,
            _ => unreachable!("slices only returns supported Apple platforms"),
        };
        crate::toolchain_checks::check_apple(host, sdk)
    }))
    .await?;
    Ok(())
}

async fn assemble_slices(
    project: &Project,
    options: &BuildOptions,
    selected: &[Slice],
    staging: &Path,
) -> Result<(
    Vec<PlatformLinks>,
    Vec<waterui_assets_planner::BundleManifest>,
)> {
    let host = project.host();
    let mut arguments = vec![OsString::from("-create-xcframework")];
    let mut links: Vec<PlatformLinks> = Vec::new();
    let mut manifests = Vec::new();
    for slice in selected {
        let directory = staging.join(slice.name);
        fs::create_dir_all(&directory).await?;
        let triple = slice.triple;
        let (built, native_links) = Box::pin(platform::build_rust_lib_with_links(
            project,
            slice.platform,
            options
                .clone()
                .with_static_runtime()
                .with_target_triple(
                    triple
                        .parse()
                        .map_err(|_| eyre::eyre!("unsupported Apple architecture {triple}"))?,
                )
                .with_output_dir(directory.join(triple)),
        ))
        .await?;
        // Stage each target's actual symbol set: platform-gated asset
        // declarations must not disappear when a later slice is built.
        let symbols = built.app_symbols().await?;
        let module = module_artifact(&built).await?;
        let archive = built.artifact;
        manifests.push(assets::plan_library_resources(project, &symbols, false).await?);
        let swift_platform = if slice.platform == TargetPlatform::MacOS {
            "macOS"
        } else {
            "iOS"
        };
        let mut closure = RuntimeClosure::new(native_links);
        closure
            .normalize_sdk_frameworks(host, slice.platform)
            .await?;
        let archive = closure
            .compose(
                slice.platform,
                project,
                &archive,
                &directory.join(format!("{triple}-closed.a")),
                &staging.join("package/Sources/WaterUI/Resources/Notices"),
            )
            .await?;
        collect_platform_links(&mut links, swift_platform, triple, closure.remaining)?;
        let framework = directory.join("WaterUIBridge.framework");
        stage_swift_framework(&archive, &module, &framework, slice).await?;
        arguments.extend([OsString::from("-framework"), framework.into_os_string()]);
    }
    arguments.extend([
        OsString::from("-output"),
        staging
            .join("package/WaterUINative.xcframework")
            .into_os_string(),
    ]);
    host.run("xcodebuild", arguments).await?;
    Ok((links, manifests))
}

/// Reads the `waterui_meta_apple_swift_module` record the backend's
/// `build.rs` emits — the `swiftc -emit-module-path` artifact, located by
/// the canonical `__wmeta` record in the built static library rather than
/// an `OUT_DIR` search.
async fn module_artifact(built: &BuiltTarget) -> Result<PathBuf> {
    let artifact = built.artifact.clone();
    let symbols = smol::unblock(move || ArtifactSymbols::read(&artifact)).await?;
    let payload = symbols
        .meta_record("waterui_meta_apple_swift_module")
        .map_err(|err| {
            eyre::eyre!("the built archive carries no WaterUIBridge module record: {err}")
        })?;
    let path = String::from_utf8(payload)
        .map_err(|err| eyre::eyre!("the WaterUIBridge module record is not UTF-8: {err}"))?;
    Ok(PathBuf::from(path))
}

/// Stages one slice's static `WaterUIBridge.framework`: the composed
/// archive as the binary (it already carries the Swift objects through
/// Cargo's static linkage), the emitted module metadata under
/// `Modules/WaterUIBridge.swiftmodule`, and the platform `Info.plist`.
async fn stage_swift_framework(
    archive: &Path,
    module: &Path,
    framework: &Path,
    slice: &Slice,
) -> Result<()> {
    fs::create_dir_all(framework.join("Modules/WaterUIBridge.swiftmodule")).await?;
    fs::copy(archive, framework.join("WaterUIBridge")).await?;
    let triple = slice.module_triple;
    fs::copy(
        module,
        framework.join(format!(
            "Modules/WaterUIBridge.swiftmodule/{triple}.swiftmodule"
        )),
    )
    .await?;
    fs::copy(
        module.with_extension("swiftdoc"),
        framework.join(format!(
            "Modules/WaterUIBridge.swiftmodule/{triple}.swiftdoc"
        )),
    )
    .await?;
    fs::copy(
        module.with_extension("swiftinterface"),
        framework.join(format!(
            "Modules/WaterUIBridge.swiftmodule/{triple}.swiftinterface"
        )),
    )
    .await?;
    fs::write(framework.join("Info.plist"), swift_framework_plist(slice)?).await?;
    Ok(())
}

/// The typed `Info.plist` shape for one slice's framework — the module is
/// the same `WaterUIBridge` on every platform; only the platform name and
/// deployment floor differ.
#[derive(serde::Serialize)]
struct SwiftFrameworkPlist<'a> {
    #[serde(rename = "CFBundleExecutable")]
    executable: &'a str,
    #[serde(rename = "CFBundleName")]
    name: &'a str,
    #[serde(rename = "CFBundleIdentifier")]
    identifier: &'a str,
    #[serde(rename = "CFBundlePackageType")]
    package_type: &'a str,
    #[serde(rename = "CFBundleInfoDictionaryVersion")]
    info_dictionary_version: &'a str,
    #[serde(rename = "CFBundleSupportedPlatforms")]
    supported_platforms: Vec<&'a str>,
}

fn swift_framework_plist(slice: &Slice) -> Result<Vec<u8>> {
    let platforms = match slice.platform {
        TargetPlatform::MacOS => vec!["MacOSX"],
        TargetPlatform::IOS => vec!["iPhoneOS"],
        TargetPlatform::IOSSimulator => vec!["iPhoneSimulator"],
        other => return Err(eyre::eyre!("unsupported framework platform {other:?}")),
    };
    let plist = SwiftFrameworkPlist {
        executable: "WaterUIBridge",
        name: "WaterUIBridge",
        identifier: "dev.waterui.WaterUIBridge",
        package_type: "FMWK",
        info_dictionary_version: "6.0",
        supported_platforms: platforms,
    };
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &plist)?;
    Ok(bytes)
}

async fn write_package(project: &Project, package: &Path, links: &[PlatformLinks]) -> Result<()> {
    let (_, macos) = platform::apple_deployment_target(project, TargetPlatform::MacOS).await?;
    let (_, ios) = platform::apple_deployment_target(project, TargetPlatform::IOS).await?;
    let manifest = PackageTemplate {
        name: project.crate_name().as_str(),
        macos: &macos,
        ios: &ios,
        links,
    }
    .render()?;
    fs::write(package.join("Package.swift"), manifest).await?;
    fs::write(
        package.join("Sources/WaterUI/WaterUIResources.swift"),
        include_str!("../templates/apple_embedded/Resources.swift"),
    )
    .await?;
    Ok(())
}

async fn stage_resources(
    project: &Project,
    destination: &std::path::Path,
    manifests: Vec<waterui_assets_planner::BundleManifest>,
) -> Result<()> {
    let manifest = merge_manifests(manifests)?;
    assets::write_library_resources(&manifest, destination).await?;
    let declarations = assets::scan_fonts(
        project,
        &project.apple_crate_path().join("Cargo.toml"),
        &[
            waterui_assets_planner::FontPlatform::Ios,
            waterui_assets_planner::FontPlatform::Macos,
        ],
    )
    .await?;
    let mut fonts = assets::resolve_fonts(project.host(), declarations).await?;
    fonts.extend(assets::scan_project_font_assets(&manifest)?);
    let font_dir = destination.join("fonts");
    fs::create_dir_all(&font_dir).await?;
    assets::copy_fonts(&fonts, &font_dir).await?;
    assets::write_font_manifest(&fonts, &font_dir, None).await?;
    Ok(())
}

fn merge_manifests(
    manifests: Vec<waterui_assets_planner::BundleManifest>,
) -> Result<waterui_assets_planner::BundleManifest> {
    let mut manifests = manifests.into_iter();
    let mut merged = manifests
        .next()
        .ok_or_else(|| eyre::eyre!("No Apple resource manifests were built"))?;
    for manifest in manifests {
        for asset in manifest.assets {
            if let Some(existing) = merged
                .assets
                .iter()
                .find(|existing| existing.logical_path == asset.logical_path)
            {
                if existing != &asset {
                    bail!(
                        "Apple slices declare conflicting assets at {}",
                        asset.logical_path.display()
                    );
                }
            } else {
                merged.assets.push(asset);
            }
        }
        for mount in manifest.mounts {
            if !merged.mounts.contains(&mount) {
                merged.mounts.push(mount);
            }
        }
    }
    merged
        .assets
        .sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    Ok(merged)
}

async fn replace_package(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    match fs::remove_dir_all(destination).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    fs::rename(source, destination).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use target_lexicon::Aarch64Architecture;

    /// A staged framework copies the archive and the module triple's
    /// metadata verbatim and decodes the typed `Info.plist`; a missing
    /// module artifact fails rather than staging a fallback.
    #[test]
    fn stage_swift_framework_stages_binary_modules_and_info() {
        let staging = tempfile::tempdir().expect("staging dir");
        let archive = staging.path().join("archive.a");
        let module = staging.path().join("WaterUIBridge.swiftmodule");
        std::fs::write(&archive, b"archive-bytes").unwrap();
        std::fs::write(&module, b"module-bytes").unwrap();
        std::fs::write(module.with_extension("swiftdoc"), b"doc-bytes").unwrap();
        std::fs::write(module.with_extension("swiftinterface"), b"interface-bytes").unwrap();
        let want_platform = [
            (TargetPlatform::MacOS, "MacOSX"),
            (TargetPlatform::IOS, "iPhoneOS"),
            (TargetPlatform::IOSSimulator, "iPhoneSimulator"),
        ];
        for slice in slices(None).expect("three default slices") {
            let framework = staging.path().join(slice.name);
            smol::block_on(stage_swift_framework(&archive, &module, &framework, &slice))
                .expect("framework stages");
            assert_eq!(
                std::fs::read(framework.join("WaterUIBridge")).unwrap(),
                b"archive-bytes",
                "{} archives the composed binary verbatim",
                slice.name
            );
            let base = format!("Modules/WaterUIBridge.swiftmodule/{}", slice.module_triple);
            for (ext, bytes) in [
                ("swiftmodule", b"module-bytes".as_slice()),
                ("swiftdoc", b"doc-bytes".as_slice()),
                ("swiftinterface", b"interface-bytes".as_slice()),
            ] {
                assert_eq!(
                    std::fs::read(framework.join(format!("{base}.{ext}"))).unwrap(),
                    bytes,
                    "{} stages {ext}",
                    slice.name
                );
            }
            let info: plist::Value =
                plist::from_file(framework.join("Info.plist")).expect("typed plist decodes");
            let dict = info.as_dictionary().unwrap();
            assert_eq!(
                dict["CFBundleExecutable"].as_string().unwrap(),
                "WaterUIBridge"
            );
            assert_eq!(
                dict["CFBundleIdentifier"].as_string().unwrap(),
                "dev.waterui.WaterUIBridge"
            );
            assert_eq!(dict["CFBundlePackageType"].as_string().unwrap(), "FMWK");
            let want = want_platform
                .iter()
                .find(|(platform, _)| *platform == slice.platform)
                .unwrap()
                .1;
            assert_eq!(
                dict["CFBundleSupportedPlatforms"].as_array().unwrap()[0]
                    .as_string()
                    .unwrap(),
                want
            );
        }
        std::fs::remove_file(module.with_extension("swiftinterface")).unwrap();
        let failed = smol::block_on(stage_swift_framework(
            &archive,
            &module,
            &staging.path().join("Broken"),
            &slices(None).unwrap()[0],
        ));
        assert!(
            failed.is_err(),
            "a missing swiftinterface fails rather than staging a fallback"
        );
    }

    #[test]
    fn default_artifact_has_only_arm64_desktop_device_and_simulator() {
        let selected = slices(None).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|slice| slice.triple)
                .collect::<Vec<_>>(),
            [
                "aarch64-apple-darwin",
                "aarch64-apple-ios",
                "aarch64-apple-ios-sim"
            ]
        );
        assert_eq!(selected[1].platform, TargetPlatform::IOS);
        assert_eq!(selected[2].platform, TargetPlatform::IOSSimulator);
    }

    #[test]
    fn unsupported_architectures_return_diagnostics() {
        for architecture in [
            Architecture::X86_64,
            Architecture::Arm(target_lexicon::ArmArchitecture::Armv7),
            Architecture::X86_32(target_lexicon::X86_32Architecture::I686),
        ] {
            let error = slices(Some(architecture)).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "Apple targets only support arm64; unsupported architecture {architecture}"
                )
            );
        }
    }

    #[test]
    fn explicit_arm64_matches_default_slices() {
        assert_eq!(
            slices(Some(Architecture::Aarch64(Aarch64Architecture::Aarch64))).unwrap(),
            slices(None).unwrap()
        );
    }

    #[test]
    fn package_uses_binary_module_and_preserves_resource_directories() {
        let links = vec![PlatformLinks {
            platform: "macOS",
            links: vec![NativeLink {
                name: "CoreFoundation".to_owned(),
                framework: true,
            }],
        }];
        let rendered = PackageTemplate {
            name: "fixture",
            macos: "26.0",
            ios: "26.0",
            links: &links,
        }
        .render()
        .unwrap();
        assert!(rendered.contains(".binaryTarget(name: \"WaterUIBridge\""));
        assert!(rendered.contains(".copy(\"Resources/waterui_assets\")"));
        assert!(rendered.contains(".enableExperimentalFeature(\"Extern\")"));
        assert!(!rendered.contains("CWaterUI"));
        for symbol in ["runtime_create", "runtime_drop", "mount", "mount_drop"] {
            assert!(rendered.contains(&format!("\"_waterui_apple_{symbol}\"")));
        }
        assert!(!rendered.contains("VideoToolbox"));
        assert!(
            rendered.contains(".linkedFramework(\"CoreFoundation\", .when(platforms: [.macOS]))")
        );
        assert!(!rendered.contains("Process()"));
        assert!(!rendered.contains("clangRuntimeLibraryDirectory"));
        assert!(rendered.contains(".copy(\"Resources/Notices\")"));
    }

    #[test]
    fn conflicting_ios_device_and_simulator_links_fail_without_merging() {
        let native = |runtime: &str| {
            ["System", runtime, runtime, "c++"]
                .map(|name| NativeLink {
                    name: name.to_owned(),
                    framework: false,
                })
                .to_vec()
        };
        let device = native("clang_rt.ios");
        let mut links = Vec::new();
        collect_platform_links(&mut links, "iOS", "aarch64-apple-ios", device.clone()).unwrap();
        let error = collect_platform_links(
            &mut links,
            "iOS",
            "aarch64-apple-ios-sim",
            native("clang_rt.iossim"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("aarch64-apple-ios-sim"));
        assert!(error.to_string().contains("clang_rt.iossim"));
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].links, device);
    }

    #[test]
    fn shared_link_contract_requires_exact_order_and_repetition() {
        let contract = ["System", "c++", "System"]
            .map(|name| NativeLink {
                name: name.to_owned(),
                framework: false,
            })
            .to_vec();
        let mut links = Vec::new();
        collect_platform_links(&mut links, "iOS", "aarch64-apple-ios", contract.clone()).unwrap();
        collect_platform_links(&mut links, "iOS", "aarch64-apple-ios-sim", contract.clone())
            .unwrap();
        assert_eq!(links.len(), 1);
        for changed in [
            vec![
                contract[1].clone(),
                contract[0].clone(),
                contract[2].clone(),
            ],
            contract[..2].to_vec(),
        ] {
            assert!(
                collect_platform_links(&mut links, "iOS", "aarch64-apple-ios-sim", changed)
                    .is_err()
            );
        }
        assert_eq!(links[0].links, contract);
    }

    fn resource_manifest(name: &str) -> waterui_assets_planner::BundleManifest {
        waterui_assets_planner::BundleManifest::new(
            PathBuf::from("/fixture"),
            PathBuf::from("/fixture/assets"),
            Vec::new(),
            vec![waterui_assets_planner::PlannedAsset::new(
                String::new(),
                PathBuf::from("/fixture/assets").join(name),
                PathBuf::from(name),
                PathBuf::from(name),
                waterui_assets_core::AssetKind::Data,
                waterui_assets_planner::AssetRole::Regular,
            )],
        )
    }

    #[test]
    fn resource_union_keeps_platform_specific_assets_and_deduplicates_shared_assets() {
        let merged = merge_manifests(vec![
            resource_manifest("desktop.json"),
            resource_manifest("phone.json"),
            resource_manifest("desktop.json"),
        ])
        .unwrap();
        assert_eq!(merged.assets.len(), 2);
        assert_eq!(merged.assets[0].logical_path, Path::new("desktop.json"));
        assert_eq!(merged.assets[1].logical_path, Path::new("phone.json"));
    }

    #[test]
    fn resource_union_rejects_conflicting_platform_definitions() {
        let first = resource_manifest("shared.json");
        let mut second = first.clone();
        second.assets[0].source_path = PathBuf::from("/other/shared.json");
        assert!(merge_manifests(vec![first, second]).is_err());
    }

    #[test]
    fn replacement_removes_obsolete_slices_and_resources() {
        smol::block_on(async {
            let root = tempfile::tempdir().unwrap();
            let old = root.path().join("old");
            let new = root.path().join("new");
            fs::create_dir_all(old.join("stale-slice")).await.unwrap();
            fs::create_dir_all(&new).await.unwrap();
            fs::write(new.join("current"), "current").await.unwrap();
            replace_package(&new, &old).await.unwrap();
            assert!(!old.join("stale-slice").exists());
            assert_eq!(
                fs::read_to_string(old.join("current")).await.unwrap(),
                "current"
            );
        });
    }
}
