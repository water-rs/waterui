//! Machine-readable `report.json` schema and self-contained static `index.html`
//! gallery generation.

use std::fs;
use std::path::{Path, PathBuf};

use askama::Template;
use serde::{Deserialize, Serialize};

use crate::error::VisualDiffError;
use crate::flip::FlipMetrics;
use crate::image::BlankFrameAnalysis;

/// Current schema version emitted in `report.json`.
pub const REPORT_SCHEMA_VERSION: u32 = 1;

const GALLERY_CSS: &str = include_str!("../assets/gallery.css");
const GALLERY_JS: &str = include_str!("../assets/gallery.js");

/// Top-level gallery report serialized to `report.json` and rendered into
/// `index.html`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GalleryReport {
    /// Schema version (`REPORT_SCHEMA_VERSION`).
    pub schema_version: u32,
    /// Human-readable gallery title.
    pub title: String,
    /// Aggregate summary computed across all suites, cases, and stages.
    pub summary: GallerySummary,
    /// Individual suite reports.
    pub suites: Vec<SuiteReport>,
}

impl GalleryReport {
    /// Builds a [`GalleryReport`] from suites and computes its [`GallerySummary`].
    #[must_use]
    pub fn new(title: impl Into<String>, suites: Vec<SuiteReport>) -> Self {
        let summary = GallerySummary::from_suites(&suites);
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            title: title.into(),
            summary,
            suites,
        }
    }

    /// Returns `true` when every stage across all suites passed.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.summary.failed_stages == 0
    }

    /// Serializes this report as pretty-printed JSON.
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::SerializeReport`] if JSON serialization fails.
    pub fn to_json_pretty(&self) -> Result<String, VisualDiffError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Renders this report into a self-contained static HTML gallery using
    /// Askama's compile-time typed template.
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::SerializeReport`] or
    /// [`VisualDiffError::RenderTemplate`] if serialization or rendering fails.
    pub fn render_html(&self) -> Result<String, VisualDiffError> {
        // Escape `<`, `>`, and `&` inside embedded JSON so HTML parsers can
        // never misinterpret `</script>`, `<!--`, or raw tags inside the
        // script element while preserving valid JSON.
        let report_json = serde_json::to_string(self)?
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026");
        let template = GalleryHtmlTemplate {
            title: &self.title,
            stylesheet: GALLERY_CSS,
            script: GALLERY_JS,
            report: self,
            report_json: &report_json,
        };
        Ok(template.render()?)
    }
}

/// Aggregate summary metrics across a [`GalleryReport`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GallerySummary {
    /// Total suites included in the report.
    pub total_suites: usize,
    /// Total cases across all suites.
    pub total_cases: usize,
    /// Total stages across all cases.
    pub total_stages: usize,
    /// Number of stages that passed all checks.
    pub passed_stages: usize,
    /// Number of stages that failed one or more checks.
    pub failed_stages: usize,
    /// Number of stages whose rendered frame was classified as blank.
    pub blank_stages: usize,
    /// Worst `flip_mean` observed across all compared stages, if any.
    pub worst_flip_mean: Option<f64>,
    /// Worst `max_block_flip_mean` observed across all compared stages, if any.
    pub worst_block_flip_mean: Option<f64>,
    /// Worst `max_local_error` observed across all compared stages, if any.
    pub worst_local_error: Option<f64>,
    /// Maximum EDR headroom (`>= 1.0`) observed across all HDR-measured stages,
    /// if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edr_headroom: Option<f32>,
}

impl GallerySummary {
    /// Computes aggregate summary statistics from `suites`.
    #[must_use]
    pub fn from_suites(suites: &[SuiteReport]) -> Self {
        let mut total_cases = 0;
        let mut total_stages = 0;
        let mut passed_stages = 0;
        let mut failed_stages = 0;
        let mut blank_stages = 0;
        let mut worst_flip_mean: Option<f64> = None;
        let mut worst_block_flip_mean: Option<f64> = None;
        let mut worst_local_error: Option<f64> = None;
        let mut max_edr_headroom: Option<f32> = None;

        for suite in suites {
            total_cases += suite.cases.len();
            for case in &suite.cases {
                for stage in &case.stages {
                    total_stages += 1;
                    if stage.passed {
                        passed_stages += 1;
                    } else {
                        failed_stages += 1;
                    }
                    if stage.blank.is_blank() {
                        blank_stages += 1;
                    }
                    if let Some(headroom) = stage.max_edr_headroom
                        && headroom.is_finite()
                    {
                        max_edr_headroom =
                            Some(max_edr_headroom.map_or(headroom, |cur| cur.max(headroom)));
                    }
                    if let Some(cmp) = &stage.comparison {
                        worst_flip_mean =
                            Some(worst_flip_mean.map_or(cmp.metrics.flip_mean, |cur| {
                                cur.max(cmp.metrics.flip_mean)
                            }));
                        worst_block_flip_mean = Some(
                            worst_block_flip_mean.map_or(cmp.metrics.max_block_flip_mean, |cur| {
                                cur.max(cmp.metrics.max_block_flip_mean)
                            }),
                        );
                        worst_local_error = Some(
                            worst_local_error.map_or(cmp.metrics.max_local_error, |cur| {
                                cur.max(cmp.metrics.max_local_error)
                            }),
                        );
                    }
                }
            }
        }

        Self {
            total_suites: suites.len(),
            total_cases,
            total_stages,
            passed_stages,
            failed_stages,
            blank_stages,
            worst_flip_mean,
            worst_block_flip_mean,
            worst_local_error,
            max_edr_headroom,
        }
    }
}

