//! Integration tests for `waterui-conformance`.

#[cfg(feature = "capture")]
mod parity_tests {
    use core::future::{self, Future};
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use waterui_capture::{RenderError, RenderRequest, RenderResult, ViewRenderer};
    use waterui_conformance::{
        FlipThresholds, ParityCase, ParityConfig, ParityRunner, ParitySuite, ParityTarget,
    };
    use waterui_core::layout::Size;
    use waterui_core::{Environment, View};

    struct SyntheticRenderer {
        patch_color: Option<[u8; 4]>,
    }

    impl ViewRenderer for SyntheticRenderer {
        fn render_to_rgba<V: View>(
            &self,
            _view: V,
            _env: &Environment,
            request: RenderRequest,
        ) -> impl Future<Output = Result<RenderResult, RenderError>> {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "test request sizes are small positive integers"
            )]
            let width = request.size().width.max(1.0).round() as u32;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "test request sizes are small positive integers"
            )]
            let height = request.size().height.max(1.0).round() as u32;
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                for x in 0..width {
                    let in_patch = (12..28).contains(&x) && (12..28).contains(&y);
                    let pixel = if in_patch && let Some(patch) = self.patch_color {
                        patch
                    } else if (x / 8 + y / 8) % 2 == 0 {
                        [32, 48, 80, 255]
                    } else {
                        [236, 240, 248, 255]
                    };
                    rgba.extend_from_slice(&pixel);
                }
            }
            future::ready(Ok(RenderResult::new(rgba, width, height)))
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after UNIX_EPOCH")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "waterui-conformance-parity-{label}-{}-{nanos}",
            process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp parity dir");
        dir
    }

    #[test]
    fn parity_runner_compares_view_renderers_and_emits_gallery() {
        let dir = temp_dir("backends");
        let reference = SyntheticRenderer { patch_color: None };
        let matching = SyntheticRenderer { patch_color: None };
        let differing = SyntheticRenderer {
            patch_color: Some([230, 30, 40, 255]),
        };

        let runner = ParityRunner::new(
            ParityConfig::default()
                .title("Backend Parity Test")
                .backend_labels("Apple", "Hydrolysis")
                .artifact_root(&dir)
                .default_thresholds(FlipThresholds::strict()),
        );

        let env = Environment::new();
        let request = RenderRequest::new(Size::new(64.0, 64.0), Duration::from_secs(1));
        let static_outcome = runner
            .compare_view(
                ParityTarget::new("static-dispatch", "button", request),
                || (),
                &env,
                &reference,
                &matching,
            )
            .expect("static-dispatch view comparison");
        assert!(static_outcome.stage_report.passed);
        assert_eq!(static_outcome.metrics.flip_mean, 0.0);

        let suite = ParitySuite::new("apple-vs-hydrolysis")
            .with_case(ParityCase::new("card", Size::new(64.0, 64.0), || ()).appearance("light"));

        let pass_output = runner
            .run_suite(&suite, &reference, &matching)
            .expect("matching suite parity run");
        pass_output.assert_pass();
        let artifacts = pass_output
            .artifacts
            .as_ref()
            .expect("gallery artifacts emitted");
        assert!(artifacts.report_json.is_file());
        assert!(artifacts.index_html.is_file());
        assert!(dir.join("apple-vs-hydrolysis/card/00_parity.png").is_file());
        assert!(
            dir.join("apple-vs-hydrolysis/card/00_parity__reference.png")
                .is_file()
        );
        assert!(
            dir.join("apple-vs-hydrolysis/card/00_parity__flip_heatmap.png")
                .is_file()
        );

        let fail_output = runner
            .run_suite(&suite, &reference, &differing)
            .expect("differing suite parity run");
        assert!(!fail_output.report.passed());
        assert_eq!(fail_output.report.summary.failed_stages, 1);
        assert!(fail_output.outcomes[0].metrics.max_block_flip_mean > 0.2);

        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(all(feature = "testing", any(feature = "gpu", feature = "cpu")))]
