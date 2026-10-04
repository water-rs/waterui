//! ESP32 backend configuration and initialization.

use std::path::{Path, PathBuf};

use cargo_toml::Manifest as CargoManifest;
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

#[cfg(feature = "esp32")]
fn subset_font(path: &Path, ranges: &str, output_dir: &Path) -> eyre::Result<PathBuf> {
    crate::esp32::fonts::subset_into(path, ranges, output_dir)
}

#[cfg(not(feature = "esp32"))]
fn subset_font(_path: &Path, _ranges: &str, _output_dir: &Path) -> eyre::Result<PathBuf> {
    eyre::bail!("[esp32] font_ranges requires the `esp32` feature of waterui-cli")
}

/// The `[esp32]` table in `Water.toml`: the project's ESP32 device
/// configuration — chip, panel geometry, and the fonts firmware embeds.
///
/// The generated harness lives under the managed backends root; that
/// runtime state is [`Esp32Backend`], not manifest configuration.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Esp32Config {
    #[serde(
        default = "default_esp32_chip",
        skip_serializing_if = "is_default_esp32_chip"
    )]
    chip: String,
    /// Panel width in pixels the generated harness reports to dew.
    #[serde(
        default = "default_esp32_panel_width",
        skip_serializing_if = "is_default_esp32_panel_width"
    )]
    pub panel_width: u32,
    /// Panel height in pixels the generated harness reports to dew.
    #[serde(
        default = "default_esp32_panel_height",
        skip_serializing_if = "is_default_esp32_panel_height"
    )]
    pub panel_height: u32,
    /// Height in pixels of the drawing band dew flush-strategies panel
    /// updates over.
    #[serde(
        default = "default_esp32_band_height",
        skip_serializing_if = "is_default_esp32_band_height"
    )]
    pub band_height: u32,
    /// TTF/OTF binaries bundled into flash for dew text shaping, relative to
    /// the project root. Firmware has no font directory to enumerate, so a
    /// text-rendering app must list at least one face here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<PathBuf>,
    /// Unicode ranges to subset every bundled font to before embedding
    /// (e.g. `["U+0020-007E", "U+00A0-00FF"]`). Absent means the whole font
    /// is embedded. Subsetting is explicit because it silently drops glyphs
    /// outside the ranges; when set, a full Latin face shrinks from
    /// hundreds of kilobytes of flash to a few dozen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_ranges: Vec<String>,
}

/// The generated ESP32 `dew` harness, managed by the CLI under the
/// project's managed backends root. Runtime state only — the persisted
/// device configuration is [`Esp32Config`].
#[derive(Debug, Clone)]
pub struct Esp32Backend {
    /// The generated harness's directory below the managed backends root.
    project_path: PathBuf,
}

impl Esp32Config {
    /// Create a new ESP32 configuration with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            chip: default_esp32_chip(),
            panel_width: default_esp32_panel_width(),
            panel_height: default_esp32_panel_height(),
            band_height: default_esp32_band_height(),
            fonts: Vec::new(),
            font_ranges: Vec::new(),
        }
    }

    /// Set the target chip, returning the updated configuration.
    #[must_use]
    pub fn with_chip(mut self, chip: Esp32Chip) -> Self {
        self.chip = chip.id().to_string();
        self
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
    /// harness can `include_bytes!` them from wherever it lives. When
    /// `font_ranges` is configured, each font is subset to those ranges
    /// into `harness_fonts_dir` and the subset file is embedded instead.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured chip string is not a supported
    /// ESP32 chip, when a configured font file does not exist, or when
    /// subsetting fails.
    pub fn template_entry(
        &self,
        project_root: &Path,
        harness_fonts_dir: &Path,
    ) -> eyre::Result<Esp32TemplateEntry> {
        let ranges = self.font_ranges.join(",");
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
                    eyre::bail!(
                        "[esp32] fonts entry {} does not exist (resolved to {})",
                        font.display(),
                        path.display()
                    );
                }
                let path = if ranges.is_empty() {
                    path
                } else {
                    subset_font(&path, &ranges, harness_fonts_dir)?
                };
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
}

