//! ESP32 backend configuration and initialization.

use std::path::{Path, PathBuf};

use cargo_toml::Manifest as CargoManifest;
use color_eyre::eyre;
use serde::{Deserialize, Serialize};

use crate::{
    backend::Backend,
    build::BuildOptions,
    device::Artifact,
    esp32::{
        chip::Esp32Chip,
        platform::{build_esp32, clean_esp32, is_esp32_platform, package_esp32},
    },
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    templates::{self, Esp32TemplateEntry, TemplateContext},
};

/// Configuration for the ESP32 backend in a `WaterUI` project.
///
/// `[backends.esp32]` in `Water.toml`
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Esp32Backend {
    #[serde(
        default = "default_esp32_project_path",
        skip_serializing_if = "is_default_esp32_project_path"
    )]
    project_path: PathBuf,
    #[serde(
        default = "default_esp32_chip",
        skip_serializing_if = "is_default_esp32_chip"
    )]
    chip: String,
    #[serde(
        default = "default_esp32_panel_width",
        skip_serializing_if = "is_default_esp32_panel_width"
    )]
    panel_width: u32,
    #[serde(
        default = "default_esp32_panel_height",
        skip_serializing_if = "is_default_esp32_panel_height"
    )]
    panel_height: u32,
    #[serde(
        default = "default_esp32_band_height",
        skip_serializing_if = "is_default_esp32_band_height"
    )]
    band_height: u32,
    /// TTF/OTF binaries bundled into flash for dew text shaping, relative to
    /// the project root. Firmware has no font directory to enumerate, so a
    /// text-rendering app must list at least one face here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fonts: Vec<PathBuf>,
}

