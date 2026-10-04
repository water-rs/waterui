//! The measured scenes for `canvas-bench` (water-rs/waterui#1564).
//!
//! Each scenario is selected at launch through `WATERUI_BENCH_*` and built
//! identically across its variants — the realization, never the scene, is
//! what a variant changes.
//!
//! Scenario 1 (many small shapes): a scrolling list of `N` rows; every row
//! carries a rounded-rect background and a gradient chip. Variant `a` uses
//! the `ResolvedShape`/`Gradient` leaves (`CAShapeLayer`/`CAGradientLayer`),
//! variant `b` draws each element through a per-instance `SceneView`.
//!
//! Scenario 3 (many `GpuContentView`s): `N` trivial clear-pass producers on
//! the `gpu_surface` leaf, animating every frame for the animated window
//! and then idle.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use waterui::graphics::cherenkov::{
    Draw, Fixed, Paint, Recorder, WorkingColor, kurbo,
};
use waterui::graphics::scene_view::{SceneContent, SceneView};
use waterui::graphics::{Context, Frame, GpuContent, GpuContentView};
use waterui::layout::scroll::{ScrollController, scroll};
use waterui::prelude::*;
use waterui::shape::{Capsule, RoundedRectangle, ShapeExt};

use crate::harness::{BenchConfig, SceneHandles, ScrollDriver};

/// The scene-side handles (scroll drive, animation switch) for `config`'s
/// scenario, built once per launch.
#[must_use]
pub fn handles(config: &BenchConfig) -> SceneHandles {
    match config.scenario {
        1 => SceneHandles {
            scroll: ScrollDriver::Controller(ScrollController::new(waterui::layout::Point::new(
                0.0, 0.0,
            ))),
            animate: Arc::new(AtomicBool::new(false)),
        },
        3 => SceneHandles {
            scroll: ScrollDriver::None,
            animate: Arc::new(AtomicBool::new(false)),
        },
        other => panic!("unsupported scenario {other}"),
    }
}

/// The root view for `config`'s scenario; built fresh on every call.
#[must_use]
pub fn view(config: &BenchConfig, handles: &SceneHandles) -> AnyView {
    match config.scenario {
        1 => {
            let ScrollDriver::Controller(controller) = &handles.scroll else {
                unreachable!("scenario 1 always wires a scroll controller")
            };
            AnyView::new(scenario1(config, controller))
        }
        3 => AnyView::new(scenario3(config, &handles.animate)),
        other => panic!("unsupported scenario {other}"),
    }
}

/// Deterministic per-row palette, identical across variants.
fn row_color(index: u32) -> (u8, u8, u8) {
    let h = index.wrapping_mul(2654435761) >> 24;
    (
        48 + (h % 160) as u8,
        48 + ((h / 7) % 160) as u8,
        48 + ((h / 13) % 160) as u8,
    )
}

/// The gradient chip's two stops.
fn chip_colors(index: u32) -> (WorkingColor, WorkingColor) {
    let (r, g, b) = row_color(index + 1000);
    let a = WorkingColor::new([
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        1.0,
    ]);
    (a, WorkingColor::new([a.components[2], a.components[0], a.components[1], 1.0]))
}

/// A `SceneContent` that fills its bounds with a rounded rect or a
/// horizontal linear gradient — the `SceneView` half of scenario 1's
/// variant `b`.
struct FillContent {
    kind: FillKind,
}

enum FillKind {
    Rounded {
        color: WorkingColor,
        radius: f64,
    },
    Linear {
        start: WorkingColor,
        end: WorkingColor,
    },
}

impl SceneContent for FillContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut waterui::graphics::resources::RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> bool {
        match self.kind {
            FillKind::Rounded { color, radius } => {
                recorder.fill(
                    Fixed(kurbo::RoundedRect::new(
                        0.0,
                        0.0,
                        f64::from(width),
                        f64::from(height),
                        radius,
                    )),
                    Fixed(color),
                );
            }
            FillKind::Linear { start, end } => {
                let gradient = waterui::graphics::cherenkov::LinearGradient::new(
                    kurbo::Point::new(0.0, f64::from(height) / 2.0),
                    kurbo::Point::new(f64::from(width), f64::from(height) / 2.0),
                )
                .stop(0.0, start)
                .stop(1.0, end);
                recorder.fill(
                    Fixed(kurbo::Rect::new(
                        0.0,
                        0.0,
                        f64::from(width),
                        f64::from(height),
                    )),
                    Fixed(Paint::Linear(gradient)),
                );
            }
        }
        false
    }

    fn rebuild_for_engine(&mut self) {}
}