impl Default for Esp32Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32Backend {
    /// Get the path to the ESP32 harness project within the `WaterUI` project.
    #[must_use]
    pub const fn project_path(&self) -> &PathBuf {
        &self.project_path
    }

    /// Check whether generated ESP32 harness files should be regenerated.
    ///
    /// The harness is generated and managed by the CLI.
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
            .esp32_config()
            .cloned()
            .unwrap_or_default()
            .template_entry(project.root(), &backend_path.join("fonts"))?;
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

        // A manifest rendered before the backend carried the framework patch
        // tables lets `waterui-dew`'s own `waterui-*` requirements resolve
        // beside the project's copies — the recorded framework selection
        // produces the patch set the manifest must already carry.
        // The emitter (`generated_crate_patches`) prefers the checkout
        // whenever `waterui_path` resolves, so the comparison must name the
        // arms in the same order — a manifest carrying both fields emits the
        // checkout's set, and expecting the channel's would regenerate
        // forever.
        let expected_patches = match (
            &project.manifest().waterui_path,
            &project.manifest().framework,
        ) {
            (Some(waterui_path), _) => {
                let path = Path::new(waterui_path);
                let root = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    project.root().join(path)
                };
                Some(
                    crate::project_model::templates::collect_framework_checkout_patches(&root)
                        .map_err(|error| {
                            eyre::eyre!(
                                "failed to read the WaterUI checkout's patch tables at {}: {error}",
                                root.display()
                            )
                        })?,
                )
            }
            (None, Some(framework)) => Some(framework.patches()),
            (None, None) => None,
        };

        // The generated package name carries the project-root tag — a
        // manifest rendered before it did must be rewritten, or artifact
        // lookups would go looking for the tagged name.
        let package_name_matches = manifest
            .package
            .as_ref()
            .is_some_and(|package| package.name == project.esp32_backend_crate_name().as_str());

        Ok(!package_name_matches
            || !manifest.dependencies.contains_key("waterui-dew")
            || !main_matches_panel
            || !main_matches_fonts
            || !cargo_target_matches
            || expected_patches.is_some_and(|expected| manifest.patch != expected)
            || !backend_path.join("rust-toolchain.toml").exists()
            || !backend_path.join(".cargo/config.toml").exists()
            || !backend_path.join("sdkconfig.defaults").exists()
            || !backend_path.join("partitions.csv").exists()
            || !backend_path.join("build.rs").exists())
    }
}

impl Default for Esp32Backend {
    fn default() -> Self {
        Self {
            project_path: PathBuf::from("esp32"),
        }
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
        let backend = Self::default();
        let config = project.esp32_config().cloned().unwrap_or_default();

        let app_name = manifest
            .package
            .name
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>();
        let template_entry = config
            .template_entry(
                project.root(),
                &project.backend_path::<Self>().join("fonts"),
            )
            .map_err(crate::backend::FailToInitBackend::Config)?;
        if template_entry.fonts.is_empty() {
            tracing::warn!(
                "[esp32] bundles no fonts; dew fails fast at the first text layout. \
                 Add `fonts = [\"path/to/Font.ttf\"]` (relative to the project root) to render text."
            );
        }
        let ctx = TemplateContext::for_project_manifest(
            manifest,
            project.crate_name().clone(),
            app_name,
            &project
                .resolved_framework()
                .await
                .map_err(crate::backend::FailToInitBackend::Config)?,
            project.local_sources(),
        )
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
    ) -> eyre::Result<crate::build::BuiltTarget> {
        if !is_esp32_platform(platform) {
            eyre::bail!("ESP32 backend only supports the esp32s3, esp32c3, and esp32p4 platforms");
        }
        build_esp32(project, options).await
    }

    async fn package(
        &self,
        project: &Project,
        platform: TargetPlatform,
        options: PackageOptions,
        built: &crate::build::BuiltTarget,
    ) -> eyre::Result<Artifact> {
        if !is_esp32_platform(platform) {
            eyre::bail!("ESP32 backend only supports the esp32s3, esp32c3, and esp32p4 platforms");
        }
        package_esp32(project, options, built).await
    }

    async fn clean(&self, project: &Project, _platform: TargetPlatform) -> eyre::Result<()> {
        clean_esp32(project).await
    }
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
