//! Per-component offscreen [`ViewRenderer`] cross-backend parity runner.
//!
//! Compares two [`ViewRenderer`] implementations (for example `waterui-apple`
//! vs `hydrolysis` + `hydrolysis-glass`) component-by-component using NVIDIA
//! FLIP metrics, blank-frame checks, Magma error heatmaps, and static gallery
//! artifacts.

use core::fmt;
use core::time::Duration;
use std::env;
use std::path::PathBuf;

use waterui_capture::{RenderRequest, ViewRenderer};
use waterui_core::layout::Size;
use waterui_core::{AnyView, Environment, View};

use crate::error::ConformanceError;
use crate::flip::{FlipConfig, FlipMetrics, FlipThresholds, compare_flip_with};
use crate::image::RgbaImage;
use crate::report::{
    CaseReport, GalleryArtifactPaths, GalleryReport, StageComparison, StageReport, SuiteReport,
    sanitize_artifact_segment, write_gallery_artifacts,
};

type ViewFactory = Box<dyn Fn() -> AnyView>;
type EnvSetup = Box<dyn Fn(&mut Environment)>;

/// Configuration for [`ParityRunner`].
#[derive(Debug, Clone)]
pub struct ParityConfig {
    /// Human-readable title displayed in the generated HTML gallery.
    pub title: String,
    /// Display label for the reference backend (for example `"waterui-apple"`).
    pub reference_label: String,
    /// Display label for the candidate backend (for example `"hydrolysis-glass"`).
    pub candidate_label: String,
    /// Root directory where PNGs, `report.json`, and `index.html` are written.
    ///
    /// When `None`, falls back to `WATERUI_TEST_ARTIFACTS_DIR` or the system
    /// temporary directory (`waterui-testing-artifacts`).
    pub artifact_root: Option<PathBuf>,
    /// FLIP algorithm configuration.
    pub flip_config: FlipConfig,
    /// Default FLIP pass/fail ceilings applied unless overridden per case.
    pub default_thresholds: FlipThresholds,
    /// Maximum wait for owed view content during each `ViewRenderer` capture.
    pub content_deadline: Duration,
    /// Whether to write Magma FLIP heatmap PNGs alongside each comparison.
    pub write_heatmaps: bool,
    /// Whether to emit `report.json` and `index.html` after running a suite.
    pub emit_gallery: bool,
}

impl Default for ParityConfig {
    fn default() -> Self {
        Self {
            title: "WaterUI Cross-Backend Parity Gallery".to_string(),
            reference_label: "Reference".to_string(),
            candidate_label: "Candidate".to_string(),
            artifact_root: None,
            flip_config: FlipConfig::default(),
            default_thresholds: FlipThresholds::cross_backend_parity(),
            content_deadline: Duration::from_secs(5),
            write_heatmaps: true,
            emit_gallery: true,
        }
    }
}

impl ParityConfig {
    /// Sets the gallery title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets display labels for the reference and candidate backends.
    #[must_use]
    pub fn backend_labels(
        mut self,
        reference: impl Into<String>,
        candidate: impl Into<String>,
    ) -> Self {
        self.reference_label = reference.into();
        self.candidate_label = candidate.into();
        self
    }

    /// Overrides the artifact output directory.
    #[must_use]
    pub fn artifact_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.artifact_root = Some(path.into());
        self
    }

    /// Sets the default FLIP pass/fail thresholds.
    #[must_use]
    pub const fn default_thresholds(mut self, thresholds: FlipThresholds) -> Self {
        self.default_thresholds = thresholds;
        self
    }

    /// Sets the FLIP engine configuration.
    #[must_use]
    pub const fn flip_config(mut self, config: FlipConfig) -> Self {
        self.flip_config = config;
        self
    }

    /// Sets the per-capture content deadline passed in [`RenderRequest`].
    #[must_use]
    pub const fn content_deadline(mut self, deadline: Duration) -> Self {
        self.content_deadline = deadline;
        self
    }

    /// Resolves the effective artifact root directory.
    #[must_use]
    pub fn resolved_artifact_root(&self) -> PathBuf {
        self.artifact_root
            .clone()
            .unwrap_or_else(default_artifact_root)
    }
}

/// One component rendered through two [`ViewRenderer`] backends for parity
/// comparison.
pub struct ParityCase {
    name: String,
    size: Size,
    appearance: String,
    view_factory: ViewFactory,
    env_setup: Option<EnvSetup>,
    thresholds: Option<FlipThresholds>,
    allow_blank: bool,
}

impl fmt::Debug for ParityCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParityCase")
            .field("name", &self.name)
            .field("size", &self.size)
            .field("appearance", &self.appearance)
            .field("thresholds", &self.thresholds)
            .field("allow_blank", &self.allow_blank)
            .finish_non_exhaustive()
    }
}

