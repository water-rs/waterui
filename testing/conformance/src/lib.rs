//! Unified visual, perceptual, and interaction conformance suite for `WaterUI`.
//!
//! `waterui-conformance` combines:
//! - A pure-Rust cross-platform NVIDIA LDR-FLIP perceptual diff engine
//!   ([`compare_flip`], [`FlipMetrics`], [`magma_heatmap`]) and blank-frame
//!   detector ([`analyze_blank_frame`]) operating directly on RGBA8 PNGs with
//!   no platform-specific decoders.
//! - Machine-readable `report.json` and self-contained static `index.html`
//!   gallery generation ([`GalleryReport`], [`write_gallery_artifacts`]).
//! - Declarative multi-stage showcase suites (`RenderCase`, `ShowcaseSuite`,
//!   `GalleryRunner`) built on `waterui-testing`, writing canonical
//!   `<suite>/<case>/<stage>.png` artifacts under `WATERUI_TEST_ARTIFACTS_DIR`
//!   (behind the `testing` feature).
//! - Standard suites for core components (`waterui_standard_suite`) and
//!   Hydrolysis `Style` packages (`hydrolysis_style_suite`).
//! - Per-component offscreen `ViewRenderer` cross-backend parity comparison
//!   (`ParityRunner`, `ParitySuite`, behind the `capture` feature).

#[cfg(feature = "testing")]
pub mod case;
pub mod error;
pub mod flip;
pub mod image;
#[cfg(feature = "capture")]
pub mod parity;
pub mod report;
#[cfg(feature = "testing")]
pub mod runner;
#[cfg(feature = "testing")]
pub mod suites;

