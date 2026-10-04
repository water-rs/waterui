//! The measured scenes for `canvas-bench` (water-rs/waterui#1564).
//!
//! One cell = one (scenario, variant, n) triple. The scene is built
//! identically across variants — the realization, never the scene, is
//! what a variant changes. Mounting a cell registers its `SceneHandles`
//! (scroll drive, animation switch, work counters) through the slot the
//! matrix harness handed in.
//!
//! Scenario 1 (many small shapes): a lazily mounted scrolling list of `N`
//! rows; every row carries a rounded-rect background and a gradient chip.
//! Variant `a` uses the `ResolvedShape`/`Gradient` leaves
//! (`CAShapeLayer`/`CAGradientLayer`), variant `b` draws each element
//! through a per-instance `SceneView`. The lazy stack rebuilds rows as
//! the fling brings them into view — the `rows_created` counter runs on
//! every rebuild, which is the measurement's work signal.
//!
//! Scenario 3 (many `GpuContentView`s): `N` trivial clear-pass producers
//! on the `gpu_surface` leaf, animating every frame; each `render` call
//! bumps `producer_calls`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use waterui::graphics::cherenkov::{
    Draw, Fixed, Paint, Recorder, WorkingColor, kurbo,
};
use waterui::graphics::scene_view::{SceneContent, SceneView};
use waterui::graphics::{Context, Frame, GpuContent, GpuContentView};
use waterui::id::IdentifiableExt;
use waterui::layout::scroll::{ScrollController, scroll};
use waterui::layout::{LazyContainer, stack::VStackLayout};
use waterui::views::ForEach;
use waterui::prelude::*;
use waterui::shape::{Capsule, RoundedRectangle, ShapeExt};

use crate::harness::{CellSpec, SceneHandles, ScrollDriver};

/// Builds a cell's scene, registering its handles into `slot` before the
/// view mounts — the matrix harness waits on this registration.
#[must_use]
pub fn cell_view(spec: &CellSpec, slot: &Rc<RefCell<Option<SceneHandles>>>) -> AnyView {
    let handles = SceneHandles {
        scroll: if spec.scenario == 1 {
            ScrollDriver::Controller(ScrollController::new(waterui::layout::Point::new(
                0.0, 0.0,
            )))
        } else {
            ScrollDriver::None
        },
        animate: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        rows_created: Arc::new(AtomicU64::new(0)),
        producer_calls: Arc::new(AtomicU64::new(0)),
    };
    *slot.borrow_mut() = Some(handles.clone());
    match spec.scenario {
        1 => {
            let ScrollDriver::Controller(controller) = &handles.scroll else {
                unreachable!("scenario 1 always wires a scroll controller")
            };
            AnyView::new(scenario1(spec, controller, &handles.rows_created))
        }
        3 => AnyView::new(scenario3(spec, &handles)),
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

/// Scenario 1: `N` rows in a lazily mounted vertical stack — rows build
/// on demand as the fling reveals them, each build counted.
fn scenario1(
    spec: &CellSpec,
    controller: &ScrollController<waterui::layout::Point>,
    rows_created: &Arc<AtomicU64>,
) -> impl View {
    let variant = spec.variant.clone();
    let rows_created = rows_created.clone();
    let items: Vec<_> = (0..spec.n).map(|index| index.self_id()).collect();
    scroll(LazyContainer::new(
        VStackLayout::default(),
        ForEach::new(items, move |item| {
            rows_created.fetch_add(1, Ordering::Relaxed);
            AnyView::new(row(&variant, *item))
        }),
    ))
    .scroll_controller(controller)
}

/// One trivial GPU producer: an animated clear colour while `animate`
/// is set, `producer_calls` counting every render.
struct TrivialProducer {
    seed: f64,
    animate: Arc<std::sync::atomic::AtomicBool>,
    calls: Arc<AtomicU64>,
}

impl GpuContent for TrivialProducer {
    fn setup(&mut self, _gpu: &Context<'_>) {}

    fn render(&mut self, frame: &mut Frame<'_>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
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
fn scenario3(spec: &CellSpec, handles: &SceneHandles) -> impl View {
    let columns = (spec.n as f64).sqrt().ceil().max(1.0) as usize;
    let indices: Vec<u32> = (0..spec.n).collect();
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
                                animate: handles.animate.clone(),
                                calls: handles.producer_calls.clone(),
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