impl ParityCase {
    /// Creates a parity case rendered at `size` in points.
    #[must_use]
    pub fn new<V, F>(name: impl Into<String>, size: Size, view_factory: F) -> Self
    where
        V: View + 'static,
        F: Fn() -> V + 'static,
    {
        Self {
            name: name.into(),
            size,
            appearance: "default".to_string(),
            view_factory: Box::new(move || AnyView::new(view_factory())),
            env_setup: None,
            thresholds: None,
            allow_blank: false,
        }
    }

    /// Sets the appearance label recorded in the report.
    #[must_use]
    pub fn appearance(mut self, appearance: impl Into<String>) -> Self {
        self.appearance = appearance.into();
        self
    }

    /// Configures environment entries installed before rendering on both backends.
    #[must_use]
    pub fn environment<F>(mut self, setup: F) -> Self
    where
        F: Fn(&mut Environment) + 'static,
    {
        self.env_setup = Some(Box::new(setup));
        self
    }

    /// Overrides the FLIP thresholds for this case.
    #[must_use]
    pub const fn thresholds(mut self, thresholds: FlipThresholds) -> Self {
        self.thresholds = Some(thresholds);
        self
    }

    /// Allows blank frames without failing this case.
    #[must_use]
    pub const fn allow_blank(mut self, allow: bool) -> Self {
        self.allow_blank = allow;
        self
    }

    /// Case name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Proposed size in points.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }
}

/// Named collection of [`ParityCase`] definitions.
#[derive(Debug)]
pub struct ParitySuite {
    name: String,
    cases: Vec<ParityCase>,
}

impl ParitySuite {
    /// Creates an empty parity suite.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            cases: Vec::new(),
        }
    }

    /// Appends a [`ParityCase`] to this suite.
    #[must_use]
    pub fn with_case(mut self, case: ParityCase) -> Self {
        self.cases.push(case);
        self
    }

    /// Pushes a [`ParityCase`] into this suite in place.
    pub fn add_case(&mut self, case: ParityCase) {
        self.cases.push(case);
    }

    /// Suite name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Cases in this suite.
    #[must_use]
    pub fn cases(&self) -> &[ParityCase] {
        &self.cases
    }
}

/// Target specification for a single static-dispatch [`ParityRunner::compare_view`]
/// capture.
#[derive(Debug, Clone, Copy)]
pub struct ParityTarget<'a> {
    /// Suite identifier (`<suite>` directory segment).
    pub suite: &'a str,
    /// Case identifier (`<case>` directory segment).
    pub case: &'a str,
    /// Offscreen render request passed to both [`ViewRenderer`]s.
    pub request: RenderRequest,
    /// Optional FLIP threshold override (defaults to
    /// [`ParityConfig::default_thresholds`]).
    pub thresholds: Option<FlipThresholds>,
    /// Allows blank frames without failing this target.
    pub allow_blank: bool,
}

impl<'a> ParityTarget<'a> {
    /// Creates a target with default thresholds and blank-frame rejection enabled.
    #[must_use]
    pub const fn new(suite: &'a str, case: &'a str, request: RenderRequest) -> Self {
        Self {
            suite,
            case,
            request,
            thresholds: None,
            allow_blank: false,
        }
    }

    /// Overrides FLIP thresholds for this target.
    #[must_use]
    pub const fn thresholds(mut self, thresholds: FlipThresholds) -> Self {
        self.thresholds = Some(thresholds);
        self
    }

    /// Allows blank frames without failing this target.
    #[must_use]
    pub const fn allow_blank(mut self, allow: bool) -> Self {
        self.allow_blank = allow;
        self
    }
}

/// Detailed result of comparing a single component across two [`ViewRenderer`]s.
#[derive(Debug, Clone)]
pub struct ParityCaseOutcome {
    /// Case name.
    pub case_name: String,
    /// Captured image from the reference backend.
    pub reference_image: RgbaImage,
    /// Captured image from the candidate backend.
    pub candidate_image: RgbaImage,
    /// Optional Magma FLIP error heatmap image.
    pub heatmap_image: Option<RgbaImage>,
    /// Computed FLIP metrics.
    pub metrics: FlipMetrics,
    /// Stage report entry recorded in `report.json`.
    pub stage_report: StageReport,
}

/// Aggregate result of running a [`ParitySuite`] across two [`ViewRenderer`]s.
#[derive(Debug, Clone)]
pub struct ParityRunOutput {
    /// Machine-readable report matching `report.json`.
    pub report: GalleryReport,
    /// Paths to emitted `report.json` and `index.html`, if gallery emission was enabled.
    pub artifacts: Option<GalleryArtifactPaths>,
    /// Per-case images and metrics.
    pub outcomes: Vec<ParityCaseOutcome>,
}

