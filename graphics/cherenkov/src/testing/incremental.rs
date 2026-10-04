//! Exact incremental/full lowering checks shared by the CPU and GPU backends.

use crate::Instant;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use crate::kurbo::{Affine, BezPath, Circle, Rect, Stroke, Vec2};
use crate::message::LayerOp;
use crate::{
    Animation, Command, ContentOp, Curve, Display, Draw, FontData, FontId, Frame, FrameId,
    FrameStats, FrameTime, Glyph, GlyphRun, GlyphStyle, Group, LayerId, LinearGradient, Offscreen,
    OffscreenFormat, Operand, Paint, Picture, Pressure, Prop, Renderer, Shadow, ShapeData,
    SlotUpdate, SurfaceFrame, SurfaceId, SurfaceTree, WorkingColor,
};

/// Runs deterministic randomized slot changes against fresh full lowering.
/// Backend initialization must succeed: a missing GPU must fail this test.
///
/// # Panics
/// If a backend fails, output differs, or unrelated commands are re-lowered.
#[cfg(not(target_arch = "wasm32"))]
pub fn equivalence<R: Renderer>(renderer: &mut R)
where
    R::Target: From<Offscreen>,
{
    let font = register_font(renderer);
    let mut list = fixture(font).display_list().clone();
    let stable = Picture::record(|c| c.fill(Rect::new(1., 1., 4., 4.), WorkingColor::WHITE));
    let mut tree = SurfaceTree::new();
    let layer = LayerId::new(1);
    let sibling = LayerId::new(2);
    tree.apply(LayerOp::Create(layer));
    tree.apply(LayerOp::Create(sibling));
    tree.apply(LayerOp::Push {
        parent: tree.root(),
        child: layer,
    });
    tree.apply(LayerOp::Push {
        parent: tree.root(),
        child: sibling,
    });
    let ids = [SurfaceId::new(1), SurfaceId::new(2)];
    let mut size = (96, 96);
    for id in ids {
        renderer
            .create_surface(
                id,
                Offscreen::new(size, OffscreenFormat::LinearF16).into(),
                unhosted_waker(),
            )
            .expect("surface");
        let _ = renderer.set_content(
            id,
            layer,
            Some(ContentOp::Replace(crate::Picture::from_list(list.clone()))),
        );
        let _ = renderer.set_content(id, sibling, Some(ContentOp::Picture(stable.clone())));
    }
    let start = Instant::now();
    let mut frames = Frames::default();
    for step in 0..160u32 {
        let mut dirty_count = 0;
        if step > 0 && step % 8 < 5 {
            // Two separate commits exercise Dirty::union before one render.
            for batch in 0..2 {
                let updates = updates(&list, step * 2 + batch);
                let dirty = list.apply(updates.clone());
                dirty_count += dirty.ranges().iter().map(|r| r.end - r.start).sum::<u32>();
                let _ = renderer.set_content(ids[0], layer, Some(ContentOp::Update(updates)));
            }
        }
        update_properties(&mut tree, layer, step);
        if step == 71 {
            size = (104, 100);
            for id in ids {
                renderer.resize_surface(id, size);
            }
        }
        if step == 93 {
            renderer.trim(Pressure::Critical);
        }
        let time = start + Duration::from_millis(u64::from(step) * 16);
        let _ = tree.sample(time, Display::default());
        let _ = renderer.set_content(
            ids[1],
            layer,
            Some(ContentOp::Replace(crate::Picture::from_list(list.clone()))),
        );
        let incremental = render(renderer, &mut frames, ids[0], &tree, size, time);
        let full = render(renderer, &mut frames, ids[1], &tree, size, time);
        if step > 0 {
            if dirty_count == 0 {
                assert_eq!(
                    incremental.commands_lowered, 0,
                    "properties re-lowered content at {step}"
                );
            } else {
                assert!(
                    incremental.commands_lowered <= u32::try_from(list.len()).unwrap(),
                    "rebuilt an unrelated layer at {step}"
                );
            }
            assert_eq!(full.commands_lowered, u32::try_from(list.len()).unwrap());
        }
        let a = renderer.readback(ids[0]).expect("incremental readback");
        let b = renderer.readback(ids[1]).expect("full readback");
        for (pixel, (a, b)) in a.pixels.iter().zip(&b.pixels).enumerate() {
            assert_eq!(
                a.map(f32::to_bits),
                b.map(f32::to_bits),
                "frame {step}, pixel {pixel}"
            );
        }
    }
    assert_patch_counts(
        renderer,
        &mut frames,
        ids[0],
        (layer, &tree, &list),
        size,
        start + Duration::from_secs(4),
    );
    for id in ids {
        renderer.destroy_surface(id);
    }
    renderer.remove_font(font);
}

