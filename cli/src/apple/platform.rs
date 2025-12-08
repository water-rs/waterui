use color_eyre::eyre;
use smol::fs::{hard_link, remove_dir_all};
use target_lexicon::{OperatingSystem, Triple};

use crate::{
    apple::{
        device::AppleDevice,
        toolchain::{AppleSdk, AppleToolchain, Xcode},
    },
    build::{BuildOptions, RustBuild},
    device::Artifact,
    platform::Platform,
    project::Project,
    utils::run_command,
};

pub struct ApplePlatform {
    triple: Triple,
}

impl Platform for ApplePlatform {
    type Device = AppleDevice;
    type Toolchain = AppleToolchain;
    async fn scan(&self) -> color_eyre::eyre::Result<Vec<Self::Device>> {
        todo!()
    }

    async fn build(
        &self,
        project: &Project,
        options: BuildOptions,
    ) -> eyre::Result<std::path::PathBuf> {
        let build = RustBuild::new(project.root(), self.triple.clone());
        let target_dir = build.build_lib(options.is_release()).await?;

        // Then copy the built library to the project's build output directory

        // Use reflink if possible for efficiency
        //reflink::reflink_or_copy(from, to)

        todo!()
    }

    fn toolchain(&self) -> Self::Toolchain {
        let sdk = match self.triple.operating_system {
            OperatingSystem::MacOSX(_) => AppleSdk::Macos,
            OperatingSystem::IOS(_) => AppleSdk::Ios,
            _ => unimplemented!(),
        };
        (Xcode::default(), sdk)
    }

    fn triple(&self) -> target_lexicon::Triple {
        self.triple.clone()
    }

    async fn clean(&self, _project: &crate::project::Project) -> color_eyre::eyre::Result<()> {
        // Clean build artifacts of a specific platform

        todo!()
    }

    async fn package(
        &self,
        project: &crate::project::Project,
        options: crate::platform::PackageOptions,
    ) -> color_eyre::eyre::Result<Artifact> {
        /*

              xcodebuild \
        -scheme YourSchemeName \
        -configuration Debug \
        -sdk iphonesimulator \
        -destination 'platform=iOS Simulator,name=iPhone 15 Pro' \
        build

               */

        let backend = project
            .apple_backend()
            .expect("Apple backend must be configured");

        run_command("xcodebuild", ["-scheme", &backend.scheme, "archive"]).await?;

        /// Then package the built artifact into an IPA or DMG file
        todo!()
    }
}
