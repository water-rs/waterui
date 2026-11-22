use std::sync::mpsc::Receiver;

use crate::{Gpu, GpuConfig, interop::TextureTarget};
use cherenkov::{
    BackdropGroup, CaptureScale, Draw, Engine, FrameTime, Layer, Recorder, Surface, WorkingColor,
    kurbo::Rect,
};

struct LiveSigma(std::sync::mpsc::Sender<filtrate_core::AnimatedCallback>);

impl filtrate_core::FilterParam for LiveSigma {
    fn snapshot(&self) -> f32 {
        1.0
    }

    fn watch_animated(
        &self,
        callback: filtrate_core::AnimatedCallback,
    ) -> filtrate_core::WatchGuard {
        self.0.send(callback).expect("parameter receiver");
        filtrate_core::WatchGuard::new(())
    }
}

#[test]
fn partial_frames_follow_retained_filter_parameters() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::<Gpu>::new(GpuConfig::default())?;
    let (send, receive) = std::sync::mpsc::channel();
    let blur = engine.filter(filtrate::filters::GaussianBlur::new(LiveSigma(send)));
    let size = (256, 192);
    let (target, _textures) = TextureTarget::new(size);
    let retained = engine.surface(target, || {})?;
    let background = WorkingColor::new([0.2, 0.3, 0.4, 0.5]);
    retained.clear_color(background);
    retained.update(|tx| {
        tx[retained.root()].content(retained.record(|r| {
            paint_frame(r, Some(80.0), false, Some(blur.id()));
        }));
    });
    engine.render(FrameTime::now())?;
    let change = receive.recv_timeout(std::time::Duration::from_secs(5))?;
    let output = std::env::temp_dir().join("cherenkov-partial-live-blur");
    std::fs::create_dir_all(&output)?;
    for (step, sigma) in [3.0f32, 1.0, 4.0, 0.5].into_iter().enumerate() {
        change(filtrate_core::AnimatedTarget {
            value: sigma,
            interpolator: None,
        });
        engine.render(FrameTime::now())?;
        let reference_blur = engine.filter(filtrate::filters::GaussianBlur::new(sigma));
        let (target, _reference_textures) = TextureTarget::new(size);
        let reference = engine.surface(target, || {})?;
        reference.clear_color(background);
        reference.update(|tx| {
            tx[reference.root()].content(reference.record(|r| {
                paint_frame(r, Some(80.0), false, Some(reference_blur.id()));
            }));
        });
        engine.render(FrameTime::now())?;
        let actual = retained.readback()?;
        let expected = reference.readback()?;
        let actual = cherenkov_oracle::F32Image {
            width: actual.width,
            height: actual.height,
            pixels: actual.pixels,
        };
        let expected = cherenkov_oracle::F32Image {
            width: expected.width,
            height: expected.height,
            pixels: expected.pixels,
        };
        actual.write_png(&output.join(format!("frame-{step}.png")))?;
        cherenkov_oracle::metrics::assert_images_match(&expected, &actual);
    }
    Ok(())
}

#[test]
fn partial_frames_match_fresh_render_after_motion_and_removal()
-> Result<(), Box<dyn std::error::Error>> {
    compare_frames(false, false)
}

#[test]
#[cfg(target_vendor = "apple")]
fn partial_frames_keep_native_isolation_contents() -> Result<(), Box<dyn std::error::Error>> {
    compare_frames(true, false)
}

#[test]
fn partial_frames_match_fresh_render_after_blur_replacement()
-> Result<(), Box<dyn std::error::Error>> {
    compare_frames(false, true)
}