/// A scenario-1 row: rounded background, label, gradient chip — the two
/// decorative elements going through the variant's realization.
fn row(variant: &str, index: u32) -> impl View {
    let (r, g, b) = row_color(index);
    let (c0, c1) = chip_colors(index);
    let fill = Color::srgb(r, g, b);

    let background: AnyView = if variant == "b" {
        AnyView::new(
            SceneView::new(FillContent {
                kind: FillKind::Rounded {
                    color: WorkingColor::new([
                        f32::from(r) / 255.0,
                        f32::from(g) / 255.0,
                        f32::from(b) / 255.0,
                        1.0,
                    ]),
                    radius: 12.0,
                },
            })
            .size(340.0, 52.0),
        )
    } else {
        AnyView::new(
            RoundedRectangle::new(0.24)
                .fill(fill)
                .size(340.0, 52.0),
        )
    };

    let chip: AnyView = if variant == "b" {
        AnyView::new(
            SceneView::new(FillContent {
                kind: FillKind::Linear { start: c0, end: c1 },
            })
            .size(72.0, 24.0)
            .clip(Capsule),
        )
    } else {
        AnyView::new(
            Gradient::linear(vec![(0.0, c0), (1.0, c1)], [0.0, 0.5], [1.0, 0.5])
                .size(72.0, 24.0)
                .clip(Capsule),
        )
    };

    zstack((
        background,
        hstack((text!("Row {index}", index = index), spacer(), chip)).padding_with(10.0),
    ))
    .padding_with(4.0)
}

/// Scenario 1: `N` rows in a vertically scrolling stack.
fn scenario1(config: &BenchConfig, controller: &ScrollController<waterui::layout::Point>) -> impl View {
    let rows: Vec<AnyView> = (0..config.n)
        .map(|index| AnyView::new(row(&config.variant, index)))
        .collect();
    scroll(vstack(rows)).scroll_controller(controller)
}

/// One trivial GPU producer: an animated clear colour while `animate` is
/// set, nothing after.
struct TrivialProducer {
    seed: f64,
    animate: Arc<AtomicBool>,
}

impl GpuContent for TrivialProducer {
    fn setup(&mut self, _gpu: &Context<'_>) {}

    fn render(&mut self, frame: &mut Frame<'_>) {
        let phase = self.seed + frame.elapsed.as_secs_f64();
        let color = waterui::graphics::wgpu::Color {
            r: (phase * 0.7).sin() * 0.5 + 0.5,
            g: (phase * 1.1).sin() * 0.5 + 0.5,
            b: (phase * 1.3).cos() * 0.5 + 0.5,
            a: 1.0,
        };
        let mut encoder = frame
            .device
            .create_command_encoder(&waterui::graphics::wgpu::CommandEncoderDescriptor {
                label: Some("canvas-bench trivial producer"),
            });
        {
            let _pass = encoder.begin_render_pass(&waterui::graphics::wgpu::RenderPassDescriptor {
                label: Some("canvas-bench trivial pass"),
                color_attachments: &[Some(waterui::graphics::wgpu::RenderPassColorAttachment {
                    view: frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: waterui::graphics::wgpu::Operations {
                        load: waterui::graphics::wgpu::LoadOp::Clear(color),
                        store: waterui::graphics::wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        frame.queue.submit([encoder.finish()]);
        if self.animate.load(Ordering::Relaxed) {
            frame.request_redraw();
        }
    }

    fn intrinsic_size(&self) -> Option<waterui::layout::Size> {
        Some(waterui::layout::Size::new(64.0, 64.0))
    }

    fn is_opaque(&self) -> bool {
        true
    }
}

/// Scenario 3: `N` `GpuContentView`s in a grid inside a scroll area.
fn scenario3(config: &BenchConfig, animate: &Arc<AtomicBool>) -> impl View {
    let columns = (config.n as f64).sqrt().ceil().max(1.0) as usize;
    let indices: Vec<u32> = (0..config.n).collect();
    let rows: Vec<AnyView> = indices
        .chunks(columns)
        .map(|chunk| {
            AnyView::new(hstack(
                chunk
                    .iter()
                    .map(|&index| {
                        AnyView::new(
                            GpuContentView::new(TrivialProducer {
                                seed: f64::from(index) * 0.37,
                                animate: animate.clone(),
                            })
                            .labeled(format!("gpu-{index}")),
                        )
                    })
                    .collect::<Vec<_>>(),
            ))
        })
        .collect();
    scroll(vstack(rows))
}