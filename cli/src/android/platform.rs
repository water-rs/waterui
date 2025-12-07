use smol::process::Command;

use crate::{
    android::{backend::AndroidBackend, device::AndroidDevice},
    platform::Platform,
    project::Project,
    utils::run_command,
};

pub struct AndroidPlatform {
    backend: AndroidBackend,
}

impl Platform for AndroidPlatform {
    type Device = AndroidDevice;
    type Toolchain = AndroidToolchain;
    async fn clean(&self, project: &Project) -> color_eyre::eyre::Result<()> {
        let project_path = project
            .android_backend()
            .expect("Android backend missing")
            .project_path();

        let gradlew = self.backend.gradlew_path();

        let mut command = Command::new(gradlew);

        let command = command.arg("clean").current_dir(&project_path);

        run_command(command).await
    }

    async fn package(
        &self,
        project: &Project,
        options: &crate::platform::PackageOptions,
    ) -> color_eyre::eyre::Result<()> {
        todo!()
    }

    async fn scan(&self) -> color_eyre::eyre::Result<Vec<Self::Device>> {
        todo!()
    }
}