impl ParityRunOutput {
    /// Returns `Ok(self)` if every parity case passed, or
    /// [`ConformanceError::ConformanceFailed`] otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError::ConformanceFailed`] when one or more cases
    /// exceeded thresholds or rendered blank frames.
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

    /// Panics with a concise summary if any parity case failed.
    ///
    /// # Panics
    ///
    /// Panics when `self.report.passed()` is `false`.
    #[track_caller]
    pub fn assert_pass(&self) {
        if self.report.passed() {
            return;
        }
        let details: Vec<String> = self
            .outcomes
            .iter()
            .filter(|outcome| !outcome.stage_report.passed)
            .map(|outcome| {
                format!(
                    "{}: {}",
                    outcome.case_name,
                    outcome.stage_report.failures.join("; ")
                )
            })
            .collect();
        panic!(
            "cross-backend parity failed ({}/{}): {}",
            self.report.summary.failed_stages,
            self.report.summary.total_stages,
            details.join(" | ")
        );
    }
}

/// Executes per-component visual parity comparisons between two [`ViewRenderer`]
/// backends.
#[derive(Debug, Clone)]
pub struct ParityRunner {
    config: ParityConfig,
}

impl ParityRunner {
    /// Creates a parity runner with the supplied configuration.
    #[must_use]
    pub const fn new(config: ParityConfig) -> Self {
        Self { config }
    }