fn compare_frames(isolated: bool, blurred: bool) -> Result<(), Box<dyn std::error::Error>> {
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
    let output = std::env::temp_dir().join(if blurred {
        "cherenkov-partial-blur"
    } else if isolated {
        "cherenkov-partial-isolation"
    } else {
        "cherenkov-partial-raster"
    });
    std::fs::create_dir_all(&output)?;
    for (step, (x, sigma)) in [
        (Some(20.0), 1.0f32),
        (Some(80.0), 3.0),
        (Some(80.0), 1.0),
        (None, 3.0),
        (Some(20.0), 2.0),
    ]
    .into_iter()
    .enumerate()
    {
        let blur = blurred.then(|| engine.filter(filtrate::filters::GaussianBlur::new(sigma)));
        let paint = |r: &mut cherenkov::Recorder| {
            paint_frame(r, x, isolated, blur.as_ref().map(cherenkov::Filter::id));
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
            .find(|report| report.surface == retained.id().raw());
        if !blurred && step == 2 {
            assert!(
                retained_report.is_none(),
                "identical replacement skips rendering"
            );
        } else if step == 0 {
            let retained_report = retained_report.expect("initial surface report");
            assert_eq!(
                retained_report.repaint_pixels,
                u64::from(size.0) * u64::from(size.1)
            );
        } else if !blurred {
            let retained_report = retained_report.expect("changed surface report");
            // Re-recording replaces both the moving shape and the distant
            // fixed shape. One scissor encloses them, including the clean gap,
            // but never reaches the rest of the 256 x 192 attachment.
            assert!(
                retained_report.repaint_pixels > 0 && retained_report.repaint_pixels <= 200 * 100,
                "replacement must stay inside the content's enclosing redraw bounds: {} pixels",
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
        let actual = cherenkov_oracle::F32Image {
            width: actual.width,
            height: actual.height,
            pixels: actual.pixels,
        };
        let expected = cherenkov_oracle::F32Image {
            width: expected.width,
            height: expected.height,
            pixels: expected.pixels,
        };
        actual.write_png(&output.join(format!("frame-{step}.png")))?;
        cherenkov_oracle::metrics::assert_images_match(&expected, &actual);
    }
    Ok(())
}

fn paint_frame(
    r: &mut cherenkov::Recorder,
    x: Option<f64>,
    isolated: bool,
    blur: Option<cherenkov::FilterId>,
) {
    let moving = |r: &mut cherenkov::Recorder| {
        if let Some(x) = x {
            r.fill(
                Rect::new(x, 30.0, x + 12.0, 42.0),
                WorkingColor::new([4.0, 0.5, 0.2, 0.5]),
            );
        }
    };
    if let Some(blur) = blur {
        r.group(cherenkov::Group::new().filter(blur), moving);
    } else if isolated {
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
}

#[test]
fn partial_clear_preserves_srgb_encoded_background() -> Result<(), Box<dyn std::error::Error>> {
    use cherenkov::{BlendSpace, Color, Srgb};

    let diagnostics = super::super::composition::Sink::new();
    let engine = Engine::<Gpu>::new(GpuConfig {
        composition_diag: Some(diagnostics.clone()),
        ..GpuConfig::default()
    })?;
    let size = (64, 64);
    for background in [
        Color::<Srgb>::new([254.0 / 255.0, 247.0 / 255.0, 1.0, 1.0]).to_working(),
        WorkingColor::new([0.2, 0.3, 0.4, 0.5]),
    ] {
        let (target, _textures) = TextureTarget::new(size);
        let retained = engine.surface(target, || {})?;
        retained.storage_space(BlendSpace::SrgbEncoded);
        retained.clear_color(background);
        retained.update(|tx| {
            tx[retained.root()].content(retained.record(|r| {
                r.fill(Rect::new(8.0, 8.0, 20.0, 20.0), WorkingColor::BLACK);
            }));
        });
        engine.render(FrameTime::now())?;
        let _ = diagnostics.take();

        // Removing the rect damages only its region and triggers partial clear.
        let (ref_target, _ref_textures) = TextureTarget::new(size);
        let reference = engine.surface(ref_target, || {})?;
        reference.storage_space(BlendSpace::SrgbEncoded);
        reference.clear_color(background);
        retained.update(|tx| {
            tx[retained.root()].content(retained.record(|_| {}));
        });
        engine.render(FrameTime::now())?;

        let reports = diagnostics.take();
        let report = reports
            .iter()
            .find(|r| r.surface == retained.id().raw())
            .expect("partial repaint report");
        assert!(
            report.repaint_pixels > 0
                && report.repaint_pixels < u64::from(size.0) * u64::from(size.1),
            "expected partial repaint, got {} pixels",
            report.repaint_pixels
        );

        let actual = retained.readback()?;
        let expected = reference.readback()?;
        let repaired = actual.pixels[14 * size.0 as usize + 14];
        let untouched = actual.pixels[2 * size.0 as usize + 2];
        for (a, b) in repaired.into_iter().zip(untouched) {
            assert!(
                (a - b).abs() < 1e-4,
                "partial clear {repaired:?} != full clear {untouched:?}"
            );
        }
        let actual = cherenkov_oracle::F32Image {
            width: actual.width,
            height: actual.height,
            pixels: actual.pixels,
        };
        let expected = cherenkov_oracle::F32Image {
            width: expected.width,
            height: expected.height,
            pixels: expected.pixels,
        };
        cherenkov_oracle::metrics::assert_images_match(&expected, &actual);
    }
    Ok(())
}

fn paint_backdrop_bars_top(r: &mut Recorder, button_color: WorkingColor) {
    r.fill(
        Rect::new(120.0, 16.0, 232.0, 44.0),
        WorkingColor::new([0.95, 0.95, 0.98, 1.0]),
    );
    r.fill(Rect::new(8.0, 12.0, 48.0, 52.0), button_color);
}

struct BackdropBarsSurface {
    surface: Surface<Gpu>,
    _textures: Receiver<wgpu::Texture>,
    _backdrops: [BackdropGroup; 2],
    top: Layer,
}

fn setup_backdrop_bars_surface(
    engine: &Engine<Gpu>,
    size: (u32, u32),
    background: WorkingColor,
    scale: CaptureScale,
    button_color: WorkingColor,
) -> Result<BackdropBarsSurface, Box<dyn std::error::Error>> {
    use cherenkov::Group;
    use filtrate::filters::GaussianBlur;

    let (target, textures) = TextureTarget::new(size);
    let surface = engine.surface(target, || {})?;
    surface.clear_color(background);
    let top_backdrop = surface.backdrop_group(GaussianBlur::new(4.0_f32), scale);
    let bottom_backdrop = surface.backdrop_group(GaussianBlur::new(4.0_f32), scale);
    let bg = surface.layer();
    let top = surface.layer();
    let bottom = surface.layer();
    surface.update(|tx| {
        tx[&bg].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 256.0, 192.0),
                WorkingColor::new([0.12, 0.24, 0.42, 1.0]),
            );
            r.fill(
                Rect::new(20.0, 16.0, 236.0, 48.0),
                WorkingColor::new([0.9, 0.4, 0.2, 1.0]),
            );
            r.fill(
                Rect::new(24.0, 146.0, 232.0, 178.0),
                WorkingColor::new([0.2, 0.75, 0.85, 1.0]),
            );
        }));
        tx[&top]
            .clip(Rect::new(0.0, 0.0, 256.0, 64.0))
            .backdrop(top_backdrop.sample())
            .content(surface.record(|r| paint_backdrop_bars_top(r, button_color)));
        tx[&bottom]
            .clip(Rect::new(0.0, 136.0, 256.0, 192.0))
            .backdrop(bottom_backdrop.sample())
            .content(surface.record(|r| {
                r.group(Group::new().opacity(0.95), |r| {
                    r.fill(
                        Rect::new(16.0, 140.0, 240.0, 184.0),
                        WorkingColor::new([0.2, 0.65, 0.35, 1.0]),
                    );
                    r.fill(
                        Rect::new(92.0, 148.0, 164.0, 176.0),
                        WorkingColor::new([0.95, 0.85, 0.2, 1.0]),
                    );
                });
            }));
        tx[surface.root()].push(&bg).push(&top).push(&bottom);
    });
    Ok(BackdropBarsSurface {
        surface,
        _textures: textures,
        _backdrops: [top_backdrop, bottom_backdrop],
        top,
    })
}