#[cfg(target_arch = "wasm32")]
/// Browser version of the same deterministic incremental/full comparison.
///
/// # Panics
/// If rendering fails, pixels differ, or unchanged commands are rebuilt.
#[expect(
    clippy::too_many_lines,
    reason = "the browser twin of the native equivalence routine above; keeping them line-parallel outweighs the limit"
)]
pub async fn equivalence<R: Renderer>(renderer: &mut R)
where
    R::Target: From<Offscreen>,
{
    let font = register_font(renderer);
    let mut list = fixture(font).display_list().clone();
    let stable = Picture::record(|c| c.fill(Rect::new(1., 1., 4., 4.), WorkingColor::WHITE));
    let mut tree = SurfaceTree::new();
    let layer = LayerId::new(1);
    let sibling = LayerId::new(2);
    tree.apply(LayerOp::Create(layer));
    tree.apply(LayerOp::Create(sibling));
    tree.apply(LayerOp::Push {
        parent: tree.root(),
        child: layer,
    });
    tree.apply(LayerOp::Push {
        parent: tree.root(),
        child: sibling,
    });
    let ids = [SurfaceId::new(1), SurfaceId::new(2)];
    let mut size = (96, 96);
    for id in ids {
        renderer
            .create_surface(
                id,
                Offscreen::new(size, OffscreenFormat::LinearF16).into(),
                unhosted_waker(),
            )
            .expect("surface");
        let _ = renderer.set_content(
            id,
            layer,
            Some(ContentOp::Replace(crate::Picture::from_list(list.clone()))),
        );
        let _ = renderer.set_content(id, sibling, Some(ContentOp::Picture(stable.clone())));
    }
    let start = Instant::now();
    let mut frames = Frames::default();
    for step in 0..160u32 {
        let mut dirty_count = 0;
        if step > 0 && step % 8 < 5 {
            // Two separate commits exercise Dirty::union before one render.
            for batch in 0..2 {
                let updates = updates(&list, step * 2 + batch);
                let dirty = list.apply(updates.clone());
                dirty_count += dirty.ranges().iter().map(|r| r.end - r.start).sum::<u32>();
                let _ = renderer.set_content(ids[0], layer, Some(ContentOp::Update(updates)));
            }
        }
        update_properties(&mut tree, layer, step);
        if step == 71 {
            size = (104, 100);
            for id in ids {
                renderer.resize_surface(id, size);
            }
        }
        if step == 93 {
            renderer.trim(Pressure::Critical);
        }
        let time = start + Duration::from_millis(u64::from(step) * 16);
        let _ = tree.sample(time, Display::default());
        let _ = renderer.set_content(
            ids[1],
            layer,
            Some(ContentOp::Replace(crate::Picture::from_list(list.clone()))),
        );
        let incremental = render(renderer, &mut frames, ids[0], &tree, size, time).await;
        let full = render(renderer, &mut frames, ids[1], &tree, size, time).await;
        if step > 0 {
            if dirty_count == 0 {
                assert_eq!(
                    incremental.commands_lowered, 0,
                    "properties re-lowered content at {step}"
                );
            } else {
                assert!(
                    incremental.commands_lowered <= u32::try_from(list.len()).unwrap(),
                    "rebuilt an unrelated layer at {step}"
                );
            }
            assert_eq!(full.commands_lowered, u32::try_from(list.len()).unwrap());
        }
        let a = renderer
            .readback(ids[0])
            .await
            .expect("incremental readback");
        let b = renderer.readback(ids[1]).await.expect("full readback");
        for (pixel, (a, b)) in a.pixels.iter().zip(&b.pixels).enumerate() {
            assert_eq!(
                a.map(f32::to_bits),
                b.map(f32::to_bits),
                "frame {step}, pixel {pixel}"
            );
        }
    }
    assert_patch_counts(
        renderer,
        &mut frames,
        ids[0],
        (layer, &tree, &list),
        size,
        start + Duration::from_secs(4),
    )
    .await;
    for id in ids {
        renderer.destroy_surface(id);
    }
    renderer.remove_font(font);
}

fn register_font<R: Renderer>(renderer: &mut R) -> FontId {
    let font = FontId::new(1);
    // The scene tree is generated, not committed: read it at call time so
    // the `testing` feature compiles for consumers pinned by git, and only
    // `equivalence` needs the generated fonts.
    let data = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes/fonts/NotoSans.ttf"),
    )
    .expect("scenes/fonts/NotoSans.ttf is generated; run `python3 scenes/tools/generate.py` first");
    renderer.add_font(
        font,
        R::prepare_font(FontData {
            data: data.into(),
            index: 0,
        })
        .expect("prepare font"),
    );
    font
}

