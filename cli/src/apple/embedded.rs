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
    assets,
    build::{BuildOptions, NativeLink},
    platform::TargetPlatform,
    project::Project,
    toolchain::Host,
};

/// Stable local Swift package location consumed by a native host.
#[derive(Debug)]
pub struct EmbeddedArtifact {
    /// Add this directory as a local package dependency in the native host.
    pub package_path: PathBuf,
    /// Header-free binary containing the application's native Rust archive.
    pub xcframework_path: PathBuf,
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
}

fn slices(architecture: Option<Architecture>) -> Result<Vec<Slice>> {
    if let Some(architecture) = architecture {
        super::platform::validate_architecture(architecture)?;
    }
    let mut result = Vec::new();
    for (platform, name, triple) in [
        (TargetPlatform::MacOS, "macos", "aarch64-apple-darwin"),
        (TargetPlatform::IOS, "ios", "aarch64-apple-ios"),
        (
            TargetPlatform::IOSSimulator,
            "ios-simulator",
            "aarch64-apple-ios-sim",
        ),
    ] {
        result.push(Slice {
            platform,
            name,
            triple,
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
    let backend = platform::apple_backend_source_root(project).await?;
    fs::create_dir_all(source.join("Resources/Notices")).await?;
    fs::copy(
        backend.join("Sources/WaterUI/Embedding.swift"),
        source.join("Embedding.swift"),
    )
    .await?;
    let (links, manifests) =
        assemble_slices(project, options, host, &selected, temporary.path()).await?;
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
    host: &Host,
    selected: &[Slice],
    staging: &Path,
) -> Result<(
    Vec<PlatformLinks>,
    Vec<waterui_assets_planner::BundleManifest>,
)> {
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
        let (archive, symbols) = smol::unblock(move || {
            let symbols = built.app_symbols()?;
            Ok::<_, eyre::Report>((built.artifact, symbols))
        })
        .await?;
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
                host,
                slice.platform,
                project,
                &archive,
                &directory.join(format!("{triple}-closed.a")),
                &staging.join("package/Sources/WaterUI/Resources/Notices"),
            )
            .await?;
        collect_platform_links(&mut links, swift_platform, triple, closure.remaining)?;
        let library = directory.join("libWaterUINative.a");
        fs::copy(&archive, &library).await?;
        arguments.extend([OsString::from("-library"), library.into_os_string()]);
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
    let declarations =
        assets::scan_fonts(project, &project.ffi_crate_path().join("Cargo.toml")).await?;
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
        assert!(rendered.contains(".binaryTarget(name: \"WaterUINative\""));
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
        waterui_assets_planner::BundleManifest {
            crate_root: PathBuf::from("/fixture"),
            assets_root: PathBuf::from("/fixture/assets"),
            mounts: Vec::new(),
            assets: vec![waterui_assets_planner::PlannedAsset {
                mount: String::new(),
                source_path: PathBuf::from("/fixture/assets").join(name),
                relative_path: PathBuf::from(name),
                logical_path: PathBuf::from(name),
                kind: waterui_assets_core::AssetKind::Data,
                role: waterui_assets_planner::AssetRole::Regular,
            }],
        }
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
