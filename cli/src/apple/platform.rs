use color_eyre::eyre;
use target_lexicon::{Aarch64Architecture, Architecture, DefaultToHost, OperatingSystem, Triple};

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
    arch: Architecture,
    kind: ApplePlatformKind,
}

impl ApplePlatform {
    pub fn from_device_type_identifier(id: &str) -> Self {
        let chunks = id.split('.').collect::<Vec<_>>();

        // If it is a simualtor, then it has a same architecture as the host machine
        // Otherwise, it is an actual device, which is always arm64
        let arch = if chunks.contains(&"CoreSimulator") {
            DefaultToHost::default().0.architecture
        } else {
            Architecture::Aarch64(Aarch64Architecture::Aarch64)
        };

        todo!()
    }
}

pub enum ApplePlatformKind {
    MacOS,
    Ios,
    IosSimulator,
}

impl Platform for ApplePlatform {
    type Device = AppleDevice;
    type Toolchain = AppleToolchain;
    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {
        // Scan for Apple devices (both simulators and physical devices)
        todo!()
    }

    async fn build(
        &self,
        project: &Project,
        options: BuildOptions,
    ) -> eyre::Result<std::path::PathBuf> {
        let build = RustBuild::new(project.root(), self.triple());
        let _target_dir = build.build_lib(options.is_release()).await?;

        // Then copy the built library to the project's build output directory

        // Use reflink if possible for efficiency
        //reflink::reflink_or_copy(from, to)

        todo!()
    }

    fn toolchain(&self) -> Self::Toolchain {
        let sdk = match self.kind {
            ApplePlatformKind::MacOS => AppleSdk::Macos,
            ApplePlatformKind::Ios | ApplePlatformKind::IosSimulator => AppleSdk::Ios,
        };
        (Xcode, sdk)
    }

    fn triple(&self) -> Triple {
        todo!()
    }

    async fn clean(&self, project: &Project) -> color_eyre::eyre::Result<()> {
        // Clean build artifacts of a specific platform

        let apple_dir = project
            .apple_backend()
            .expect("Apple backend must be configured")
            .project_path();

        todo!()
    }

    async fn package(
        &self,
        project: &crate::project::Project,
        _options: crate::platform::PackageOptions,
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
