//! A look's parts — a slider's, and a button chrome's custom parts — follow
//! the widget's value and interaction state in the rendered pixels, whether
//! they paint or record a backdrop material (water-rs/waterui#4503).

use core::time::Duration;
use std::cell::RefCell;
use std::time::Instant;

#[cfg(hydrolysis_gpu_painter)]
use cherenkov_record::MaterialEffect;
use cherenkov_record::{Draw as _, Paint, ShapeData, WorkingColor};
use kurbo::{Point, Rect, Size};
use waterui::animation::Animation;
use waterui::{Binding, Environment, ViewExt};
use waterui_controls::slider::slider;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::LayoutDirection;

#[cfg(hydrolysis_gpu_painter)]
use super::toggle_glass::{GLASS, GLASS_SHADER, plan};
use super::{MinimalTestTheme, pumped_test_environment};
use crate::num_cast::f64_as_f32;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::theme::{
    Custom, DragMapping, Driven, HitRegion, Inputs, Knob, Motion, SliderLayout, SliderLook,
    SliderSpec,
};
use crate::{HeadlessRuntime, HeadlessSnapshot};

const WIDTH: u32 = 400;
const HEIGHT: u32 = 100;
const POINTER: u64 = 3;
const KNOB: Size = Size::new(24.0, 24.0);

/// The knob's rect at rest for `inputs`: centred on the value's position
/// along the row.
fn knob_rect(inputs: &Inputs<'_, f64>) -> Rect {
    let frame = inputs.frame;
    let x = inputs.value.mul_add(frame.width(), frame.x0);
    Rect::from_center_size(Point::new(x, frame.center().y), KNOB)
}

fn look(drag: DragMapping) -> SliderLook {
    look_moving(
        drag,
        Motion::uniform(Animation::linear(Duration::from_millis(15))),
    )
}

/// A look whose value and interaction changes run with `motion`.
fn look_moving(drag: DragMapping, motion: Motion) -> SliderLook {
    SliderLook::new(
        SliderLayout {
            row_height: 44.0,
            track_inset: 12.0,
            label_inset: 12.0,
            label_gap: 8.0,
            minimum_track: 72.0,
            label_spacing: None,
        },
        Knob {
            size: KNOB,
            hit: HitRegion::NONE,
        },
        drag,
        motion,
    )
}

/// A red knob that turns blue while the slider is pressed.
fn painted_knob(_spec: &SliderSpec, _env: &Environment) -> SliderLook {
    look(DragMapping::Relative { knob_only: false }).part(Custom::new(
        (
            Driven::<ShapeData, f64>::new(|inputs| ShapeData::of(&knob_rect(inputs))),
            Driven::<Paint, f64>::new(|inputs| {
                let pressed = inputs.state.contains(InteractionState::PRESSED);
                Paint::from(if pressed {
                    WorkingColor::new([0.0, 0.0, 1.0, 1.0])
                } else {
                    WorkingColor::new([1.0, 0.0, 0.0, 1.0])
                })
            }),
        ),
        |recorder, _frame, (knob, paint)| recorder.fill(knob, paint),
    ))
}

/// A knob that is only a backdrop-material lens, tinting what lies behind
/// it — the glass look's lens part.
#[cfg(hydrolysis_gpu_painter)]
fn lens_knob(_spec: &SliderSpec, _env: &Environment) -> SliderLook {
    look(DragMapping::Absolute).part(Custom::new(
        (
            Driven::<ShapeData, f64>::new(|inputs| ShapeData::of(&knob_rect(inputs))),
            Driven::<MaterialEffect, f64>::constant(MaterialEffect::new(vec![0.8, 0.0, 0.0, 0.0])),
        ),
        |recorder, _frame, (lens, effect)| {
            recorder.backdrop_material(lens, GLASS_SHADER, GLASS, effect);
        },
    ))
}

struct Rig {
    runtime: HeadlessRuntime,
    start: Instant,
    next: u64,
    value: Binding<f64>,
}

