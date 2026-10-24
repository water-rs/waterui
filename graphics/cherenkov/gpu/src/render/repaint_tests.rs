use crate::{Gpu, GpuConfig, interop::TextureTarget};
use cherenkov::{Draw, Engine, FrameTime, WorkingColor, kurbo::Rect};

#[test]
fn partial_frames_match_fresh_render_after_motion_and_removal()
-> Result<(), Box<dyn std::error::Error>> {
    compare_frames(false)
}

#[test]
#[cfg(target_vendor = "apple")]
fn partial_frames_keep_native_isolation_contents() -> Result<(), Box<dyn std::error::Error>> {
    compare_frames(true)
}

fn compare_frames(isolated: bool) -> Result<(), Box<dyn std::error::Error>> {
    let diagnostics = super::super::composition::Sink::new();
    let engine = Engine::<Gpu>::new(GpuConfig {
        composition_diag: Some(diagnostics.clone()),
        ..GpuConfig::default()
    })?;
    let size = (256, 192);
    let (target, _textures) = TextureTarget::new(size);
    let retained = engine.surface(target, || {})?;
    let background = WorkingColor::new([0.2, 0.3, 0.4, 0.5]);
    retained.clear_color(background);
    let output = std::env::temp_dir().join(if isolated {
        "cherenkov-partial-isolation"
    } else {
        "cherenkov-partial-raster"
    });
    std::fs::create_dir_all(&output)?;
    for (step, x) in [Some(20.0), Some(80.0), None].into_iter().enumerate() {
        let paint = |r: &mut cherenkov::Recorder| {
            let moving = |r: &mut cherenkov::Recorder| {
                if let Some(x) = x {
                    r.fill(
                        Rect::new(x, 30.0, x + 12.0, 42.0),
                        WorkingColor::new([4.0, 0.5, 0.2, 0.5]),
                    );
                }
            };
            if isolated {
                r.group(cherenkov::Group::new().opacity(0.7), |r| {
                    r.fill(Rect::new(10.0, 20.0, 14.0, 24.0), WorkingColor::WHITE);
                    r.fill(
                        Rect::new(12.0, 22.0, 16.0, 26.0),
                        WorkingColor::new([0.2, 0.6, 0.4, 0.5]),
                    );
                    moving(r);
                    r.fill(Rect::new(110.0, 60.0, 114.0, 64.0), WorkingColor::WHITE);
                });
            } else {
                moving(r);
            }
            r.fill(
                Rect::new(180.0, 100.0, 194.0, 114.0),
                WorkingColor::new([0.0, 0.8, 0.1, 0.6]),
            );
        };
        retained.update(|tx| {
            tx[retained.root()].content(retained.record(paint));
        });
        let (target, _reference_textures) = TextureTarget::new(size);
        let reference = engine.surface(target, || {})?;
        reference.clear_color(background);
        reference.update(|tx| {
            tx[reference.root()].content(reference.record(paint));
        });
        engine.render(FrameTime::now())?;
        let reports = diagnostics.take();
        let retained_report = reports
            .iter()
            .find(|report| report.surface == retained.id().raw())
            .expect("retained surface report");
        if step == 0 {
            assert_eq!(
                retained_report.repaint_pixels,
                u64::from(size.0) * u64::from(size.1)
            );
        } else {
            assert!(
                retained_report.repaint_pixels > 0
                    && retained_report.repaint_pixels < u64::from(size.0) * u64::from(size.1) / 20,
                "small update must repaint less than five percent: {} pixels",
                retained_report.repaint_pixels
            );
        }
        if isolated {
            assert!(
                reports.iter().any(|report| !report.epochs.is_empty()),
                "isolation must exercise native tile execution: {reports:?}"
            );
        }
        let actual = retained.readback()?;
        let expected = reference.readback()?;
        for (actual, expected) in actual.pixels.iter().zip(&expected.pixels) {
            for (actual, expected) in actual.iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 0.002,
                    "frame {step}: {actual} versus {expected}"
                );
            }
        }
        cherenkov_oracle::F32Image {
            width: actual.width,
            height: actual.height,
            pixels: actual.pixels,
        }
        .write_png(&output.join(format!("frame-{step}.png")))?;
    }
    Ok(())
}
