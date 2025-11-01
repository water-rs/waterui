use std::{
    path::{Path, PathBuf},
    process::Command,
};

use color_eyre::eyre::{self, bail};
use heck::ToSnakeCase;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use which::which;

use crate::{
    backend::{AnyBackend, Backend, scan_backends},
    doctor::{AnyToolchainIssue, ToolchainIssue},
    platform::Platform,
};

#[derive(Debug)]
pub struct Project {
    dir: PathBuf,
    config: ProjectConfig,
    identifier: String,
}

/// Configuration for a `WaterUI` project.
/// Usually loaded from a `Water.toml` file.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectConfig {
    app: ProjectConfigApp,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectConfigApp {
    name: String,
    author: String,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("TOML error: {0}")]
    TomlError(#[from] toml::de::Error),
    #[error("Build error: {0}")]
    BuildError(String),
    #[error("Backend {0} already exists for this project")]
    BackendExists(String),
    #[error("{0}")]
    Other(#[from] eyre::Report),
}

#[derive(Debug, Error)]
pub enum FailToPackage {
    #[error("Build error: {0}")]
    BuildError(String),
    #[error("Toolchain requirement not met")]
    RequirementNotMet(Vec<AnyToolchainIssue>),
    #[error("Packaging error: {0}")]
    Other(#[from] eyre::Report),
}

macro_rules! tool_chain_requirement {
    ($ty:ty) => {
        impl<T: ToolchainIssue> From<T> for $ty {
            fn from(issue: T) -> Self {
                let any_issue = AnyToolchainIssue::from(Box::new(issue));
                Self::RequirementNotMet(vec![any_issue])
            }
        }

        impl<T: ToolchainIssue> From<Vec<T>> for $ty {
            fn from(issues: Vec<T>) -> Self {
                let any_issues = issues
                    .into_iter()
                    .map(|issue| AnyToolchainIssue::from(Box::new(issue)))
                    .collect();
                Self::RequirementNotMet(any_issues)
            }
        }
    };
}

tool_chain_requirement!(FailToPackage);
tool_chain_requirement!(FailToRun);

#[derive(Debug, Clone, Error)]
pub enum BasicToolchainIssue {
    /// Rust is not installed
    #[error("Rust is not installed")]
    RustNotInstalled,
}

impl ToolchainIssue for BasicToolchainIssue {
    fn suggestion(&self) -> String {
        match self {
            Self::RustNotInstalled => {
                "Install Rust from https://www.rust-lang.org/tools/install".to_string()
            }
        }
    }

    fn fix(&self) -> eyre::Result<()> {
        todo!()
    }
}

fn check_requirements() -> Result<(), BasicToolchainIssue> {
    // check rust basic toolchain
    if which("rustc").is_err() {
        return Err(BasicToolchainIssue::RustNotInstalled);
    }

    Ok(())
}

/// Errors that can occur when running a project.
#[derive(Debug, Error)]
pub enum FailToRun {
    /// Build error
    #[error("Build error: {0}")]
    BuildError(String),
    /// Toolchain requirement not met
    #[error("Toolchain requirement not met")]
    RequirementNotMet(Vec<AnyToolchainIssue>),
    /// Other runtime error
    #[error("Runtime error: {0}")]
    Other(#[from] eyre::Report),
}

impl Project {
    /// Open a `WaterUI` project from the specified directory.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let dir = dir.as_ref().to_path_buf();
        let contents = std::fs::read_to_string(dir.join("Water.toml")).map_err(Error::IoError)?;
        let config: ProjectConfig = toml::from_str(&contents).map_err(Error::TomlError)?;
        let identifier = config.app.name.to_snake_case();
        Ok(Self {
            dir,
            config,
            identifier,
        })
    }

    /// Run the project on the specified platform.
    pub fn run(&self, platform: impl Platform) -> Result<(), FailToRun> {
        check_requirements()?;
        platform.check_requirements(self)?;

        self.build_rust(platform, false)?;

        // run the project on the specified platform

        todo!();

        Ok(())
    }

    /// Package the project for the specified platform.
    pub fn package(&self, platform: impl Platform) -> Result<(), FailToPackage> {
        check_requirements()?;
        platform.check_requirements(self)?;

        self.build_rust(&platform, true)?;

        platform.package(self, true)?;
        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.config.app.name
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub fn backends(&self) -> Vec<AnyBackend> {
        scan_backends(self)
    }

    pub const fn author(&self) -> &str {
        self.config.app.author.as_str()
    }

    pub fn root(&self) -> &Path {
        &self.dir
    }

    pub fn add_backend(&self, backend: impl Backend, dev: bool) -> Result<(), Error> {
        if backend.is_existing(self) {
            return Err(Error::BackendExists(backend.to_string()));
        }
        backend.init(self, dev)?;
        Ok(())
    }

    fn build_rust(&self, platform: impl Platform, release: bool) -> eyre::Result<PathBuf> {
        let target_triple = platform.target_triple();
        // use cargo to build the project for the specified target
        let status = Command::new("cargo")
            .args([
                "build",
                "--manifest-path",
                &self.root().join("Cargo.toml").to_string_lossy(),
                "--target",
                target_triple,
                if release { "--release" } else { "" },
            ])
            .status()?;

        if !status.success() {
            bail!("Failed to build project for target {}", target_triple);
        }

        Ok(PathBuf::from(format!(
            "target/{}/{}",
            target_triple,
            if release { "release" } else { "debug" }
        )))
    }
}
