//! Destination-specific compiler runtime closure for static Apple archives.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use eyre::{Context, Result, bail};
use smol::fs;
use target_lexicon::Triple;

use crate::{build::NativeLink, platform::TargetPlatform, toolchain::Host};

mod destination;

pub(super) struct RuntimeClosure {
    archives: Vec<String>,
    pub(super) remaining: Vec<NativeLink>,
}

impl RuntimeClosure {
    pub(super) fn new(links: Vec<NativeLink>) -> Self {
        let mut archives = Vec::new();
        let mut remaining = Vec::new();
        for link in links {
            if !link.framework && link.name.starts_with("clang_rt.") {
                if !archives.contains(&link.name) {
                    archives.push(link.name);
                }
            } else {
                remaining.push(link);
            }
        }
        Self {
            archives,
            remaining,
        }
    }

    pub(super) async fn normalize_sdk_frameworks(
        &mut self,
        host: &Host,
        platform: TargetPlatform,
    ) -> Result<()> {
        let sdk = platform
            .sdk_name()
            .ok_or_else(|| eyre::eyre!("No Apple SDK for {platform:?}"))?;
        let root = host.run("xcrun", ["--sdk", sdk, "--show-sdk-path"]).await?;
        let root = PathBuf::from(root.trim());
        if !root.is_absolute() {
            bail!("Invalid SDK path {}", root.display());
        }
        let names: std::collections::BTreeSet<_> = self
            .remaining
            .iter()
            .filter(|link| link.framework)
            .map(|link| link.name.clone())
            .collect();
        let dynamic = futures_util::future::try_join_all(names.into_iter().map(|name| {
            let path = root
                .join("System/Library/Frameworks")
                .join(format!("{name}.framework"))
                .join(format!("{name}.tbd"));
            async move {
                match fs::metadata(&path).await {
                    Ok(metadata) if metadata.is_file() => Ok(Some(name)),
                    Ok(_) => Ok(None),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(error),
                }
            }
        }))
        .await?;
        retain_first_dynamic_frameworks(
            &mut self.remaining,
            &dynamic.into_iter().flatten().collect(),
        );
        Ok(())
    }

    pub(super) async fn compose(
        &self,
        platform: TargetPlatform,
        project: &crate::project::Project,
        archive: &Path,
        output: &Path,
        notices: &Path,
    ) -> Result<PathBuf> {
        let host = project.host();
        let triple = platform.triple().to_string();
        if self.archives.is_empty() {
            return Ok(archive.to_path_buf());
        }
        let sdk = platform
            .sdk_name()
            .ok_or_else(|| eyre::eyre!("No Apple SDK for {platform:?}"))?;
        let resource = host
            .run("xcrun", ["--sdk", sdk, "clang", "-print-resource-dir"])
            .await?;
        let resource = PathBuf::from(resource.trim());
        if !resource.is_absolute() {
            bail!(
                "Clang returned a non-absolute resource directory: {}",
                resource.display()
            );
        }
        let mut runtimes = Vec::new();
        for name in &self.archives {
            let path = resource.join("lib/darwin").join(format!("lib{name}.a"));
            let path = fs::canonicalize(&path).await.wrap_err_with(|| {
                format!(
                    "Cannot resolve {triple} compiler runtime {}",
                    path.display()
                )
            })?;
            if !runtimes.contains(&path) {
                runtimes.push(path);
            }
        }
        let inputs = if platform == TargetPlatform::MacOS {
            let (_, deployment) =
                crate::apple::platform::apple_deployment_target(project, platform).await?;
            let directory = output.with_extension("runtime-objects");
            destination::macos_objects(host, &runtimes, &directory, &deployment).await?
        } else {
            runtimes
        };
        let args = composition_arguments(sdk, &triple, archive, output, &inputs)?;
        host.run("xcrun", args).await?;
        copy_notices(&resource, notices).await?;
        Ok(output.to_path_buf())
    }
}

// A repeated reference to the same SDK dylib adds no new archive search.
// Keep its first position; static/custom frameworks and every -l stay ordered.
fn retain_first_dynamic_frameworks(
    links: &mut Vec<NativeLink>,
    dynamic: &std::collections::BTreeSet<String>,
) {
    let mut seen = std::collections::BTreeSet::new();
    links.retain(|link| {
        !link.framework || !dynamic.contains(&link.name) || seen.insert(link.name.clone())
    });
}

