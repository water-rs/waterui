//! Showcase gallery runner built on `waterui-testing`.

use core::cell::RefCell;
use std::path::{Path, PathBuf};

use waterui::theme::Theme;
use waterui_core::{AnyView, Environment};
use waterui_testing::{Style, artifact_root as default_testing_artifact_root, ui};

use crate::case::{Appearance, RenderCase, RenderStage, ShowcaseSuite};
use crate::error::ConformanceError;
use crate::flip::{FlipConfig, FlipThresholds, compare_flip_with};
use crate::image::RgbaImage;
use crate::report::{
    CaseReport, GalleryArtifactPaths, GalleryReport, StageComparison, StageReport, SuiteReport,
    sanitize_artifact_segment, write_gallery_artifacts,
};

/// Configuration for [`GalleryRunner`].
#[derive(Debug, Clone)]
pub struct GalleryConfig {
    /// Human-readable title displayed in `index.html`.
    pub title: String,
    /// Root directory where `<suite>/<case>/<stage>.png`, `report.json`, and
    /// `index.html` are written.
    ///
    /// Defaults to `waterui_testing::artifact_root()` (which honors
    /// `WATERUI_TEST_ARTIFACTS_DIR`).
    pub artifact_root: Option<PathBuf>,
    /// Optional directory containing reference PNGs laid out in the same
    /// `<suite>/<case>/<stage>.png` hierarchy. When present, every matching
    /// stage is compared with NVIDIA FLIP and emits a Magma heatmap.
    pub reference_root: Option<PathBuf>,
    /// Optional display label for rendered candidate frames.
    pub candidate_label: Option<String>,
    /// Optional display label for reference frames.
    pub reference_label: Option<String>,
    /// NVIDIA FLIP engine configuration.
    pub flip_config: FlipConfig,
    /// Default FLIP pass/fail thresholds when `reference_root` is configured.
    pub default_thresholds: FlipThresholds,
    /// Whether to write Magma FLIP heatmap PNGs for compared stages.
    pub write_heatmaps: bool,
    /// Whether to emit `report.json` and `index.html`.
    pub emit_gallery: bool,
}

impl Default for GalleryConfig {
    fn default() -> Self {
        Self {
            title: "WaterUI Visual Conformance Gallery".to_string(),
            artifact_root: None,
            reference_root: None,
            candidate_label: None,
            reference_label: None,
            flip_config: FlipConfig::default(),
            default_thresholds: FlipThresholds::strict(),
            write_heatmaps: true,
            emit_gallery: true,
        }
    }
}

impl GalleryConfig {
    /// Sets the gallery HTML title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Overrides the artifact output root directory.
    #[must_use]
    pub fn artifact_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.artifact_root = Some(path.into());
        self
    }

    /// Sets a reference artifact root for FLIP comparison.
    #[must_use]
    pub fn reference_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.reference_root = Some(path.into());
        self
    }

    /// Sets labels shown in the gallery for candidate and reference frames.
    #[must_use]
    pub fn labels(mut self, candidate: impl Into<String>, reference: impl Into<String>) -> Self {
        self.candidate_label = Some(candidate.into());
        self.reference_label = Some(reference.into());
        self
    }

    /// Sets the FLIP comparison configuration.
    #[must_use]
    pub const fn flip_config(mut self, config: FlipConfig) -> Self {
        self.flip_config = config;
        self
    }

    /// Sets the default FLIP pass/fail thresholds.
    #[must_use]
    pub const fn default_thresholds(mut self, thresholds: FlipThresholds) -> Self {
        self.default_thresholds = thresholds;
        self
    }

    /// Enables or disables writing `report.json` and `index.html`.
    #[must_use]
    pub const fn emit_gallery(mut self, emit: bool) -> Self {
        self.emit_gallery = emit;
        self
    }

    /// Resolves the effective artifact root directory.
    #[must_use]
    pub fn resolved_artifact_root(&self) -> PathBuf {
        self.artifact_root
            .clone()
            .unwrap_or_else(default_testing_artifact_root)
    }
}

