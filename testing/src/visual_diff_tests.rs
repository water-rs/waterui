//! Unit tests for pure visual-diffing, RGBA8/PNG I/O, blank-frame analysis,
//! and static HTML gallery reporting (zero UI harness dependencies).

use std::env;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use png::{BitDepth, ColorType, Decoder};

use crate::{
    BlankFrameReason, CICP_DISPLAY_P3_PQ, CICP_DISPLAY_P3_SDR, HdrSnapshot, RgbaImage, Snapshot,
    VisualDiffError, analyze_blank_frame,
};

fn solid_pixels(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..(width * height) {
        pixels.extend_from_slice(&rgba);
    }
    pixels
}

fn solid_image(width: u32, height: u32, rgba: [u8; 4]) -> RgbaImage {
    RgbaImage::new(width, height, solid_pixels(width, height, rgba))
        .expect("solid_image dimensions are valid")
}

fn checkerboard_pixels(width: u32, height: u32, a: [u8; 4], b: [u8; 4]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let px = if (x / 4 + y / 4) % 2 == 0 { a } else { b };
            pixels.extend_from_slice(&px);
        }
    }
    pixels
}

fn checkerboard_image(width: u32, height: u32, a: [u8; 4], b: [u8; 4]) -> RgbaImage {
    RgbaImage::new(width, height, checkerboard_pixels(width, height, a, b))
        .expect("checkerboard dimensions are valid")
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after UNIX_EPOCH")
        .as_nanos();
    let dir = env::temp_dir().join(format!(
        "waterui-testing-visual-diff-{label}-{}-{nanos}",
        process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp test directory");
    dir
}

#[test]
fn blank_frame_detector_classifies_empty_transparent_uniform_and_rendered_frames() {
    let empty = Snapshot::new(0, 0, Vec::new());
    let empty_analysis = empty.analyze_blank();
    assert!(empty_analysis.is_blank());
    assert_eq!(
        empty_analysis.reason,
        Some(BlankFrameReason::EmptyDimensions)
    );

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

    let snapshot = Snapshot::from(rendered);
    assert_eq!(snapshot.analyze_blank(), analysis);
}

#[test]
fn snapshot_and_rgba_image_round_trip_png_and_conversions() {
    let dir = unique_temp_dir("png-roundtrip");
    let png_path = dir.join("nested/frame.png");
    let original = checkerboard_image(16, 12, [12, 34, 56, 255], [210, 220, 230, 255]);
    original.save_png(&png_path).expect("save RgbaImage PNG");

    let reloaded = Snapshot::load_png(&png_path).expect("load Snapshot PNG");
    assert_eq!(reloaded.width(), 16);
    assert_eq!(reloaded.height(), 12);
    assert_eq!(reloaded.rgba8(), original.rgba8());

    let png_bytes = fs::read(&png_path).expect("read saved PNG bytes");
    let decoder = Decoder::new(Cursor::new(&png_bytes));
    let reader = decoder.read_info().expect("decode PNG header");
    let cicp = reader
        .info()
        .coding_independent_code_points
        .expect("saved SDR PNG must carry Display P3 cICP chunk");
    assert_eq!(
        [
            cicp.color_primaries,
            cicp.transfer_function,
            cicp.matrix_coefficients,
            u8::from(cicp.is_video_full_range_image),
        ],
        CICP_DISPLAY_P3_SDR
    );

    let snap_path = dir.join("nested/snapshot.png");
    reloaded.save_png(&snap_path).expect("save Snapshot PNG");
    let snap_bytes = fs::read(&snap_path).expect("read Snapshot PNG bytes");
    let snap_reader = Decoder::new(Cursor::new(&snap_bytes))
        .read_info()
        .expect("decode Snapshot PNG header");
    let snap_cicp = snap_reader
        .info()
        .coding_independent_code_points
        .expect("Snapshot::save_png must carry Display P3 cICP chunk");
    assert_eq!(
        [
            snap_cicp.color_primaries,
            snap_cicp.transfer_function,
            snap_cicp.matrix_coefficients,
            u8::from(snap_cicp.is_video_full_range_image),
        ],
        CICP_DISPLAY_P3_SDR
    );

    let back_to_rgba = RgbaImage::try_from(&reloaded).expect("valid Snapshot -> RgbaImage");
    assert_eq!(back_to_rgba, original);

    let invalid = Snapshot::new(4, 4, vec![0; 8]);
    assert!(matches!(
        RgbaImage::try_from(invalid),
        Err(VisualDiffError::InvalidImageBuffer {
            width: 4,
            height: 4,
            expected: 64,
            actual: 8,
        })
    ));

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(feature = "flip")]
mod flip_tests {
    use super::*;
    use crate::{FlipConfig, FlipThresholds, compare_flip, compare_flip_with};

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

        let snap = Snapshot::from(img);
        let snap_result = snap
            .compare_flip(&snap)
            .expect("compare identical snapshots");
        assert_eq!(snap_result.metrics(), metrics);
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
        assert!(
            heatmap
                .rgba8()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| px[3] == 255)
        );
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

        let heatmap_snap = result.magma_heatmap_snapshot();
        assert_eq!(heatmap_snap.rgba8(), heatmap.rgba8());
    }

    #[test]
    fn flip_rejects_mismatched_dimensions_and_invalid_config() {
        let a = Snapshot::from(solid_image(16, 16, [10, 20, 30, 255]));
        let b = Snapshot::from(solid_image(16, 8, [10, 20, 30, 255]));
        assert!(matches!(
            a.compare_flip(&b),
            Err(VisualDiffError::DimensionMismatch {
                ref_width: 16,
                ref_height: 16,
                test_width: 16,
                test_height: 8,
            })
        ));
        assert!(matches!(
            a.compare_flip_with(
                &a,
                FlipConfig {
                    ppd: 0.0,
                    ..FlipConfig::default()
                }
            ),
            Err(VisualDiffError::InvalidConfig(_))
        ));
        assert!(matches!(
            a.compare_flip_with(
                &a,
                FlipConfig {
                    block_size: 0,
                    ..FlipConfig::default()
                }
            ),
            Err(VisualDiffError::InvalidConfig(_))
        ));
    }
}

