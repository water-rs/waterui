use target_lexicon::{Architecture, Triple};

use crate::{
    android::{device::AndroidDevice, toolchain::AndroidToolchain},
    build::BuildOptions,
    device::Artifact,
    platform::{PackageOptions, Platform},
    project::Project,
    utils::run_command,
};

pub struct AndroidPlatform {
    architecture: Architecture,
}

impl AndroidPlatform {
    #[must_use]
    pub const fn new(architecture: Architecture) -> Self {
        Self { architecture }
    }
}

impl Platform for AndroidPlatform {
    type Device = AndroidDevice;
    type Toolchain = AndroidToolchain;
    async fn scan(&self) -> color_eyre::eyre::Result<Vec<Self::Device>> {
        todo!()
    }

    fn toolchain(&self) -> Self::Toolchain {
        AndroidToolchain::default()
    }

    async fn clean(&self, _project: &crate::project::Project) -> color_eyre::eyre::Result<()> {
        todo!()
    }

    async fn build(
        &self,
        _project: &Project,
        _options: BuildOptions,
    ) -> color_eyre::eyre::Result<std::path::PathBuf> {
        todo!()
    }

    fn triple(&self) -> Triple {
        Triple {
            architecture: self.architecture,
            vendor: target_lexicon::Vendor::Unknown,
            operating_system: target_lexicon::OperatingSystem::Linux,
            environment: target_lexicon::Environment::Android,
            binary_format: target_lexicon::BinaryFormat::Elf,
        }
    }

    async fn package(
        &self,
        project: &Project,
        options: PackageOptions,
    ) -> color_eyre::eyre::Result<Artifact> {
        let backend = project
            .android_backend()
            .expect("Android backend must be configured");

        let project_path = backend.project_path();
        let gradlew = backend.gradlew_path();

        let (command_name, path) = if options.is_distribution() && !options.is_debug() {
            (
                "bundleRelease",
                project_path.join("app/build/outputs/bundle/release/app-release.aab"),
            )
        } else if !options.is_distribution() && !options.is_debug() {
            (
                "assembleRelease",
                project_path.join("app/build/outputs/apk/release/app-release.apk"),
            )
        } else if !options.is_distribution() && options.is_debug() {
            (
                "assembleDebug",
                project_path.join("app/build/outputs/apk/debug/app-debug.apk"),
            )
        } else if options.is_distribution() && options.is_debug() {
            (
                "bundleDebug",
                project_path.join("app/build/outputs/bundle/debug/app-debug.aab"),
            )
        } else {
            unreachable!()
        };

        run_command(
            gradlew.to_str().unwrap(),
            [
                command_name,
                "--project-dir",
                backend.project_path().to_str().unwrap(),
            ],
        )
        .await?;

        Ok(Artifact::new(project.bundle_identifier(), path))
    }
}