impl Rig {
    /// A slider at `value` in `direction`, styled by `theme`.
    fn new(theme: MinimalTestTheme, direction: LayoutDirection, value: f64) -> Self {
        let value = Binding::container(value);
        let view = RefCell::new(Some(AnyView::new(
            slider("Volume", &value).hide_label().range(0.0..=1.0),
        )));
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            view.borrow_mut()
                .take()
                .expect("the fixture view is built once")
        });
        let mut env = pumped_test_environment();
        env.insert(direction);
        let runtime = HeadlessRuntime::new_for_tests(env, builder, WIDTH, HEIGHT, theme);
        let mut rig = Self {
            runtime,
            start: Instant::now(),
            next: 0,
            value,
        };
        rig.settle();
        rig
    }

    fn snapshot(&mut self) -> HeadlessSnapshot {
        let at = self.start + Duration::from_millis(self.next * 16);
        self.next += 1;
        self.runtime
            .pump_at(true, at)
            .into_snapshot()
            .expect("a captured frame carries pixels")
    }

    fn settle(&mut self) {
        for _ in 0..8 {
            let _ = self.snapshot();
        }
    }

    fn row_y(&self) -> f32 {
        let targets = &self.runtime.renderer().hit_test.pointer_targets;
        let [target] = targets.as_slice() else {
            panic!("one slider takes one pointer target, got {}", targets.len());
        };
        f64_as_f32(target.bounds.center().y)
    }

    fn press(&mut self, x: f32) {
        let y = self.row_y();
        self.runtime.push_input_event(InputEvent::PointerDown {
            id: POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
        self.settle();
    }
}

/// A theme handing out `look` for every slider.
fn painted(look: impl Fn(&SliderSpec, &Environment) -> SliderLook + 'static) -> MinimalTestTheme {
    MinimalTestTheme::default().with_slider(look)
}

/// A theme handing out `look` for every slider, with the glass switch's
/// backdrop shader registered — which only the GPU painter realizes until
/// water-rs/waterui#4065.
#[cfg(hydrolysis_gpu_painter)]
fn glass(look: impl Fn(&SliderSpec, &Environment) -> SliderLook + 'static) -> MinimalTestTheme {
    MinimalTestTheme {
        chrome: plan(),
        ..MinimalTestTheme::default()
    }
    .with_slider(look)
}

fn save(case: &str, stage: &str, snapshot: &HeadlessSnapshot) {
    let path = crate::testing::artifact_snapshot_path("hydrolysis", case, stage);
    std::fs::create_dir_all(path.parent().expect("the case directory"))
        .expect("the capture directory must be creatable");
    image::RgbaImage::from_raw(
        snapshot.width(),
        snapshot.height(),
        snapshot.rgba8().to_vec(),
    )
    .expect("snapshot dimensions must match the rgba buffer")
    .save(&path)
    .expect("the frame must be writable");
}

/// A value change from outside moves a knob drawn by an ordinary part.
#[test]
fn a_value_change_moves_a_painted_knob() {
    let mut rig = Rig::new(painted(painted_knob), LayoutDirection::LeftToRight, 0.2);
    let before = rig.snapshot();
    rig.value.set(0.8);
    rig.settle();
    let after = rig.snapshot();
    save("slider_painted_knob_value", "before", &before);
    save("slider_painted_knob_value", "after", &after);
    assert_ne!(
        before.rgba8(),
        after.rgba8(),
        "the knob moves from 0.2 to 0.8"
    );
}

/// A press recolours a knob whose paint is driven by the pressed state.
#[test]
fn a_press_recolours_a_painted_knob() {
    let mut rig = Rig::new(painted(painted_knob), LayoutDirection::LeftToRight, 0.5);
    let before = rig.snapshot();
    rig.press(f64_as_f32(f64::from(WIDTH) / 2.0));
    let pressed = rig.snapshot();
    save("slider_painted_knob_press", "before", &before);
    save("slider_painted_knob_press", "pressed", &pressed);
    assert_ne!(
        before.rgba8(),
        pressed.rgba8(),
        "the knob turns blue while pressed"
    );
}

/// A value change moves a knob that is a backdrop-material lens.
#[test]
#[cfg(hydrolysis_gpu_painter)]
fn a_value_change_moves_a_lens_knob() {
    let mut rig = Rig::new(glass(lens_knob), LayoutDirection::LeftToRight, 0.2);
    let before = rig.snapshot();
    rig.value.set(0.8);
    rig.settle();
    let after = rig.snapshot();
    save("slider_lens_knob_value", "before", &before);
    save("slider_lens_knob_value", "after", &after);
    assert_ne!(
        before.rgba8(),
        after.rgba8(),
        "the lens moves from 0.2 to 0.8"
    );
}

/// A theme whose slider look has a red painted knob and a lens knob four
/// points larger, both following the value under a one-second linear value
/// motion.
#[cfg(hydrolysis_gpu_painter)]
fn lens_over_painted_knob() -> MinimalTestTheme {
    let motion = Motion::uniform(Animation::linear(Duration::from_secs(1)));
    glass(move |_spec: &SliderSpec, _env: &Environment| {
        look_moving(DragMapping::Absolute, motion.clone())
            .part(Custom::new(
                (Driven::<ShapeData, f64>::new(|inputs| {
                    ShapeData::of(&knob_rect(inputs))
                }),),
                |recorder, _frame, (knob,)| {
                    recorder.fill(knob, WorkingColor::new([1.0, 0.0, 0.0, 1.0]));
                },
            ))
            .part(Custom::new(
                (
                    Driven::<ShapeData, f64>::new(|inputs| {
                        ShapeData::of(&knob_rect(inputs).inflate(4.0, 4.0))
                    }),
                    Driven::<MaterialEffect, f64>::constant(MaterialEffect::new(vec![
                        -0.8, -0.8, 0.0, 0.0,
                    ])),
                ),
                |recorder, _frame, (lens, effect)| {
                    recorder.backdrop_material(lens, GLASS_SHADER, GLASS, effect);
                },
            ))
    })
}

/// A slider whose painted knob and lens knob follow one value under a
/// one-second linear value motion: mid-transition the lens sits where the
/// painted knob does. The GPU painter captures the frames; the mirror
/// mount, committing the same view, holds the sampled tree the assertion
/// reads — the lens member's clip and the painted knob's fill operand.
#[test]
#[cfg(hydrolysis_gpu_painter)]
fn a_lens_knob_moves_with_the_painted_knob() {
    let mut rig = Rig::new(lens_over_painted_knob(), LayoutDirection::LeftToRight, 0.2);
    let before = rig.snapshot();
    rig.value.set(0.8);
    for _ in 0..6 {
        let _ = rig.snapshot();
    }
    let mid = rig.snapshot();
    rig.settle();
    for _ in 0..64 {
        let _ = rig.snapshot();
    }
    let after = rig.snapshot();
    save("slider_lens_follows_painted", "before", &before);
    save("slider_lens_follows_painted", "mid", &mid);
    save("slider_lens_follows_painted", "after", &after);

    let value = Binding::container(0.2);
    let mut renderer = super::test_renderer_with_theme(lens_over_painted_knob());
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(slider("Volume", &value).hide_label().range(0.0..=1.0)),
        &pumped_test_environment(),
        Rect::new(0.0, 0.0, f64::from(WIDTH), f64::from(HEIGHT)),
        kurbo::Affine::IDENTITY,
        kurbo::Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
    renderer.commit_mirror();
    let start = Instant::now();
    let mirror = renderer.mirror();
    mirror.present(start);
    let (rest, _) = knobs(mirror);
    value.set(0.8);
    mirror.present(start + Duration::from_millis(16));
    mirror.present(start + Duration::from_millis(116));
    let (painted, lens) = knobs(mirror);
    mirror.present(start + Duration::from_secs(2));
    let (end, _) = knobs(mirror);

    assert!(
        painted.center().x > rest.center().x + 1.0 && painted.center().x < end.center().x - 1.0,
        "the frame is mid-transition: the painted knob is at {painted:?} between {rest:?} \
         and {end:?}"
    );
    let expected = painted.inflate(4.0, 4.0);
    for (edge, (got, want)) in [
        (lens.x0, expected.x0),
        (lens.y0, expected.y0),
        (lens.x1, expected.x1),
        (lens.y1, expected.y1),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            (got - want).abs() < 1e-3,
            "edge {edge}: the lens {lens:?} sits on the painted knob inflated by 4, {expected:?}"
        );
    }
}