#[test]
fn partial_repaint_preserves_siblings_when_backdrop_filter_expands_earlier_pass()
-> Result<(), Box<dyn std::error::Error>> {
    use cherenkov_oracle::{F32Image, metrics::assert_images_match};

    use super::super::composition::Sink;

    let diagnostics = Sink::new();
    let engine = Engine::<Gpu>::new(GpuConfig {
        composition_diag: Some(diagnostics.clone()),
        ..GpuConfig::default()
    })?;
    let size = (256, 192);
    let background = WorkingColor::new([0.05, 0.08, 0.14, 1.0]);
    let idle = WorkingColor::new([0.2, 0.5, 0.9, 1.0]);
    let pressed = WorkingColor::new([0.95, 0.25, 0.55, 1.0]);

    for (scale, expect_partial) in [(CaptureScale::FULL, true), (CaptureScale::new(0.5)?, false)] {
        let retained = setup_backdrop_bars_surface(&engine, size, background, scale, idle)?;
        engine.render(FrameTime::now())?;
        let _ = diagnostics.take();

        // Frame 1: change ONLY the small top-left button inside `top`.
        retained.surface.update(|tx| {
            tx[&retained.top].content(
                retained
                    .surface
                    .record(|r| paint_backdrop_bars_top(r, pressed)),
            );
        });
        engine.render(FrameTime::now())?;

        if expect_partial {
            let reports = diagnostics.take();
            let report = reports
                .iter()
                .find(|r| r.surface == retained.surface.id().raw())
                .expect("partial repaint report");
            assert!(
                report.repaint_pixels > 0
                    && report.repaint_pixels < u64::from(size.0) * u64::from(size.1),
                "expected partial repaint for full-scale clipped backdrop, got {} pixels",
                report.repaint_pixels
            );
        } else {
            let _ = diagnostics.take();
        }

        let reference = setup_backdrop_bars_surface(&engine, size, background, scale, pressed)?;
        engine.render(FrameTime::now())?;
        let _ = diagnostics.take();

        let actual = retained.surface.readback()?;
        let expected = reference.surface.readback()?;
        assert_images_match(
            &F32Image {
                width: expected.width,
                height: expected.height,
                pixels: expected.pixels,
            },
            &F32Image {
                width: actual.width,
                height: actual.height,
                pixels: actual.pixels,
            },
        );
    }
    Ok(())
}