    /// Creates a parity runner with [`ParityConfig::default`].
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(ParityConfig::default())
    }

    /// Returns the active configuration.
    #[must_use]
    pub const fn config(&self) -> &ParityConfig {
        &self.config
    }

    /// Compares a single concrete view `V` across two [`ViewRenderer`]s without
    /// erasing `V` to [`AnyView`], blocking on the captures via `pollster`.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError`] if either backend fails to render,
    /// dimensions disagree, or artifact I/O fails.
    pub fn compare_view<V, F, A, B>(
        &self,
        target: ParityTarget<'_>,
        view_factory: F,
        env: &Environment,
        reference_backend: &A,
        candidate_backend: &B,
    ) -> Result<ParityCaseOutcome, ConformanceError>
    where
        V: View,
        F: Fn() -> V,
        A: ViewRenderer,
        B: ViewRenderer,
    {
        pollster::block_on(self.compare_view_async(
            target,
            view_factory,
            env,
            reference_backend,
            candidate_backend,
        ))
    }

    /// Async variant of [`Self::compare_view`] preserving static dispatch over `V`.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError`] if either backend fails to render,
    /// dimensions disagree, or artifact I/O fails.
    #[expect(
        clippy::future_not_send,
        reason = "ViewRenderer futures run on the UI thread with !Send Environment"
    )]
    pub async fn compare_view_async<V, F, A, B>(
        &self,
        target: ParityTarget<'_>,
        view_factory: F,
        env: &Environment,
        reference_backend: &A,
        candidate_backend: &B,
    ) -> Result<ParityCaseOutcome, ConformanceError>
    where
        V: View,
        F: Fn() -> V,
        A: ViewRenderer,
        B: ViewRenderer,
    {
        let reference_result = reference_backend
            .render_to_rgba(view_factory(), env, target.request)
            .await
            .map_err(|source| ConformanceError::BackendRender {
                backend: self.config.reference_label.clone(),
                case: target.case.to_string(),
                source,
            })?;
        let candidate_result = candidate_backend
            .render_to_rgba(view_factory(), env, target.request)
            .await
            .map_err(|source| ConformanceError::BackendRender {
                backend: self.config.candidate_label.clone(),
                case: target.case.to_string(),
                source,
            })?;

        let reference_image = RgbaImage::from(reference_result);
        let candidate_image = RgbaImage::from(candidate_result);
        let thresholds = target.thresholds.unwrap_or(self.config.default_thresholds);
        self.evaluate_pair(
            target.suite,
            target.case,
            reference_image,
            candidate_image,
            thresholds,
            target.allow_blank,
        )
    }

    /// Runs every case in `suite` against `reference_backend` and
    /// `candidate_backend`, blocking via `pollster`.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError`] if rendering, FLIP comparison, or artifact
    /// writing fails.
    pub fn run_suite<A, B>(
        &self,
        suite: &ParitySuite,
        reference_backend: &A,
        candidate_backend: &B,
    ) -> Result<ParityRunOutput, ConformanceError>
    where
        A: ViewRenderer,
        B: ViewRenderer,
    {
        pollster::block_on(self.run_suite_async(suite, reference_backend, candidate_backend))
    }

    /// Async variant of [`Self::run_suite`].
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError`] if rendering, FLIP comparison, or artifact
    /// writing fails.
    #[expect(
        clippy::future_not_send,
        reason = "ViewRenderer futures run on the UI thread with !Send Environment"
    )]
    pub async fn run_suite_async<A, B>(
        &self,
        suite: &ParitySuite,
        reference_backend: &A,
        candidate_backend: &B,
    ) -> Result<ParityRunOutput, ConformanceError>
    where
        A: ViewRenderer,
        B: ViewRenderer,
    {
        let mut case_reports = Vec::with_capacity(suite.cases.len());
        let mut outcomes = Vec::with_capacity(suite.cases.len());

        for case in &suite.cases {
            let mut env = Environment::new();
            if let Some(setup) = &case.env_setup {
                setup(&mut env);
            }
            let request = RenderRequest::new(case.size, self.config.content_deadline);
            let target = ParityTarget {
                suite: suite.name(),
                case: case.name(),
                request,
                thresholds: case.thresholds,
                allow_blank: case.allow_blank,
            };
            let outcome = self
                .compare_view_async(
                    target,
                    || (case.view_factory)(),
                    &env,
                    reference_backend,
                    candidate_backend,
                )
                .await?;

            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "viewport dimensions in points are small non-negative integers"
            )]
            let (vw, vh) = (
                case.size.width.max(1.0).round() as u32,
                case.size.height.max(1.0).round() as u32,
            );

            case_reports.push(CaseReport {
                name: case.name.clone(),
                appearance: case.appearance.clone(),
                viewport_width: vw,
                viewport_height: vh,
                scale_factor: 1.0,
                stages: vec![outcome.stage_report.clone()],
            });
            outcomes.push(outcome);
        }

        let suite_report = SuiteReport {
            name: suite.name.clone(),
            cases: case_reports,
        };
        let report = GalleryReport::new(self.config.title.clone(), vec![suite_report]);
        let artifacts = if self.config.emit_gallery {
            Some(write_gallery_artifacts(
                self.config.resolved_artifact_root(),
                &report,
            )?)
        } else {
            None
        };

        Ok(ParityRunOutput {
            report,
            artifacts,
            outcomes,
        })
    }

    fn evaluate_pair(
        &self,
        suite_name: &str,
        case_name: &str,
        reference_image: RgbaImage,
        candidate_image: RgbaImage,
        thresholds: FlipThresholds,
        allow_blank: bool,
    ) -> Result<ParityCaseOutcome, ConformanceError> {
        let root = self.config.resolved_artifact_root();
        let suite_seg = sanitize_artifact_segment(suite_name);
        let case_seg = sanitize_artifact_segment(case_name);
        let stage_name = "00_parity";

        let rel_candidate = format!("{suite_seg}/{case_seg}/{stage_name}.png");
        let rel_reference = format!("{suite_seg}/{case_seg}/{stage_name}__reference.png");
        candidate_image.save_png(root.join(&rel_candidate))?;
        reference_image.save_png(root.join(&rel_reference))?;

        let flip = compare_flip_with(&reference_image, &candidate_image, self.config.flip_config)?;
        let metrics = flip.metrics();

        let (heatmap_image, rel_heatmap) = if self.config.write_heatmaps {
            let heatmap = flip.magma_heatmap();
            let rel = format!("{suite_seg}/{case_seg}/{stage_name}__flip_heatmap.png");
            heatmap.save_png(root.join(&rel))?;
            (Some(heatmap), Some(rel))
        } else {
            (None, None)
        };

        let candidate_blank = candidate_image.analyze_blank();
        let reference_blank = reference_image.analyze_blank();
        let mut failures = Vec::new();
        if !allow_blank {
            if let Some(reason) = candidate_blank.reason {
                failures.push(format!(
                    "candidate `{}` frame is blank: {reason:?}",
                    self.config.candidate_label
                ));
            }
            if let Some(reason) = reference_blank.reason {
                failures.push(format!(
                    "reference `{}` frame is blank: {reason:?}",
                    self.config.reference_label
                ));
            }
        }
        failures.extend(thresholds.evaluate(&metrics));

        let stage_report = StageReport {
            name: stage_name.to_string(),
            image_path: rel_candidate,
            candidate_label: Some(self.config.candidate_label.clone()),
            width: candidate_image.width(),
            height: candidate_image.height(),
            blank: candidate_blank,
            comparison: Some(StageComparison {
                reference_path: rel_reference,
                reference_label: Some(self.config.reference_label.clone()),
                heatmap_path: rel_heatmap,
                metrics,
            }),
            passed: failures.is_empty(),
            failures,
        };

        Ok(ParityCaseOutcome {
            case_name: case_name.to_string(),
            reference_image,
            candidate_image,
            heatmap_image,
            metrics,
            stage_report,
        })
    }
}

fn default_artifact_root() -> PathBuf {
    env::var_os("WATERUI_TEST_ARTIFACTS_DIR").map_or_else(
        || env::temp_dir().join("waterui-testing-artifacts"),
        PathBuf::from,
    )
}