/// Output produced by [`GalleryRunner`].
#[derive(Debug, Clone)]
pub struct GalleryRunOutput {
    /// Structured conformance report matching `report.json`.
    pub report: GalleryReport,
    /// Emitted gallery paths, when [`GalleryConfig::emit_gallery`] is enabled.
    pub artifacts: Option<GalleryArtifactPaths>,
}

impl GalleryRunOutput {
    /// Converts this output into `Err(ConformanceError::ConformanceFailed)` if
    /// any stage failed.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError::ConformanceFailed`] when one or more stages
    /// failed blank-frame or FLIP checks.
    pub fn into_result(self) -> Result<Self, ConformanceError> {
        if self.report.passed() {
            Ok(self)
        } else {
            Err(ConformanceError::ConformanceFailed {
                failed_stages: self.report.summary.failed_stages,
                total_stages: self.report.summary.total_stages,
            })
        }
    }

    /// Panics with a concise summary if any stage failed.
    ///
    /// # Panics
    ///
    /// Panics when `self.report.passed()` is `false`.
    #[track_caller]
    pub fn assert_pass(&self) {
        if self.report.passed() {
            return;
        }
        let mut details = Vec::new();
        for suite in &self.report.suites {
            for case in &suite.cases {
                for stage in &case.stages {
                    if !stage.passed {
                        details.push(format!(
                            "{}/{}/{}: {}",
                            suite.name,
                            case.name,
                            stage.name,
                            stage.failures.join("; ")
                        ));
                    }
                }
            }
        }
        panic!(
            "conformance gallery failed ({}/{} stages): {}",
            self.report.summary.failed_stages,
            self.report.summary.total_stages,
            details.join(" | ")
        );
    }
}

/// Executes [`ShowcaseSuite`] definitions through `waterui-testing`'s
/// offscreen renderer, writing canonical `<suite>/<case>/<stage>.png`
/// artifacts, `report.json`, and `index.html`.
#[derive(Debug, Clone)]
pub struct GalleryRunner {
    config: GalleryConfig,
}

impl GalleryRunner {
    /// Creates a runner with the supplied configuration.
    #[must_use]
    pub const fn new(config: GalleryConfig) -> Self {
        Self { config }
    }