/// Report for a single named visual-test suite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuiteReport {
    /// Suite identifier (matches the top-level artifact directory segment).
    pub name: String,
    /// Cases executed within this suite.
    pub cases: Vec<CaseReport>,
}

/// Report for a single render case (fixed viewport, scale factor, and
/// appearance) across one or more stages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseReport {
    /// Case identifier (matches the `<case>` artifact directory segment).
    pub name: String,
    /// Color appearance label (for example `"light"` or `"dark"`).
    pub appearance: String,
    /// Logical viewport width in points.
    pub viewport_width: u32,
    /// Logical viewport height in points.
    pub viewport_height: u32,
    /// Physical pixels per logical point.
    pub scale_factor: f64,
    /// Captured stages in execution order.
    pub stages: Vec<StageReport>,
}

/// Report for one captured stage within a case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageReport {
    /// Stage identifier (matches the `<stage>.png` filename stem).
    pub name: String,
    /// Relative path to the captured PNG under the artifact root
    /// (`<suite>/<case>/<stage>.png`, forward-slash separated).
    pub image_path: String,
    /// Optional label displayed above the candidate image in the gallery (for
    /// example a backend name in cross-backend parity mode).
    pub candidate_label: Option<String>,
    /// Captured image width in physical pixels.
    pub width: u32,
    /// Captured image height in physical pixels.
    pub height: u32,
    /// Blank-frame analysis result.
    pub blank: BlankFrameAnalysis,
    /// Optional perceptual comparison against a reference render.
    pub comparison: Option<StageComparison>,
    /// Optional peak EDR headroom (`>= 1.0`, relative to SDR white) measured
    /// when this stage was captured in HDR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edr_headroom: Option<f32>,
    /// Whether this stage passed every check (blank-frame + FLIP thresholds).
    pub passed: bool,
    /// Human-readable reasons for any failed check.
    pub failures: Vec<String>,
}

/// Perceptual diff comparison metadata attached to a [`StageReport`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageComparison {
    /// Relative path to the reference PNG under the artifact root.
    pub reference_path: String,
    /// Optional label displayed above the reference image in the gallery.
    pub reference_label: Option<String>,
    /// Relative path to the Magma FLIP heatmap PNG, if written.
    pub heatmap_path: Option<String>,
    /// Computed NVIDIA FLIP and local error metrics.
    pub metrics: FlipMetrics,
}

/// Paths emitted by [`write_gallery_artifacts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GalleryArtifactPaths {
    /// Directory containing the suite PNGs, `report.json`, and `index.html`.
    pub root: PathBuf,
    /// Path to `report.json`.
    pub report_json: PathBuf,
    /// Path to `index.html`.
    pub index_html: PathBuf,
}

/// Writes `report.json` and `index.html` into `artifact_root`.
///
/// # Errors
///
/// Returns [`VisualDiffError`] if creating the directory, serializing JSON,
/// rendering the HTML template, or writing either file fails.
pub fn write_gallery_artifacts(
    artifact_root: impl AsRef<Path>,
    report: &GalleryReport,
) -> Result<GalleryArtifactPaths, VisualDiffError> {
    let root = artifact_root.as_ref().to_path_buf();
    fs::create_dir_all(&root).map_err(|source| VisualDiffError::Io {
        path: root.clone(),
        source,
    })?;

    let report_json_path = root.join("report.json");
    let json = report.to_json_pretty()?;
    fs::write(&report_json_path, json).map_err(|source| VisualDiffError::Io {
        path: report_json_path.clone(),
        source,
    })?;

    let index_html_path = root.join("index.html");
    let html = report.render_html()?;
    fs::write(&index_html_path, html).map_err(|source| VisualDiffError::Io {
        path: index_html_path.clone(),
        source,
    })?;

    Ok(GalleryArtifactPaths {
        root,
        report_json: report_json_path,
        index_html: index_html_path,
    })
}

/// Sanitizes an artifact path segment (suite, case, or stage name) to match
/// `waterui-testing`'s canonical layout.
#[must_use]
pub fn sanitize_artifact_segment(segment: &str) -> String {
    let mut sanitized = String::with_capacity(segment.len());
    let mut prev_dash = false;
    for ch in segment.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
            sanitized.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !sanitized.is_empty() {
            sanitized.push('-');
            prev_dash = true;
        }
    }
    let trimmed = sanitized.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "unnamed".to_string()
    } else {
        trimmed
    }
}

#[derive(Template)]
#[template(path = "gallery.html")]
struct GalleryHtmlTemplate<'a> {
    title: &'a str,
    stylesheet: &'a str,
    script: &'a str,
    report: &'a GalleryReport,
    report_json: &'a str,
}