mod showcase_tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use waterui::component::button::button;
    use waterui::component::vstack;
    use waterui::prelude::text;
    use waterui::reactive::{Binding, Signal as _, binding};
    use waterui::theme::color as theme_color;
    use waterui::{View, ViewExt as _};
    use waterui_conformance::{
        GalleryConfig, GalleryRunner, RenderCase, ShowcaseSuite, hydrolysis_style_suite,
        waterui_standard_suite,
    };
    use waterui_testing::TestStyle;

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after UNIX_EPOCH")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "waterui-conformance-showcase-{label}-{}-{nanos}",
            process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp showcase dir");
        dir
    }

    fn counter_view() -> impl View {
        let count: Binding<u32> = binding(0_u32);
        let inc = {
            let count = count.clone();
            move || {
                let next = count.snapshot() + 1;
                count.set(next);
            }
        };
        vstack((
            text!("Counter: {count}").headline(),
            button("Increment").bordered_prominent().action(inc),
        ))
        .spacing(12.0)
        .padding_with(16.0)
        .background(theme_color::Background)
    }

    #[test]
    fn gallery_runner_captures_multistage_case_and_compares_against_reference() {
        let ref_dir = temp_dir("reference");
        let cand_dir = temp_dir("candidate");

        let suite =
            ShowcaseSuite::new("counter-showcase").with_both_appearances("counter", |name, _| {
                RenderCase::new(name, counter_view)
                    .viewport(320, 180)
                    .interaction_stage("00_idle", |app| {
                        app.query().label("Counter: 0").assert_exists();
                    })
                    .interaction_stage("01_tapped", |app| {
                        app.query().label("Increment").tap();
                        app.query().label("Counter: 1").assert_exists();
                    })
            });

        let ref_runner = GalleryRunner::new(
            GalleryConfig::default()
                .artifact_root(&ref_dir)
                .title("Reference Run"),
        );
        let ref_output = ref_runner
            .run_suite(&suite, TestStyle::new())
            .expect("reference showcase run");
        ref_output.assert_pass();
        assert!(
            ref_dir
                .join("counter-showcase/counter-light/00_idle.png")
                .is_file()
        );
        assert!(
            ref_dir
                .join("counter-showcase/counter-dark/01_tapped.png")
                .is_file()
        );

        let cand_runner = GalleryRunner::new(
            GalleryConfig::default()
                .artifact_root(&cand_dir)
                .reference_root(&ref_dir)
                .labels("Candidate", "Baseline")
                .title("Candidate vs Baseline"),
        );
        let cand_output = cand_runner
            .run_suite(&suite, TestStyle::new())
            .expect("candidate showcase run");
        cand_output.assert_pass();
        assert_eq!(cand_output.report.summary.total_stages, 4);
        assert_eq!(cand_output.report.summary.worst_flip_mean, Some(0.0));
        assert!(
            cand_dir
                .join("counter-showcase/counter-light/00_idle__flip_heatmap.png")
                .is_file()
        );
        let artifacts = cand_output
            .artifacts
            .as_ref()
            .expect("candidate gallery emitted");
        let html = fs::read_to_string(&artifacts.index_html).expect("read index.html");
        assert!(html.contains("Candidate vs Baseline"));
        assert!(html.contains("counter-showcase/counter-light/00_idle__flip_heatmap.png"));

        let _ = fs::remove_dir_all(&ref_dir);
        let _ = fs::remove_dir_all(&cand_dir);
    }

    #[test]
    fn standard_and_hydrolysis_style_suites_pass_with_test_style() {
        let dir = temp_dir("standard");
        let runner = GalleryRunner::new(
            GalleryConfig::default()
                .artifact_root(&dir)
                .title("Standard Conformance"),
        );
        let standard = waterui_standard_suite();
        let output = runner
            .run_suite(&standard, TestStyle::new())
            .expect("run waterui_standard_suite");
        output.assert_pass();
        assert!(output.report.summary.total_cases >= 18);
        assert_eq!(output.report.summary.blank_stages, 0);

        let style_dir = temp_dir("style");
        let style_output = hydrolysis_style_suite(TestStyle::new())
            .with_config(GalleryConfig::default().artifact_root(&style_dir))
            .assert_pass();
        assert!(style_output.report.summary.total_cases >= 20);
        assert!(
            style_output
                .artifacts
                .as_ref()
                .expect("style gallery emitted")
                .index_html
                .is_file()
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&style_dir);
    }
}