#[cfg(feature = "gallery")]
mod gallery_tests {
    use super::*;
    use crate::{
        CaseReport, GalleryReport, StageComparison, StageReport, SuiteReport, compare_flip,
        write_gallery_artifacts,
    };

    #[test]
    fn gallery_artifacts_emit_valid_report_json_and_self_contained_html() {
        let dir = unique_temp_dir("gallery");
        let reference = checkerboard_image(32, 32, [20, 20, 20, 255], [240, 240, 240, 255]);
        let candidate = checkerboard_image(32, 32, [30, 30, 30, 255], [240, 240, 240, 255]);
        let flip = compare_flip(&reference, &candidate).expect("flip diff");

        let report = GalleryReport::new(
            "Visual <Test> & Gallery",
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
                        max_edr_headroom: Some(2.75),
                        passed: true,
                        failures: Vec::new(),
                    }],
                }],
            }],
        );

        assert_eq!(report.summary.max_edr_headroom, Some(2.75));

        let paths = write_gallery_artifacts(&dir, &report).expect("write gallery artifacts");
        assert!(paths.report_json.is_file());
        assert!(paths.index_html.is_file());

        let json_text = fs::read_to_string(&paths.report_json).expect("read report.json");
        assert!(json_text.contains("\"max_edr_headroom\": 2.75"));
        let parsed: GalleryReport =
            serde_json::from_str(&json_text).expect("report.json round-trips cleanly");
        assert_eq!(parsed, report);

        let html = fs::read_to_string(&paths.index_html).expect("read index.html");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(!html.contains("<Test>"));
        assert!(html.contains("Visual &#60;Test&#62; &#38; Gallery"));
        assert!(html.contains("demo-suite/buttons-light/00_idle.png"));
        assert!(html.contains("demo-suite/buttons-light/00_idle__flip_heatmap.png"));
        assert!(html.contains("filter-flip"));

        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn hdr_snapshot_encodes_16bit_display_p3_pq_png_with_cicp() {
    let dir = unique_temp_dir("hdr-pq-png");
    let hdr_path = dir.join("hdr_frame.png");
    // 3x1 pixels: black (0.0), SDR white (1.0 = 203 nits), and 4x EDR white (4.0 = 812 nits).
    let pixels = vec![
        0.0_f32, 0.0, 0.0, 1.0, // black
        1.0, 1.0, 1.0, 1.0, // SDR white
        4.0, 2.0, 0.5, 1.0, // HDR highlight
    ];
    let hdr = HdrSnapshot::new(3, 1, pixels).expect("valid HdrSnapshot");
    assert!((hdr.max_channel() - 4.0).abs() < 1e-6);
    assert!((hdr.max_edr_headroom() - 4.0).abs() < 1e-6);

    hdr.save_hdr_png(&hdr_path).expect("save 16-bit HDR PQ PNG");
    let bytes = fs::read(&hdr_path).expect("read HDR PQ PNG");
    let decoder = Decoder::new(Cursor::new(&bytes));
    let mut reader = decoder.read_info().expect("decode HDR PNG header");
    let info = reader.info();
    assert_eq!(info.bit_depth, BitDepth::Sixteen);
    assert_eq!(info.color_type, ColorType::Rgba);
    let cicp = info
        .coding_independent_code_points
        .expect("HDR PNG must carry Display P3 PQ cICP chunk");
    assert_eq!(
        [
            cicp.color_primaries,
            cicp.transfer_function,
            cicp.matrix_coefficients,
            u8::from(cicp.is_video_full_range_image),
        ],
        CICP_DISPLAY_P3_PQ
    );

    let mut frame_bytes = vec![0u8; reader.output_buffer_size().expect("output buffer size")];
    reader
        .next_frame(&mut frame_bytes)
        .expect("decode HDR frame");
    let words: Vec<u16> = frame_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_be_bytes(*b))
        .collect();
    // Pixel 0 (black): R=0, A=65535
    assert_eq!(words[0], 0);
    assert_eq!(words[3], u16::MAX);
    // Pixel 1 (203 nits SDR white): ST 2084 PQ(203/10000) ≈ 0.5806889 -> ~38055
    let sdr_white_pq = words[4];
    assert!(
        (37_900..=38_200).contains(&sdr_white_pq),
        "203-nit SDR white must map near PQ code 38055, got {sdr_white_pq}"
    );
    // Pixel 2 (812 nits = 4x SDR): ST 2084 PQ(812/10000) ≈ 0.7279 -> ~47703 > sdr_white_pq
    let hdr_red_pq = words[8];
    assert!(
        hdr_red_pq > sdr_white_pq + 8_000,
        "4x EDR red ({hdr_red_pq}) must be strictly brighter than SDR white ({sdr_white_pq}) in PQ"
    );

    let sdr = hdr.to_sdr_snapshot();
    assert_eq!((sdr.width(), sdr.height()), (3, 1));
    assert_eq!(sdr.rgba8()[4..8], [255, 255, 255, 255]);

    let _ = fs::remove_dir_all(&dir);
}