/// The painted knob's and the lens's rects in window points, as the
/// mirror's tree samples them: the painted knob's red fill operand and the
/// lens member's clip.
#[cfg(hydrolysis_gpu_painter)]
fn knobs(mirror: &super::mirror::MirrorWindow) -> (Rect, Rect) {
    use cherenkov_record::Command;

    let red = Paint::from(WorkingColor::new([1.0, 0.0, 0.0, 1.0]));
    let painted: Vec<Rect> = mirror
        .content_layers()
        .into_iter()
        .flat_map(|layer| {
            let world = mirror.world(layer);
            let picture = mirror.picture(layer).expect("a content layer");
            picture
                .display_list()
                .commands()
                .iter()
                .filter_map(|command| match command {
                    Command::Fill { shape, paint } if *paint == red => {
                        Some(world.transform_rect_bbox(shape.bounds()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let [painted] = painted.as_slice() else {
        panic!("one red knob fill, got {painted:?}");
    };
    let [member] = mirror.backdrop_layers()[..] else {
        panic!("one lens member layer");
    };
    let clip = mirror
        .layer(member)
        .clip
        .as_ref()
        .expect("the lens member clips to its shape")
        .bounds();
    let parent_world = mirror
        .parent(member)
        .map_or(kurbo::Affine::IDENTITY, |parent| mirror.world(parent));
    let lens = (parent_world * mirror.layer(member).transform).transform_rect_bbox(clip);
    (*painted, lens)
}

/// A right-to-left slider mirrors an ordinary part about its row: at 0.2
/// the knob sits a fifth of the way in from the right.
#[test]
fn a_mirrored_slider_mirrors_a_painted_knob() {
    let mut rig = Rig::new(painted(painted_knob), LayoutDirection::RightToLeft, 0.2);
    let frame = rig.snapshot();
    save("slider_painted_knob_rtl", "rest", &frame);
}

/// A right-to-left slider records a lens part, mirrored about its row: the
/// material is listed with the parts' mirror rather than recorded inside a
/// transform scope, where a material cannot record (water-rs/waterui#4504).
#[test]
#[cfg(hydrolysis_gpu_painter)]
fn a_mirrored_slider_records_a_lens_knob() {
    let mut rig = Rig::new(glass(lens_knob), LayoutDirection::RightToLeft, 0.2);
    let frame = rig.snapshot();
    save("slider_lens_knob_rtl", "rest", &frame);
}

/// A press recolours a button chrome's custom part whose paint is driven
/// by the pressed state: the chrome's custom parts take the same recording
/// path as a slider's parts.
#[test]
fn a_press_recolours_a_chrome_custom_part() {
    use waterui_controls::button::button;

    let paint = Driven::<Paint, ()>::new(|inputs| {
        let pressed = inputs.state.contains(InteractionState::PRESSED);
        Paint::from(if pressed {
            WorkingColor::new([0.0, 0.0, 1.0, 1.0])
        } else {
            WorkingColor::new([1.0, 0.0, 0.0, 1.0])
        })
    });
    let mut env = pumped_test_environment();
    env.install(super::invisible_chrome().custom(Custom::new(
        (paint,),
        |recorder, frame, (paint,)| {
            recorder.fill(frame, paint);
        },
    )));
    let view = RefCell::new(Some(AnyView::new(ViewExt::size(
        button("Tap").action(|| {}),
        200.0,
        60.0,
    ))));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the fixture view is built once")
    });
    let runtime =
        HeadlessRuntime::new_for_tests(env, builder, WIDTH, HEIGHT, MinimalTestTheme::default());
    let mut rig = Rig {
        runtime,
        start: Instant::now(),
        next: 0,
        value: Binding::container(0.0),
    };
    rig.settle();
    let before = rig.snapshot();
    let center = rig.runtime.renderer().hit_test.pointer_targets[0]
        .bounds
        .center();
    rig.runtime.push_input_event(InputEvent::PointerDown {
        id: POINTER,
        kind: PointerKind::Mouse,
        x: f64_as_f32(center.x),
        y: f64_as_f32(center.y),
        button: PointerButton::Primary,
    });
    for _ in 0..16 {
        let _ = rig.snapshot();
    }
    let pressed = rig.snapshot();
    save("chrome_custom_press", "before", &before);
    save("chrome_custom_press", "pressed", &pressed);
    assert_ne!(
        before.rgba8(),
        pressed.rgba8(),
        "the custom part turns blue while pressed"
    );
}
