use std::path::{Path, PathBuf};

use color_eyre::eyre;
use serde::{Deserialize, Serialize};

use crate::{
    backend::Backend,
    project::Project,
    templates::{self, TemplateContext},
    water_dir,
};

#[derive(Debug, Serialize, Deserialize, Clone)]
// Warn: You cannot use both revision and local_path at the same time.
/// Configuration for the Apple backend in a `WaterUI` project.
///
/// `[backend.apple]` in `Water.toml`
pub struct AppleBackend {
    #[serde(
        default = "default_apple_project_path",
        skip_serializing_if = "is_default_apple_project_path"
    )]
    /// Path to the Apple project within the `WaterUI` project.
    pub project_path: PathBuf,
    /// The scheme to use for building the Apple project.
    pub scheme: String,
    /// The branch of the Apple backend to use.
    ///
    /// You cannot use both branch and revision at the same time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// The revision (commit hash or tag) of the Apple backend to use.
    ///
    /// You cannot use both revision and branch at the same time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Local path to the Apple backend for local dev.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_path: Option<String>,
}

impl AppleBackend {
    /// Create a new Apple backend configuration with the given scheme.
    #[must_use]
    pub fn new(scheme: impl Into<String>) -> Self {
        Self {
            project_path: default_apple_project_path(),
            scheme: scheme.into(),
            branch: None,
            revision: None,
            backend_path: None,
        }
    }

    /// Set a custom project path (defaults to "apple").
    #[must_use]
    pub fn with_project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = path.into();
        self
    }

    /// Set the local backend path for development.
    #[must_use]
    pub fn with_backend_path(mut self, path: impl Into<String>) -> Self {
        self.backend_path = Some(path.into());
        self
    }

    /// Get the path to the Apple project within the `WaterUI` project.
    #[must_use]
    pub fn project_path(&self) -> &Path {
        &self.project_path
    }
}

fn default_apple_project_path() -> PathBuf {
    PathBuf::from("apple")
}

fn is_default_apple_project_path(s: &Path) -> bool {
    s == Path::new("apple")
}

/// Scheme name used for all playground projects.
const PLAYGROUND_SCHEME: &str = "WaterUIApp";

impl AppleBackend {
    /// Initialize the Apple backend for a project.
    ///
    /// For playground projects, the backend is created in `.water/apple` with a fixed
    /// scheme name `WaterUIApp`. For regular apps, the backend is created in `apple/`
    /// with a scheme derived from the app name.
    ///
    /// # Errors
    ///
    /// Returns an error if scaffolding fails.
    pub async fn init_backend(project: &Project, playground: bool) -> eyre::Result<Self> {
        if playground {
            // Ensure .water directory is valid for current CLI version
            water_dir::ensure_valid(project.root()).await?;
        }

        let manifest = project.manifest();

        // For playground: use fixed scheme and .water/apple path
        // For app: derive scheme from app name and use apple/ path
        let (scheme, project_path) = if playground {
            (PLAYGROUND_SCHEME.to_string(), PathBuf::from(".water/apple"))
        } else {
            let app_name = manifest
                .package
                .name
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>();
            (app_name, default_apple_project_path())
        };

        let ctx = TemplateContext {
            app_display_name: manifest.package.name.clone(),
            app_name: scheme.clone(),
            // For playground, use scheme for crate_name too (Xcode uses __CRATE_NAME__ for scheme)
            crate_name: if playground {
                scheme.clone()
            } else {
                project.crate_name().to_string()
            },
            bundle_identifier: manifest.package.bundle_identifier.clone(),
            author: String::new(),
            android_backend_path: None,
            use_remote_dev_backend: manifest.waterui_path.is_none(),
            waterui_path: manifest.waterui_path.as_ref().map(PathBuf::from),
            backend_project_path: Some(project_path.clone()),
        };

        let output_dir = project.root().join(&project_path);
        templates::apple::scaffold(&output_dir, &ctx).await?;

        Ok(Self {
            project_path,
            scheme,
            branch: None,
            revision: None,
            backend_path: manifest.waterui_path.clone(),
        })
    }
}

impl Backend for AppleBackend {
    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        Self::init_backend(project, false)
            .await
            .map_err(|e| crate::backend::FailToInitBackend::Io(std::io::Error::other(e)))
    }
}