#[cfg(feature = "testing")]
pub use case::{Appearance, RenderCase, RenderStage, ShowcaseSuite, Viewport};
pub use error::ConformanceError;
pub use flip::{
    ColorGamut, DEFAULT_BLOCK_SIZE, DEFAULT_PPD, FlipConfig, FlipMetrics, FlipResult,
    FlipThresholds, compare_flip, compare_flip_with, magma_heatmap,
};
pub use image::{BlankFrameAnalysis, BlankFrameReason, RgbaImage, analyze_blank_frame};
#[cfg(feature = "capture")]
pub use parity::{
    ParityCase, ParityCaseOutcome, ParityConfig, ParityRunOutput, ParityRunner, ParitySuite,
    ParityTarget,
};
pub use report::{
    CaseReport, GalleryArtifactPaths, GalleryReport, GallerySummary, REPORT_SCHEMA_VERSION,
    StageComparison, StageReport, SuiteReport, sanitize_artifact_segment, write_gallery_artifacts,
};
#[cfg(feature = "testing")]
pub use runner::{GalleryConfig, GalleryRunOutput, GalleryRunner};
#[cfg(feature = "testing")]
pub use suites::{
    StyleConformanceSuite, hydrolysis_style_suite, hydrolysis_style_suite_with,
    waterui_standard_suite,
};

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{
        BlankFrameReason, CaseReport, FlipConfig, FlipThresholds, GalleryReport, RgbaImage,
        StageComparison, StageReport, SuiteReport, analyze_blank_frame, compare_flip,
        compare_flip_with, write_gallery_artifacts,
    };

    fn solid_image(width: u32, height: u32, rgba: [u8; 4]) -> RgbaImage {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            pixels.extend_from_slice(&rgba);
        }
        RgbaImage::new(width, height, pixels).expect("solid_image dimensions are valid")
    }

    fn checkerboard_image(width: u32, height: u32, a: [u8; 4], b: [u8; 4]) -> RgbaImage {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let px = if (x / 4 + y / 4) % 2 == 0 { a } else { b };
                pixels.extend_from_slice(&px);
            }
        }
        RgbaImage::new(width, height, pixels).expect("checkerboard dimensions are valid")
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after UNIX_EPOCH")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "waterui-conformance-{label}-{}-{nanos}",
            process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp test directory");
        dir
    }

    #[test]
    fn flip_identical_images_report_zero_error() {
        let img = checkerboard_image(32, 32, [30, 40, 50, 255], [220, 225, 230, 255]);
        let result = compare_flip(&img, &img).expect("compare identical images");
        let metrics = result.metrics();
        assert_eq!(metrics.flip_mean, 0.0);
        assert_eq!(metrics.flip_p95, 0.0);
        assert_eq!(metrics.flip_max, 0.0);
        assert_eq!(metrics.max_block_flip_mean, 0.0);
        assert_eq!(metrics.max_local_error, 0.0);
        assert_eq!(
            FlipThresholds::strict().evaluate(&metrics),
            Vec::<String>::new()
        );
    }

    #[test]
    fn flip_black_vs_white_reports_high_error() {
        let black = solid_image(32, 32, [0, 0, 0, 255]);
        let white = solid_image(32, 32, [255, 255, 255, 255]);
        let result = compare_flip(&black, &white).expect("compare black vs white");
        let metrics = result.metrics();
        assert!(
            metrics.flip_mean > 0.8,
            "expected high FLIP error between black and white, got {}",
            metrics.flip_mean
        );
        assert!(metrics.flip_p95 >= metrics.flip_mean - 1e-9);
        assert!(metrics.flip_max >= metrics.flip_p95);
        assert!(metrics.max_block_flip_mean > 0.8);
        assert!((metrics.max_local_error - 1.0).abs() < 1e-6);
        assert_ne!(
            FlipThresholds::strict().evaluate(&metrics),
            Vec::<String>::new()
        );
    }

    #[test]
    fn flip_localized_defect_caught_by_block_metric_and_heatmap() {
        let reference = checkerboard_image(64, 64, [240, 240, 240, 255], [200, 200, 200, 255]);
        let mut candidate_bytes = reference.rgba8().to_vec();
        // Paint a 16x16 bright red patch inside an otherwise identical 64x64 frame.
        for y in 16..32 {
            for x in 16..32 {
                let offset = (y * 64 + x) * 4;
                candidate_bytes[offset..offset + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let candidate = RgbaImage::new(64, 64, candidate_bytes).expect("valid candidate");
        let result = compare_flip_with(
            &reference,
            &candidate,
            FlipConfig {
                block_size: 16,
                ..FlipConfig::default()
            },
        )
        .expect("compare localized defect");
        let metrics = result.metrics();
        assert!(
            metrics.max_block_flip_mean > metrics.flip_mean * 2.0,
            "localized 16x16 defect should make max_block_flip_mean ({}) significantly exceed global flip_mean ({})",
            metrics.max_block_flip_mean,
            metrics.flip_mean
        );
        assert!(metrics.flip_max >= metrics.flip_p95);

        let heatmap = result.magma_heatmap();
        assert_eq!(heatmap.width(), 64);
        assert_eq!(heatmap.height(), 64);
        // All magma heatmap pixels must be opaque.
        assert!(
            heatmap
                .rgba8()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| px[3] == 255)
        );
        // Inside the red patch should be noticeably brighter than the untouched corner.
        let corner_luma = u16::from(heatmap.rgba8()[0])
            + u16::from(heatmap.rgba8()[1])
            + u16::from(heatmap.rgba8()[2]);
        let patch_offset = (24 * 64 + 24) * 4;
        let patch_luma = u16::from(heatmap.rgba8()[patch_offset])
            + u16::from(heatmap.rgba8()[patch_offset + 1])
            + u16::from(heatmap.rgba8()[patch_offset + 2]);
        assert!(
            patch_luma > corner_luma,
            "heatmap should highlight the differing patch ({patch_luma}) brighter than untouched corner ({corner_luma})"
        );
    }

    #[test]
    fn blank_frame_detector_classifies_transparent_uniform_and_rendered_frames() {
        let transparent = solid_image(16, 16, [0, 0, 0, 0]);
        let analysis = analyze_blank_frame(&transparent);
        assert!(analysis.is_blank());
        assert_eq!(analysis.reason, Some(BlankFrameReason::FullyTransparent));

        let uniform = solid_image(16, 16, [255, 255, 255, 255]);
        let analysis = analyze_blank_frame(&uniform);
        assert!(analysis.is_blank());
        assert_eq!(
            analysis.reason,
            Some(BlankFrameReason::UniformColor {
                rgba: [255, 255, 255, 255]
            })
        );

        let rendered = checkerboard_image(16, 16, [10, 20, 30, 255], [240, 240, 240, 255]);
        let analysis = analyze_blank_frame(&rendered);
        assert!(!analysis.is_blank());
        assert_eq!(analysis.reason, None);
        assert!(analysis.max_channel_spread > 200);
    }

    #[test]
    fn gallery_artifacts_emit_valid_report_json_and_self_contained_html() {
        let dir = unique_temp_dir("gallery");
        let reference = checkerboard_image(32, 32, [20, 20, 20, 255], [240, 240, 240, 255]);
        let candidate = checkerboard_image(32, 32, [30, 30, 30, 255], [240, 240, 240, 255]);
        let flip = compare_flip(&reference, &candidate).expect("flip diff");

        let report = GalleryReport::new(
            "Conformance <Test> & Gallery",
            vec![SuiteReport {
                name: "demo-suite".to_string(),
                cases: vec![CaseReport {
                    name: "buttons-light".to_string(),
                    appearance: "light".to_string(),
                    viewport_width: 32,
                    viewport_height: 32,
                    scale_factor: 1.0,
                    stages: vec![StageReport {
                        name: "00_idle".to_string(),
                        image_path: "demo-suite/buttons-light/00_idle.png".to_string(),
                        candidate_label: Some("Hydrolysis".to_string()),
                        width: 32,
                        height: 32,
                        blank: analyze_blank_frame(&candidate),
                        comparison: Some(StageComparison {
                            reference_path: "demo-suite/buttons-light/00_idle__reference.png"
                                .to_string(),
                            reference_label: Some("Reference".to_string()),
                            heatmap_path: Some(
                                "demo-suite/buttons-light/00_idle__flip_heatmap.png".to_string(),
                            ),
                            metrics: flip.metrics(),
                        }),
                        passed: true,
                        failures: Vec::new(),
                    }],
                }],
            }],
        );

        let paths = write_gallery_artifacts(&dir, &report).expect("write gallery artifacts");
        assert!(paths.report_json.is_file());
        assert!(paths.index_html.is_file());

        let json_text = fs::read_to_string(&paths.report_json).expect("read report.json");
        let parsed: GalleryReport =
            serde_json::from_str(&json_text).expect("report.json round-trips cleanly");
        assert_eq!(parsed, report);

        let html = fs::read_to_string(&paths.index_html).expect("read index.html");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(!html.contains("<Test>"));
        assert!(html.contains("Conformance &#60;Test&#62; &#38; Gallery"));
        assert!(html.contains("demo-suite/buttons-light/00_idle.png"));
        assert!(html.contains("demo-suite/buttons-light/00_idle__flip_heatmap.png"));
        assert!(html.contains("filter-flip"));

        let _ = fs::remove_dir_all(&dir);
    }
}