impl Esp32Backend {
    /// Create a new ESP32 backend configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            project_path: default_esp32_project_path(),
            chip: default_esp32_chip(),
            panel_width: default_esp32_panel_width(),
            panel_height: default_esp32_panel_height(),
            band_height: default_esp32_band_height(),
            fonts: Vec::new(),
        }
    }

    /// Set a custom project path (defaults to "esp32").
    #[must_use]
    pub fn with_project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = path.into();
        self
    }

    /// Set the target chip, returning the updated configuration.
    #[must_use]
    pub fn with_chip(mut self, chip: Esp32Chip) -> Self {
        self.chip = chip.id().to_string();
        self
    }

    /// Get the path to the ESP32 harness project within the `WaterUI` project.
    #[must_use]
    pub const fn project_path(&self) -> &PathBuf {
        &self.project_path
    }

    /// Get the configured target chip identifier (e.g. "esp32s3").
    #[must_use]
    pub fn chip(&self) -> &str {
        &self.chip
    }

    /// Parse the configured chip into an [`Esp32Chip`].
    ///
    /// # Errors
    ///
    /// Returns an error when the configured chip string is not a supported
    /// ESP32 chip.
    pub fn resolved_chip(&self) -> eyre::Result<Esp32Chip> {
        self.chip.parse()
    }

    /// Get the harness parameters substituted into generated templates.
    ///
    /// Font paths are resolved against `project_root` so the generated
    /// harness can `include_bytes!` them from wherever it lives.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured chip string is not a supported
    /// ESP32 chip, or when a configured font file does not exist.
    pub fn template_entry(&self, project_root: &Path) -> eyre::Result<Esp32TemplateEntry> {
        let fonts = self
            .fonts
            .iter()
            .map(|font| {
                let path = if font.is_absolute() {
                    font.clone()
                } else {
                    project_root.join(font)
                };
                if !path.is_file() {
                    color_eyre::eyre::bail!(
                        "[backends.esp32] fonts entry {} does not exist (resolved to {})",
                        font.display(),
                        path.display()
                    );
                }
                Ok(path.to_string_lossy().into_owned())
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        Ok(Esp32TemplateEntry::new(
            self.resolved_chip()?,
            self.panel_width,
            self.panel_height,
            self.band_height,
        )
        .with_fonts(fonts))
    }

    /// Check whether generated ESP32 harness files should be regenerated.
    ///
    /// This is used by playground mode where backend glue code is fully managed by the CLI.
    ///
    /// # Errors
    ///
    /// Returns an error when the harness `Cargo.toml` exists but cannot be parsed.
    pub fn requires_regeneration(project: &Project) -> eyre::Result<bool> {
        let backend_path = project.backend_path::<Self>();
        let cargo_toml_path = backend_path.join("Cargo.toml");
        if !cargo_toml_path.exists() {
            return Ok(true);
        }

        let manifest =
            CargoManifest::<cargo_toml::Value>::from_path(&cargo_toml_path).map_err(|error| {
                eyre::eyre!("failed to parse {}: {error}", cargo_toml_path.display())
            })?;
        let main_rs = std::fs::read_to_string(backend_path.join("src/main.rs")).unwrap_or_default();
        let config = project
            .esp32_backend()
            .cloned()
            .unwrap_or_default()
            .template_entry(project.root())?;
        let main_matches_panel = main_rs.contains(&format!(
            "PanelConfig::new({}, {}, {})",
            config.panel_width, config.panel_height, config.band_height
        ));
        let main_matches_fonts = main_rs.matches("include_bytes!").count() == config.fonts.len()
            && config
                .fonts
                .iter()
                .all(|font| main_rs.contains(font.as_str()));
        let cargo_target_matches = backend_path
            .join(".cargo/config.toml")
            .exists()
            .then(|| std::fs::read_to_string(backend_path.join(".cargo/config.toml")).ok())
            .flatten()
            .is_some_and(|cargo_config| {
                cargo_config.contains(&format!("target = \"{}\"", config.resolved_target_triple()))
            });

        Ok(!manifest.dependencies.contains_key("waterui-dew")
            || !main_matches_panel
            || !main_matches_fonts
            || !cargo_target_matches
            || !backend_path.join("rust-toolchain.toml").exists()
            || !backend_path.join(".cargo/config.toml").exists()
            || !backend_path.join("sdkconfig.defaults").exists()
            || !backend_path.join("partitions.csv").exists()
            || !backend_path.join("build.rs").exists())
    }
}

impl Default for Esp32Backend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for Esp32Backend {
    const DEFAULT_PATH: &'static str = "esp32";

    // The ESP32 harness uses Cargo build cache under the project target tree.
    const CACHE_PATHS: &'static [&'static str] = &[];

    fn path(&self) -> &Path {
        &self.project_path
    }

    async fn init(project: &Project) -> Result<Self, crate::backend::FailToInitBackend> {
        let manifest = project.manifest();
        let backend = project.esp32_backend().cloned().unwrap_or_default();

        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let template_entry = backend
            .template_entry(project.root())
            .map_err(crate::backend::FailToInitBackend::Config)?;
        if template_entry.fonts.is_empty() {
            tracing::warn!(
                "[backends.esp32] bundles no fonts; dew fails fast at the first text layout. \
                 Add `fonts = [\"path/to/Font.ttf\"]` (relative to the project root) to render text."
            );
        }
        let ctx =
            TemplateContext::for_project_manifest(manifest, project.crate_name().clone(), app_name)
                .with_backend_project_path(project.backend_path::<Self>())
                .with_project_root_path(project.root().to_path_buf())
                .with_esp32(template_entry);

        templates::esp32::scaffold(&project.backend_path::<Self>(), &ctx)
            .await
            .map_err(crate::backend::FailToInitBackend::Io)?;

        Ok(backend)
    }

    fn supports(&self, platform: TargetPlatform) -> bool {
        is_esp32_platform(platform)
    }

    async fn build(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: BuildOptions,
    ) -> eyre::Result<PathBuf> {
        if !is_esp32_platform(platform) {
            color_eyre::eyre::bail!(
                "ESP32 backend only supports the esp32s3, esp32c3, and esp32p4 platforms"
            );
        }
        build_esp32(project, options).await
    }

    async fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
    ) -> eyre::Result<Artifact> {
        if !is_esp32_platform(platform) {
            color_eyre::eyre::bail!(
                "ESP32 backend only supports the esp32s3, esp32c3, and esp32p4 platforms"
            );
        }
        package_esp32(project, options).await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_esp32(project).await
    }
}

fn default_esp32_project_path() -> PathBuf {
    PathBuf::from("esp32")
}

fn is_default_esp32_project_path(path: &Path) -> bool {
    path == Path::new("esp32")
}

fn default_esp32_chip() -> String {
    "esp32s3".to_string()
}

fn is_default_esp32_chip(chip: &str) -> bool {
    chip == "esp32s3"
}

const fn default_esp32_panel_width() -> u32 {
    410
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires a reference predicate"
)]
const fn is_default_esp32_panel_width(width: &u32) -> bool {
    *width == default_esp32_panel_width()
}

const fn default_esp32_panel_height() -> u32 {
    502
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires a reference predicate"
)]
const fn is_default_esp32_panel_height(height: &u32) -> bool {
    *height == default_esp32_panel_height()
}

const fn default_esp32_band_height() -> u32 {
    16
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires a reference predicate"
)]
const fn is_default_esp32_band_height(band_height: &u32) -> bool {
    *band_height == default_esp32_band_height()
}