fn composition_arguments(
    sdk: &str,
    triple: &str,
    archive: &Path,
    output: &Path,
    runtimes: &[PathBuf],
) -> Result<Vec<OsString>> {
    let target: Triple = triple
        .parse()
        .map_err(|_| eyre::eyre!("Invalid Apple target {triple}"))?;
    crate::apple::platform::validate_architecture(target.architecture)?;
    let arch = "arm64";
    let mut args = ["--sdk", sdk, "libtool", "-static", "-arch_only", arch, "-o"]
        .map(OsString::from)
        .to_vec();
    args.push(output.as_os_str().to_owned());
    args.push(archive.as_os_str().to_owned());
    args.extend(runtimes.iter().map(|path| path.as_os_str().to_owned()));
    Ok(args)
}

async fn copy_notices(resource: &Path, destination: &Path) -> Result<()> {
    // Keep the selected Xcode distribution's original third-party notices,
    // including its compiler runtime attribution, with the packaged objects.
    let xcode = resource
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .ok_or_else(|| {
            eyre::eyre!(
                "Cannot locate Xcode notices for compiler runtime {}",
                resource.display()
            )
        })?;
    let source = xcode.join("Contents/Resources/Acknowledgments.pdf");
    fs::create_dir_all(destination).await?;
    fs::copy(&source, destination.join("Xcode-Acknowledgments.pdf"))
        .await
        .wrap_err_with(|| {
            format!(
                "Cannot package compiler runtime notices from {}",
                source.display()
            )
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str) -> NativeLink {
        NativeLink {
            name: name.to_owned(),
            framework: false,
        }
    }

    #[test]
    fn runtime_closure_preserves_system_order_and_composes_each_runtime_once() {
        let closure = RuntimeClosure::new(
            [
                "System",
                "clang_rt.ios",
                "c++",
                "clang_rt.ios",
                "System",
                "clang_rt.profile_ios",
            ]
            .map(link)
            .to_vec(),
        );
        assert_eq!(closure.archives, ["clang_rt.ios", "clang_rt.profile_ios"]);
        assert_eq!(closure.remaining, ["System", "c++", "System"].map(link));
    }

    #[test]
    fn device_and_simulator_have_separate_archive_inputs_but_common_system_links() {
        let device = RuntimeClosure::new(["System", "clang_rt.ios", "c++"].map(link).to_vec());
        let simulator =
            RuntimeClosure::new(["System", "clang_rt.iossim", "c++"].map(link).to_vec());
        assert_eq!(device.remaining, simulator.remaining);
        assert_ne!(device.archives, simulator.archives);
        for (sdk, triple, runtime) in [
            ("iphoneos", "aarch64-apple-ios", "libclang_rt.ios.a"),
            (
                "iphonesimulator",
                "aarch64-apple-ios-sim",
                "libclang_rt.iossim.a",
            ),
        ] {
            let args = composition_arguments(
                sdk,
                triple,
                Path::new("app.a"),
                Path::new("closed.a"),
                &[PathBuf::from(runtime)],
            )
            .unwrap();
            assert_eq!(
                args,
                [
                    "--sdk",
                    sdk,
                    "libtool",
                    "-static",
                    "-arch_only",
                    "arm64",
                    "-o",
                    "closed.a",
                    "app.a",
                    runtime
                ]
                .map(OsString::from)
            );
        }
    }

    #[test]
    fn repeated_sdk_frameworks_normalize_without_reordering_libraries() {
        let framework = |name: &str| NativeLink {
            name: name.to_string(),
            framework: true,
        };
        let prefix = vec![framework("Foundation"), link("System"), framework("UIKit")];
        let mut device = prefix.clone();
        device.extend([framework("Foundation"), framework("UIKit"), link("System")]);
        let mut simulator = prefix;
        simulator.extend([framework("UIKit"), framework("Foundation"), link("System")]);
        let dynamic = ["Foundation".to_string(), "UIKit".to_string()]
            .into_iter()
            .collect();
        retain_first_dynamic_frameworks(&mut device, &dynamic);
        retain_first_dynamic_frameworks(&mut simulator, &dynamic);
        assert_eq!(device, simulator);
        assert_eq!(
            device,
            [
                framework("Foundation"),
                link("System"),
                framework("UIKit"),
                link("System")
            ]
        );
        let mut custom = vec![framework("Custom"), framework("Custom")];
        retain_first_dynamic_frameworks(&mut custom, &dynamic);
        assert_eq!(custom.len(), 2);
    }

    #[test]
    fn frameworks_are_not_folded_into_the_runtime_archive() {
        let framework = NativeLink {
            name: "clang_rt.test".to_string(),
            framework: true,
        };
        let closure = RuntimeClosure::new(vec![framework.clone()]);
        assert_eq!(closure.archives, [] as [String; 0]);
        assert_eq!(closure.remaining, [framework]);
    }
}