    /// Creates a runner with [`GalleryConfig::default`].
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(GalleryConfig::default())
    }

    /// Returns the active configuration.
    #[must_use]
    pub const fn config(&self) -> &GalleryConfig {
        &self.config
    }

    /// Runs a single [`ShowcaseSuite`] under a cloneable Hydrolysis [`Style`].
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError`] if image encoding, reference comparison, or
    /// gallery emission fails.
    pub fn run_suite<S>(
        &self,
        suite: &ShowcaseSuite,
        style: S,
    ) -> Result<GalleryRunOutput, ConformanceError>
    where
        S: Style + Clone + 'static,
    {
        self.run_suites(&[suite], style)
    }

    /// Runs multiple [`ShowcaseSuite`]s under a cloneable Hydrolysis [`Style`],
    /// combining them into one `report.json` and `index.html`.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError`] if image encoding, reference comparison, or
    /// gallery emission fails.
    pub fn run_suites<S>(
        &self,
        suites: &[&ShowcaseSuite],
        style: S,
    ) -> Result<GalleryRunOutput, ConformanceError>
    where
        S: Style + Clone + 'static,
    {
        self.run_suites_with(suites, move |_| style.clone())
    }

    /// Runs multiple [`ShowcaseSuite`]s using a factory that builds a [`Style`]
    /// per [`Appearance`].
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError`] if image encoding, reference comparison, or
    /// gallery emission fails.
    pub fn run_suites_with<S, F>(
        &self,
        suites: &[&ShowcaseSuite],
        style_factory: F,
    ) -> Result<GalleryRunOutput, ConformanceError>
    where
        S: Style,
        F: Fn(Appearance) -> S,
    {
        let root = self.config.resolved_artifact_root();
        let mut suite_reports = Vec::with_capacity(suites.len());

        for suite in suites {
            let mut case_reports = Vec::with_capacity(suite.cases.len());
            for case in &suite.cases {
                let style = style_factory(case.appearance);
                case_reports.push(self.run_case(&root, suite.name(), case, style)?);
            }
            suite_reports.push(SuiteReport {
                name: suite.name.clone(),
                cases: case_reports,
            });
        }

        let report = GalleryReport::new(self.config.title.clone(), suite_reports);
        let artifacts = if self.config.emit_gallery {
            Some(write_gallery_artifacts(&root, &report)?)
        } else {
            None
        };

        Ok(GalleryRunOutput { report, artifacts })
    }

    fn run_case<S: Style>(
        &self,
        root: &Path,
        suite_name: &str,
        case: &RenderCase,
        style: S,
    ) -> Result<CaseReport, ConformanceError> {
        let mut env = Environment::new();
        env.install(Theme::new().color_scheme(case.appearance.color_scheme()));
        if let Some(setup) = &case.env_setup {
            setup(&mut env);
        }

        // Build the initial view once per case mount; `UiBuilder::mount_offscreen`
        // invokes the closure once at mount. We move an owned `Option<AnyView>`
        // inside a `RefCell` so the view factory stays owned by `RenderCase`.
        let initial_view = RefCell::new(Some((case.view_factory)()));
        let mut app = ui()
            .environment(env)
            .viewport(case.viewport.width, case.viewport.height)
            .scale_factor(case.viewport.scale_factor)
            .theme(style)
            .mount_offscreen(move || {
                initial_view
                    .borrow_mut()
                    .take()
                    .unwrap_or_else(|| AnyView::new(()))
            });

        let default_stage;
        let stages: &[RenderStage] = if case.stages.is_empty() {
            default_stage = [RenderStage::capture("00_initial")];
            &default_stage
        } else {
            &case.stages
        };

        let suite_seg = sanitize_artifact_segment(suite_name);
        let case_seg = sanitize_artifact_segment(case.name());
        let mut stage_reports = Vec::with_capacity(stages.len());

        for stage in stages {
            if let Some(action) = &stage.action {
                action(&mut app);
            }

            let image = RgbaImage::from(app.snapshot());
            let stage_seg = sanitize_artifact_segment(stage.name());
            let rel_image = format!("{suite_seg}/{case_seg}/{stage_seg}.png");
            image.save_png(root.join(&rel_image))?;

            let blank = image.analyze_blank();
            let mut failures = Vec::new();
            if !stage.allow_blank
                && let Some(reason) = blank.reason
            {
                failures.push(format!("rendered frame is blank: {reason:?}"));
            }

            let comparison = if let Some(ref_root) = &self.config.reference_root {
                let ref_src = ref_root.join(&rel_image);
                if ref_src.is_file() {
                    let ref_img = RgbaImage::load_png(&ref_src)?;
                    let rel_ref = format!("{suite_seg}/{case_seg}/{stage_seg}__reference.png");
                    ref_img.save_png(root.join(&rel_ref))?;

                    let flip = compare_flip_with(&ref_img, &image, self.config.flip_config)?;
                    let metrics = flip.metrics();
                    let thresholds = stage.thresholds.unwrap_or(self.config.default_thresholds);
                    failures.extend(thresholds.evaluate(&metrics));

                    let heatmap_path = if self.config.write_heatmaps {
                        let rel_heat =
                            format!("{suite_seg}/{case_seg}/{stage_seg}__flip_heatmap.png");
                        flip.magma_heatmap().save_png(root.join(&rel_heat))?;
                        Some(rel_heat)
                    } else {
                        None
                    };

                    Some(StageComparison {
                        reference_path: rel_ref,
                        reference_label: self.config.reference_label.clone(),
                        heatmap_path,
                        metrics,
                    })
                } else {
                    None
                }
            } else {
                None
            };

            stage_reports.push(StageReport {
                name: stage.name.clone(),
                image_path: rel_image,
                candidate_label: self.config.candidate_label.clone(),
                width: image.width(),
                height: image.height(),
                blank,
                comparison,
                passed: failures.is_empty(),
                failures,
            });
        }

        Ok(CaseReport {
            name: case.name.clone(),
            appearance: case.appearance.as_str().to_string(),
            viewport_width: case.viewport.width,
            viewport_height: case.viewport.height,
            scale_factor: case.viewport.scale_factor,
            stages: stage_reports,
        })
    }
}