/// Numbers the renders a harness drives directly, as the engine's render
/// loop numbers its own.
#[derive(Default)]
struct Frames(u64);

impl Frames {
    const fn next(&mut self) -> FrameId {
        let id = FrameId::new(self.0);
        self.0 += 1;
        id
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn render<R: Renderer>(
    renderer: &mut R,
    frames: &mut Frames,
    id: SurfaceId,
    tree: &SurfaceTree,
    size: (u32, u32),
    time: Instant,
) -> FrameStats {
    let mut stats = FrameStats::default();
    renderer
        .render(
            &Frame {
                id: frames.next(),
                time: FrameTime::at(time),
                surfaces: &[SurfaceFrame {
                    id,
                    size,
                    display: Display::default(),
                    clear: WorkingColor::new([0.1, 0.2, 0.3, 1.]),
                    changed: true,
                    plane_frames: None,
                    present_pending: false,
                    display_moved: false,
                    tree,
                }],
            },
            &mut stats,
        )
        .expect("render");
    stats
}

#[cfg(target_arch = "wasm32")]
async fn render<R: Renderer>(
    renderer: &mut R,
    frames: &mut Frames,
    id: SurfaceId,
    tree: &SurfaceTree,
    size: (u32, u32),
    time: Instant,
) -> FrameStats {
    let mut stats = FrameStats::default();
    renderer
        .render(
            &Frame {
                id: frames.next(),
                time: FrameTime::at(time),
                surfaces: &[SurfaceFrame {
                    id,
                    size,
                    display: Display::default(),
                    clear: WorkingColor::new([0.1, 0.2, 0.3, 1.]),
                    changed: true,
                    plane_frames: None,
                    present_pending: false,
                    display_moved: false,
                    tree,
                }],
            },
            &mut stats,
        )
        .await
        .expect("render");
    stats
}

fn fixture(font: FontId) -> Picture {
    let nested = Picture::record(|c| {
        c.group(Group::new().opacity(0.7), |c| {
            c.fill(Circle::new((24., 24.), 13.), WorkingColor::WHITE);
            c.fill(
                Rect::new(10., 10., 35., 35.),
                WorkingColor::new([0.2, 0.7, 0.3, 0.8]),
            );
        });
    });
    Picture::record(|c| {
        for i in 0..24u32 {
            let x = f64::from(i % 6) * 12.;
            let y = f64::from(i / 6) * 16.;
            c.fill(
                Rect::new(x, y, x + 8., y + 10.),
                WorkingColor::new([0.2, 0.3, 0.5, 0.6]),
            );
        }
        c.transform(Affine::translate((2.25, 3.5)), |c| {
            c.clip(Rect::new(1., 1., 80., 85.), |c| {
                c.group(Group::new().opacity(0.8), |c| {
                    c.fill(path(0.4), gradient(0.3));
                    c.stroke(path(0.6), Stroke::new(1.5), WorkingColor::WHITE);
                    c.glyphs(run(font, 4, 0.5), gradient(0.8));
                    c.shadow(
                        Rect::new(20., 30., 40., 50.),
                        Shadow::new(2., WorkingColor::new([0., 0., 0., 0.4])),
                    );
                    // Exercise shadow occlusion-key reuse and invalidation when
                    // the following opaque shape is patched or repainted.
                    c.fill(Rect::new(20., 30., 40., 50.), WorkingColor::WHITE);
                    c.picture(&nested, Affine::translate((35., 30.)));
                });
            });
        });
        c.fill(Rect::new(85., 85., 90., 90.), WorkingColor::WHITE);
    })
}

// Hashing the frame/index pair supplies reproducible pseudo-random operands
// without platform RNG state or a custom random-number generator.
fn random(frame: u32, lane: u32) -> f64 {
    let mut hash = DefaultHasher::new();
    (0x31_d17eu64, frame, lane).hash(&mut hash);
    f64::from(u32::try_from(hash.finish() & 0xffff).unwrap()) / 65535.
}

fn path(value: f64) -> BezPath {
    let mut path = BezPath::new();
    path.move_to((9., 13.));
    path.curve_to((20., value.mul_add(30., 5.)), (45., 72.), (62., 19.));
    path.line_to((70., 65.));
    path.close_path();
    path
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "bounded test colour and glyph coordinates"
)]
fn gradient(value: f64) -> Paint {
    let mut gradient = LinearGradient::new((2., 4.), (72., 80.));
    gradient.stops = vec![
        crate::ColorStop {
            offset: 0.,
            color: WorkingColor::new([value as f32, 0.2, 0.6, 0.8]),
        },
        crate::ColorStop {
            offset: 1.,
            color: WorkingColor::new([0.8, value as f32, 0.2, 0.5]),
        },
    ];
    Paint::Linear(gradient)
}

fn run(font: FontId, count: u32, value: f64) -> GlyphRun {
    GlyphRun {
        font,
        size: 14.,
        coords: Vec::new().into(),
        style: GlyphStyle::Fill,
        glyphs: (0..count)
            .map(|i| Glyph {
                id: 36 + i % 8,
                x: f32::from(u16::try_from(i).unwrap()).mul_add(12., 10.),
                y: 65. + if value > 0.5 { 0.25 } else { 0.75 },
                transform: None,
            })
            .collect::<Vec<_>>()
            .into(),
    }
}

fn updates(list: &crate::DisplayList, frame: u32) -> Vec<SlotUpdate> {
    list.commands()
        .iter()
        .enumerate()
        .filter_map(|(i, command)| {
            let index = u32::try_from(i).unwrap();
            let value = random(frame, index);
            if value < 0.88 {
                return None;
            }
            let value = (value - 0.88) / 0.12;
            let operand = match command {
                Command::Fill {
                    shape: ShapeData::Path { .. },
                    ..
                }
                | Command::BeginClip { .. } => Operand::Shape(ShapeData::of(&path(value))),
                Command::Fill { .. } => {
                    if frame.is_multiple_of(2) {
                        Operand::Paint(gradient(value))
                    } else {
                        Operand::Shape(ShapeData::Rect(Rect::new(
                            12.,
                            15.,
                            value.mul_add(50., 20.),
                            value.mul_add(35., 28.),
                        )))
                    }
                }
                Command::Stroke { .. } => Operand::Stroke(Stroke::new(value.mul_add(3., 0.5))),
                Command::Glyphs { run: old, .. } => Operand::Run(run(old.font, frame % 6, value)),
                Command::Shadow { .. } => Operand::Shadow(Shadow::new(
                    value.mul_add(3., 1.),
                    WorkingColor::new([0.1, 0.2, 0.3, 0.5]),
                )),
                Command::Picture { .. } | Command::BeginTransform { .. } => {
                    Operand::Transform(Affine::translate((value * 5., value * 3.)))
                }
                Command::BeginGroup { .. } => {
                    Operand::Group(Group::new().opacity(if value > 0.5 { 1. } else { 0.6 }))
                }
                Command::Image { .. } | Command::End => return None,
            };
            Some(SlotUpdate {
                command: index,
                value: operand,
            })
        })
        .collect()
}

fn update_properties(tree: &mut SurfaceTree, layer: LayerId, step: u32) {
    if step % 8 == 5 {
        tree.apply(LayerOp::Transform(
            layer,
            Prop {
                target: Affine::translate((random(step, 9) * 8., random(step, 10) * 8.))
                    * Affine::scale_non_uniform(
                        random(step, 11).mul_add(0.4, 0.8),
                        random(step, 12).mul_add(0.4, 0.8),
                    ),
                animation: Some(Animation::Curve(Curve::linear(Duration::from_millis(40)))),
            },
        ));
    }
    if step % 8 == 6 {
        tree.apply(LayerOp::Opacity(
            layer,
            Prop {
                target: if step % 16 == 6 { 0.6 } else { 1. },
                animation: Some(Animation::Curve(Curve::linear(Duration::from_millis(40)))),
            },
        ));
    }
    if step % 19 == 7 {
        tree.apply(LayerOp::Clip(
            layer,
            Some(ShapeData::Rect(Rect::new(3.25, 2.5, 88., 90.))),
        ));
        tree.apply(LayerOp::ScrollOffset(
            layer,
            Prop {
                target: Vec2::new(1., 2.),
                animation: None,
            },
        ));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn assert_patch_counts<R: Renderer>(
    renderer: &mut R,
    frames: &mut Frames,
    id: SurfaceId,
    scene: (LayerId, &SurfaceTree, &crate::DisplayList),
    size: (u32, u32),
    time: Instant,
) {
    let (layer, tree, list) = scene;
    // Two stable leaf updates must lower exactly two commands in one layer.
    let updates: Vec<_> = list
        .commands()
        .iter()
        .enumerate()
        .filter_map(|(i, c)| matches!(c, Command::Fill { .. }).then_some(u32::try_from(i).unwrap()))
        .take(2)
        .map(|command| SlotUpdate {
            command,
            value: Operand::Paint(Paint::Solid(WorkingColor::WHITE)),
        })
        .collect();
    let _ = renderer.set_content(id, layer, Some(ContentOp::Update(updates)));
    let stats = render(renderer, frames, id, tree, size, time);
    assert_eq!(
        stats.commands_lowered, 2,
        "two dirty fills must patch just two commands"
    );
    assert_eq!(
        stats.layers_composed, 1,
        "an unrelated layer was recomposed"
    );
    let unchanged = render(renderer, frames, id, tree, size, time);
    assert_eq!(unchanged.commands_lowered, 0);
    assert_eq!(
        unchanged.layers_composed, 0,
        "unchanged device output must be reused"
    );
    let (index, mut changed_run) = list
        .commands()
        .iter()
        .enumerate()
        .find_map(|(i, command)| {
            if let Command::Glyphs { run, .. } = command {
                Some((i, run.clone()))
            } else {
                None
            }
        })
        .expect("glyph fixture");
    let mut glyphs = changed_run.glyphs.to_vec();
    glyphs.push(Glyph {
        id: 36,
        x: 12.,
        y: 60.,
        transform: None,
    });
    changed_run.glyphs = glyphs.into();
    let _ = renderer.set_content(
        id,
        layer,
        Some(ContentOp::Update(vec![SlotUpdate {
            command: u32::try_from(index).unwrap(),
            value: Operand::Run(changed_run),
        }])),
    );
    let structural = render(renderer, frames, id, tree, size, time);
    assert_eq!(
        structural.commands_lowered,
        u32::try_from(list.len()).unwrap(),
        "a glyph-count change must rebuild only its layer"
    );
}

#[cfg(target_arch = "wasm32")]
async fn assert_patch_counts<R: Renderer>(
    renderer: &mut R,
    frames: &mut Frames,
    id: SurfaceId,
    scene: (LayerId, &SurfaceTree, &crate::DisplayList),
    size: (u32, u32),
    time: Instant,
) {
    let (layer, tree, list) = scene;
    // Two stable leaf updates must lower exactly two commands in one layer.
    let updates: Vec<_> = list
        .commands()
        .iter()
        .enumerate()
        .filter_map(|(i, c)| matches!(c, Command::Fill { .. }).then_some(u32::try_from(i).unwrap()))
        .take(2)
        .map(|command| SlotUpdate {
            command,
            value: Operand::Paint(Paint::Solid(WorkingColor::WHITE)),
        })
        .collect();
    let _ = renderer.set_content(id, layer, Some(ContentOp::Update(updates)));
    let stats = render(renderer, frames, id, tree, size, time).await;
    assert_eq!(
        stats.commands_lowered, 2,
        "two dirty fills must patch just two commands"
    );
    assert_eq!(
        stats.layers_composed, 1,
        "an unrelated layer was recomposed"
    );
    let unchanged = render(renderer, frames, id, tree, size, time).await;
    assert_eq!(unchanged.commands_lowered, 0);
    assert_eq!(
        unchanged.layers_composed, 0,
        "unchanged device output must be reused"
    );
    let (index, mut changed_run) = list
        .commands()
        .iter()
        .enumerate()
        .find_map(|(i, command)| {
            if let Command::Glyphs { run, .. } = command {
                Some((i, run.clone()))
            } else {
                None
            }
        })
        .expect("glyph fixture");
    let mut glyphs = changed_run.glyphs.to_vec();
    glyphs.push(Glyph {
        id: 36,
        x: 12.,
        y: 60.,
        transform: None,
    });
    changed_run.glyphs = glyphs.into();
    let _ = renderer.set_content(
        id,
        layer,
        Some(ContentOp::Update(vec![SlotUpdate {
            command: u32::try_from(index).unwrap(),
            value: Operand::Run(changed_run),
        }])),
    );
    let structural = render(renderer, frames, id, tree, size, time).await;
    assert_eq!(
        structural.commands_lowered,
        u32::try_from(list.len()).unwrap(),
        "a glyph-count change must rebuild only its layer"
    );
}

/// A surface wake-up with no host behind it: the harness drives the
/// renderer directly and renders on its own schedule.
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::arc_with_non_send_sync,
        reason = "the browser engine's waker is single-threaded; `Arc` matches the native type"
    )
)]
fn unhosted_waker() -> crate::CompletionWaker {
    let engine = std::sync::Arc::new(crate::engine::Waker::new());
    crate::CompletionWaker::new(&std::sync::Arc::new(crate::engine::SurfaceWaker::new(
        engine,
    )))
}
