//! Hydrolysis's realization of the theme's chrome primitives
//! ([`Chrome`](crate::theme::Chrome)) and of the [`Driven`] values a look
//! hands out.
//!
//! A [`LiveChrome`] records a widget part's chrome once, as live sources
//! the node's program lists around the widget's content: the surface below
//! it (elevation, fill or backdrop material, border, custom parts) and the
//! interaction layers above it (state layer, ripple waves, wash, focus
//! ring). Every value that follows the widget's inputs is an [`Operand`] —
//! a signal the recording subscribes to — so a hover, press or focus change
//! retargets operands under the chrome's [`Motion`] and the engine animates
//! the recorded commands in place: nothing re-records, no node is marked
//! and no layout pass runs. A signal a [`Driven`] value follows — a theme
//! token, a colour that depends on the widget's selection — retargets the
//! same operands under the animation its change carries.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use cherenkov_record::{
    ColorStop, Draw as _, Fixed, MaterialEffect, Paint, RadialGradient, Recorder, Shadow,
    ShapeData, Stroke, WorkingColor,
};
use kurbo::{Affine, Point, Rect, Vec2};
use nami::{Computed, Signal};
use waterui_core::Environment;
use waterui_core::animation::{Animation, ChangeAnimation};
use waterui_core::interaction::InteractionState;

use crate::renderer::recording::{LoweredResources, SceneResources};
use crate::renderer::{HydrolysisRenderer, RenderContext};
use crate::theme::live::Operand;
use crate::theme::{
    Change, Chrome, Corners, CustomPart, Driven, Elevation, Fill, FocusIndicator, Inputs,
    InteractionMotion, LiveDriven, MAX_PRESS_WAVES, Motion, RealizedCustom, Retarget, Ripple, Wash,
};
use waterui_graphics::color::Color;

/// Content a node's program lists as a live item: recorded once into its
/// own engine layer and kept while the program lists the same source.
///
/// A source may contain fixed commands or operands read as signals. Signal
/// updates reach the engine without another recording. The mount re-records
/// only when the program lists a different source in its place, or when the
/// node mounts on a new engine surface.
pub trait LiveSource {
    /// Records the source's commands into `recorder`, in the record space of
    /// the item's transform. Returned registrations stay alive with the
    /// installed content, including while its retirement is pending.
    fn record(
        &self,
        recorder: &mut Recorder,
        resources: &SceneResources,
    ) -> Option<LoweredResources>;

    /// Fixed content available to scene inspection, when this source owns a recording.
    fn recording(&self) -> Option<&crate::renderer::recording::Recording> {
        None
    }
}

/// The focused editor's caret, retained across animation ticks.
pub(super) struct LiveCaret {
    rect: Operand<Rect>,
    clip: Operand<Rect>,
    paint: Operand<Paint>,
    group: Operand<cherenkov_record::Group>,
}

impl LiveCaret {
    pub(super) fn new(rect: Rect, clip: Rect, paint: Paint, alpha: f32) -> Self {
        Self {
            rect: Operand::new(rect),
            clip: Operand::new(clip),
            paint: Operand::new(paint),
            group: Operand::new(cherenkov_record::Group::new().opacity(alpha)),
        }
    }

    pub(super) fn update(&self, rect: Rect, clip: Rect, paint: Paint, alpha: f32) {
        self.rect.set(rect, None);
        self.clip.set(clip, None);
        self.paint.set(paint, None);
        self.group
            .set(cherenkov_record::Group::new().opacity(alpha), None);
    }
}

impl LiveSource for LiveCaret {
    fn record(
        &self,
        recorder: &mut Recorder,
        _resources: &SceneResources,
    ) -> Option<LoweredResources> {
        recorder.clip(self.clip.live(), |recorder| {
            recorder.group(self.group.live(), |recorder| {
                recorder.fill(self.rect.live(), self.paint.live());
            });
        });
        None
    }
}

struct BorderOperands {
    shape: Operand<ShapeData>,
    stroke: Operand<Stroke>,
    paint: Operand<Paint>,
}

struct WaveOperands {
    shape: Operand<ShapeData>,
    paint: Operand<Paint>,
    /// The opacity the wave's paint is heading to, so a colour change
    /// recolours it at the same opacity.
    opacity: Cell<f32>,
}

struct RingOperands {
    shape: Operand<ShapeData>,
    stroke: Stroke,
    paint: Operand<Paint>,
    color: Computed<WorkingColor>,
}

struct WashOperands {
    paint: Operand<Paint>,
    color: Computed<WorkingColor>,
    radius: f64,
    /// Where the current or last press landed, in the widget's space.
    center: Cell<Point>,
}

impl WashOperands {
    /// The wash at rest over `frame`, resolving its colour in `env`.
    fn new(wash: &Wash, frame: Rect, env: &Environment) -> Self {
        let color = wash.color.resolve(env);
        let center = frame.center();
        Self {
            paint: Operand::new(wash_paint(center, wash.radius, color.snapshot(), 0.0)),
            color,
            radius: wash.radius,
            center: Cell::new(center),
        }
    }
}

/// How the surface is filled.
enum FillOperands {
    Paint(Operand<Paint>),
    Backdrop(Operand<MaterialEffect>),
}

/// What moved the chrome's inputs, deciding each operand's animation.
#[derive(Clone, Copy)]
enum Retargeting<'a> {
    /// The interaction state moved from `old`.
    Interaction(InteractionState),
    /// A signal a value follows, or the press point, changed under this
    /// animation.
    Animated(Option<&'a Animation>),
}

/// One widget part's chrome, recorded once with live operands.
///
/// The widget keeps it across renders while its [`Chrome`] and bounds stay
/// the same; the interaction handles bound to the widget steer it between
/// renders through [`Self::set_flag`], [`Self::begin_wave`] and
/// [`Self::fade_wave`].
pub struct LiveChrome {
    spec: Chrome,
    /// The widget's bounds in its record space.
    bounds: Rect,
    /// The chrome's frame in the widget's record space.
    frame: Rect,
    state: Cell<InteractionState>,
    press_point: Cell<Option<Point>>,
    /// The widget's own reported-state binding, kept current between
    /// renders so content resolving per-state values follows the chrome.
    report: nami::Binding<InteractionState>,
    layer_color: Computed<WorkingColor>,
    shape: Operand<ShapeData>,
    fill: FillOperands,
    border: Option<BorderOperands>,
    /// The key and ambient shadows; `None` when the surface never elevates.
    elevation: Option<(Operand<Shadow>, Operand<Shadow>)>,
    tint: Operand<Paint>,
    waves: Option<(Ripple, [WaveOperands; MAX_PRESS_WAVES])>,
    wash: Option<WashOperands>,
    ring: Option<RingOperands>,
    /// The press scale about the frame's centre, for content that follows
    /// it.
    content_scale: Operand<Affine>,
    customs: Vec<RealizedCustom<()>>,
    /// A slider's parts over its value.
    slider: Option<LiveSlider>,
    /// The content styling realized live: the group opacity and the label
    /// colour when they vary.
    content: LiveContent,
    /// The subscriptions to the signals the chrome's values follow.
    guards: RefCell<Vec<Box<dyn Any>>>,
    /// How many times the mount recorded any of its live sources.
    #[cfg(test)]
    recordings: Cell<u32>,
}

impl std::fmt::Debug for LiveChrome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveChrome")
            .field("frame", &self.frame)
            .field("state", &self.state.get())
            .finish_non_exhaustive()
    }
}

/// Every interaction state, for resolving which values a surface can take.
fn every_state() -> impl Iterator<Item = InteractionState> {
    (0..=InteractionState::all().bits()).map(InteractionState::from_bits_truncate)
}

/// What a look styles in the content its chrome wraps: the group opacity
/// the whole widget draws at and the colour its label takes.
#[derive(Clone, Default)]
pub struct ChromeContent {
    pub group_opacity: Option<Driven<f32>>,
    pub label_color: Option<Driven<Color>>,
    /// A slider's parts over its value; the chrome's bounds are the row
    /// between the two ends of the track.
    pub slider: Option<SliderContent>,
}

/// What a slider's chrome records beyond its own layers: the look's parts
/// over the normalized value, and the bubble of its value indicator.
#[derive(Clone)]
pub struct SliderContent {
    /// The look's parts in paint order.
    pub parts: Vec<Rc<dyn CustomPart<f64>>>,
    /// The value normalized to the range, `0..=1`. The chrome follows it and
    /// holds the slider's live value; one instance per slider, whose
    /// identity the chrome is kept by.
    pub value: Rc<Computed<f64>>,
    /// Whether the parts draw mirrored about the row, in a right-to-left
    /// layout.
    pub mirrored: bool,
    /// The value indicator's bubble, when the slider has one.
    pub indicator: Option<IndicatorContent>,
}

/// A slider's value indicator bubble: where it rests and how far it follows
/// the knob.
#[derive(Clone)]
pub struct IndicatorContent {
    /// The look's container part, recorded on [`Self::rest`].
    pub container: Rc<dyn CustomPart<f64>>,
    /// The bubble's frame centred on the value's start of the track.
    pub rest: Rect,
    /// The least and greatest x the bubble's centre reaches, so it stays in
    /// the slider's bounds.
    pub limits: (f64, f64),
}

impl SliderContent {
    /// Whether `other` is this content: the same parts, value and bubble.
    fn same(&self, other: &Self) -> bool {
        self.parts.len() == other.parts.len()
            && self
                .parts
                .iter()
                .zip(&other.parts)
                .all(|(a, b)| Rc::ptr_eq(a, b))
            && Rc::ptr_eq(&self.value, &other.value)
            && self.mirrored == other.mirrored
            && match (&self.indicator, &other.indicator) {
                (Some(a), Some(b)) => {
                    Rc::ptr_eq(&a.container, &b.container)
                        && a.rest == b.rest
                        && a.limits == b.limits
                }
                (None, None) => true,
                _ => false,
            }
    }
}

/// A [`SliderContent`] realized: the look's parts as live operands, and the
/// slider's value.
struct LiveSlider {
    source: SliderContent,
    /// The normalized value the parts show.
    value: Cell<f64>,
    /// The normalized value when the current press landed, for a drag that
    /// moves the value relative to it.
    press_value: Cell<f64>,
    /// The offset of an active drag from where it was pressed.
    drag: Cell<Option<Vec2>>,
    parts: Vec<RealizedCustom<f64>>,
    indicator: Option<LiveIndicator>,
}

/// The value indicator realized: its container, and the operands that show
/// it while the slider is held and move it along with the knob.
struct LiveIndicator {
    container: RealizedCustom<f64>,
    opacity: Operand<f32>,
    offset: Operand<Affine>,
}

impl LiveSlider {
    fn new(content: &SliderContent, frame: Rect, state: InteractionState) -> Self {
        let value = content.value.snapshot();
        let inputs = Inputs {
            frame,
            state,
            value: &value,
            drag: None,
            press_point: None,
        };
        let parts = content
            .parts
            .iter()
            .map(|part| part.realize(&inputs))
            .collect();
        let indicator = content.indicator.as_ref().map(|indicator| LiveIndicator {
            container: indicator.container.realize(&Inputs {
                frame: indicator.rest,
                ..inputs
            }),
            opacity: Operand::new(Self::indicator_opacity(state)),
            offset: Operand::new(Self::indicator_offset(content, frame, value)),
        });
        Self {
            source: content.clone(),
            value: Cell::new(value),
            press_value: Cell::new(value),
            drag: Cell::new(None),
            parts,
            indicator,
        }
    }

    /// The indicator shows while the slider is held.
    const fn indicator_opacity(state: InteractionState) -> f32 {
        if state.contains(InteractionState::PRESSED) && !state.contains(InteractionState::DISABLED)
        {
            1.0
        } else {
            0.0
        }
    }

    /// The indicator's offset from its resting place for `value`: following
    /// the knob along the row `frame`, held inside its limits.
    fn indicator_offset(content: &SliderContent, frame: Rect, value: f64) -> Affine {
        let Some(indicator) = &content.indicator else {
            return Affine::IDENTITY;
        };
        let along = value * frame.width();
        let x = if content.mirrored {
            frame.x1 - along
        } else {
            frame.x0 + along
        };
        let (least, greatest) = indicator.limits;
        Affine::translate((x.clamp(least, greatest) - indicator.rest.center().x, 0.0))
    }

    /// Every look value the slider realizes as an operand, and whether it
    /// belongs to the value indicator's container, whose frame is the
    /// bubble.
    fn drivens(&self) -> impl Iterator<Item = (Rc<dyn Retarget<f64>>, bool)> + '_ {
        let parts = self
            .parts
            .iter()
            .flat_map(|part| part.drivens.iter().cloned().map(|driven| (driven, false)));
        let container = self
            .indicator
            .iter()
            .flat_map(|indicator| indicator.container.drivens.iter().cloned())
            .map(|driven| (driven, true));
        parts.chain(container)
    }
}

/// The states that change between renders: a content value varying with
/// them is realized live.
const LIVE_STATES: InteractionState = InteractionState::HOVERED
    .union(InteractionState::PRESSED)
    .union(InteractionState::FOCUSED);

/// The [`ChromeContent`] values that vary between renders — with hover,
/// press or focus over the widget's disabled and selected state, or with a
/// signal — realized as operands. A value that does not stays with the
/// widget's render.
struct LiveContent {
    /// The look's values this realizes.
    source: ChromeContent,
    /// The label colour in the chrome's colour space.
    resolved_color: Option<Driven<WorkingColor>>,
    frame: Rect,
    group_opacity: Option<Rc<LiveDriven<f32, ()>>>,
    label_color: Option<Rc<LiveDriven<WorkingColor, ()>>>,
}

/// Whether `driven` varies between renders over `state`'s disabled and
/// selected bits: with a signal, with hover, press or focus, or with any of
/// the widget's `values` (a value change is not a render).
pub fn varies_live<T: PartialEq + 'static, V: 'static>(
    driven: &Driven<T, V>,
    frame: Rect,
    state: InteractionState,
    values: &[V],
) -> bool {
    let base = state.difference(LIVE_STATES);
    let at = |live: InteractionState, value: &V| {
        driven.target(&Inputs::at_rest(frame, base.union(live), value))
    };
    let first = values.first().expect("a widget has at least one value");
    let rest = at(InteractionState::empty(), first);
    driven.follows_signal()
        || values.iter().any(|value| {
            every_state()
                .filter(|live| LIVE_STATES.contains(*live))
                .any(|live| at(live, value) != rest)
        })
}

impl LiveContent {
    fn new(
        content: &ChromeContent,
        frame: Rect,
        state: InteractionState,
        env: &Environment,
    ) -> Self {
        let inputs = Inputs::at_rest(frame, state, &());
        let resolved_color = content.label_color.as_ref().map(|color| {
            let env = env.clone();
            color.map(move |color| color.resolve(&env).snapshot())
        });
        let group_opacity = content
            .group_opacity
            .as_ref()
            .filter(|opacity| varies_live(*opacity, frame, state, &[()]))
            .map(|opacity| Rc::new(LiveDriven::new(opacity.clone(), &inputs)));
        let label_color = resolved_color
            .as_ref()
            .filter(|color| varies_live(*color, frame, state, &[()]))
            .map(|color| Rc::new(LiveDriven::new(color.clone(), &inputs)));
        Self {
            source: content.clone(),
            resolved_color,
            frame,
            group_opacity,
            label_color,
        }
    }

    /// Whether this realizes `content`'s values in `state`: the same look
    /// values, live where they vary over `state`.
    fn realizes(&self, content: &ChromeContent, state: InteractionState) -> bool {
        fn same<T: 'static>(a: Option<&Driven<T>>, b: Option<&Driven<T>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => a.same(b),
                (None, None) => true,
                _ => false,
            }
        }
        let opacity_live = self
            .source
            .group_opacity
            .as_ref()
            .is_some_and(|opacity| varies_live(opacity, self.frame, state, &[()]));
        let color_live = self
            .resolved_color
            .as_ref()
            .is_some_and(|color| varies_live(color, self.frame, state, &[()]));
        if opacity_live != self.group_opacity.is_some() || color_live != self.label_color.is_some()
        {
            return false;
        }
        same(
            self.source.group_opacity.as_ref(),
            content.group_opacity.as_ref(),
        ) && same(
            self.source.label_color.as_ref(),
            content.label_color.as_ref(),
        )
    }

    /// The operands' retargeting handles.
    fn drivens(&self) -> impl Iterator<Item = Rc<dyn Retarget<()>>> {
        let opacity = self
            .group_opacity
            .clone()
            .map(|live| live as Rc<dyn Retarget<()>>);
        let color = self
            .label_color
            .clone()
            .map(|live| live as Rc<dyn Retarget<()>>);
        opacity.into_iter().chain(color)
    }
}

fn with_alpha(color: WorkingColor, opacity: f32) -> Paint {
    Paint::from(color.with_alpha(color.components[3] * opacity))
}

/// A radial falloff of `color` from `center` to transparent at `radius`.
fn wash_paint(center: Point, radius: f64, color: WorkingColor, opacity: f32) -> Paint {
    let mut gradient = RadialGradient::new(center, radius);
    gradient.stops = vec![
        ColorStop {
            offset: 0.0,
            color: color.with_alpha(color.components[3] * opacity),
        },
        ColorStop {
            offset: 1.0,
            color: color.with_alpha(0.0),
        },
    ];
    Paint::Radial(gradient)
}

impl LiveChrome {
    /// The chrome `spec` on `bounds` (the widget's record-space bounds), at
    /// rest in `state`, resolving its colours in `env`. `report` is the
    /// widget's reported-state binding.
    fn new(
        spec: Chrome,
        content: &ChromeContent,
        bounds: Rect,
        state: InteractionState,
        env: &Environment,
        report: nami::Binding<InteractionState>,
    ) -> Self {
        let frame = spec.frame(bounds);
        let slider = content
            .slider
            .as_ref()
            .map(|slider| LiveSlider::new(slider, frame, state));
        let content = LiveContent::new(content, frame, state, env);
        let inputs = Inputs::at_rest(frame, state, &());
        let surface = spec.surface();
        let (silhouette, corners) = Self::silhouette_of(&spec, &inputs);
        let shape = surface.shape();
        let border = surface.get_border().map(|border| {
            let resolved = border.target(&inputs);
            BorderOperands {
                shape: Operand::new(shape.outset(silhouette, corners, -resolved.width / 2.0)),
                stroke: Operand::new(Stroke::new(resolved.width)),
                paint: Operand::new(resolved.paint),
            }
        });
        let elevation = surface.get_elevation();
        let elevates = elevation.follows_signal()
            || every_state().any(|state| {
                !elevation
                    .target(&Inputs::at_rest(frame, state, &()))
                    .is_none()
            });
        let elevation = elevates.then(|| {
            let resolved = elevation.target(&inputs);
            (Operand::new(resolved.key), Operand::new(resolved.ambient))
        });
        let fill = match surface.fill() {
            Fill::Paint(paint) => FillOperands::Paint(Operand::new(paint.target(&inputs))),
            Fill::Backdrop(backdrop) => {
                FillOperands::Backdrop(Operand::new(backdrop.effect().target(&inputs)))
            }
        };
        let layer = spec.state_layer();
        let layer_color = layer.color.resolve(env);
        let color = layer_color.snapshot();
        let waves = spec.press().get_ripple().map(|ripple| {
            (
                ripple.clone(),
                core::array::from_fn(|_| WaveOperands {
                    shape: Operand::new(ShapeData::Circle(kurbo::Circle::new(frame.center(), 0.0))),
                    paint: Operand::new(with_alpha(color, 0.0)),
                    opacity: Cell::new(0.0),
                }),
            )
        });
        let wash = spec
            .press()
            .get_wash()
            .map(|wash| WashOperands::new(wash, frame, env));
        let ring = match spec.focus() {
            FocusIndicator::Ring {
                width,
                offset,
                color,
            } => {
                let color = color.resolve(env);
                Some(RingOperands {
                    shape: Operand::new(shape.outset(silhouette, corners, offset + width / 2.0)),
                    stroke: Stroke::new(*width),
                    paint: Operand::new(Self::ring_paint(color.snapshot(), state)),
                    color,
                })
            }
            FocusIndicator::Tint => None,
        };
        let customs = spec
            .customs()
            .iter()
            .map(|custom| custom.realize(&inputs))
            .collect();
        Self {
            shape: Operand::new(shape.shape(silhouette, corners)),
            fill,
            border,
            elevation,
            tint: Operand::new(with_alpha(color, layer.opacity(state, spec.press()))),
            waves,
            wash,
            ring,
            content_scale: Operand::new(Self::scale_about(frame, Self::scale_of(&spec, state))),
            customs,
            slider,
            content,
            layer_color,
            guards: RefCell::new(Vec::new()),
            state: Cell::new(state),
            press_point: Cell::new(None),
            report,
            spec,
            bounds,
            frame,
            #[cfg(test)]
            recordings: Cell::new(0),
        }
    }

    /// Subscribes the chrome to every signal its values follow, so a change
    /// retargets its operands under the change's animation.
    fn subscribe(this: &Rc<Self>) {
        let notify: Rc<dyn Fn(Option<Animation>)> = {
            let weak: Weak<Self> = Rc::downgrade(this);
            Rc::new(move |animation: Option<Animation>| {
                if let Some(chrome) = weak.upgrade() {
                    chrome.retarget(Retargeting::Animated(animation.as_ref()));
                }
            })
        };
        let mut guards: Vec<Box<dyn Any>> = Vec::new();
        let surface = this.spec.surface();
        let drivens = [
            Some(surface.shape().corners().subscribe(Rc::clone(&notify))),
            surface
                .get_border()
                .map(|border| border.subscribe(Rc::clone(&notify))),
            Some(surface.get_elevation().subscribe(Rc::clone(&notify))),
            Some(match surface.fill() {
                Fill::Paint(paint) => paint.subscribe(Rc::clone(&notify)),
                Fill::Backdrop(backdrop) => backdrop.effect().subscribe(Rc::clone(&notify)),
            }),
        ];
        guards.extend(drivens.into_iter().flatten().flatten());
        let watch_color = |color: &Computed<WorkingColor>| -> Box<dyn Any> {
            let notify = Rc::clone(&notify);
            Box::new(color.watch(move |context| {
                notify(ChangeAnimation::of(context.metadata()).animation());
            }))
        };
        guards.push(watch_color(&this.layer_color));
        if let Some(ring) = &this.ring {
            guards.push(watch_color(&ring.color));
        }
        if let Some(wash) = &this.wash {
            guards.push(watch_color(&wash.color));
        }
        for driven in this.drivens() {
            let weak: Weak<Self> = Rc::downgrade(this);
            let target = Rc::downgrade(&driven);
            let notify: Rc<dyn Fn(Option<Animation>)> =
                Rc::new(move |animation: Option<Animation>| {
                    if let (Some(chrome), Some(target)) = (weak.upgrade(), target.upgrade()) {
                        target.retarget(&chrome.inputs(), Change::Animated(animation.as_ref()));
                    }
                });
            if let Some(guard) = driven.subscribe(notify) {
                guards.push(guard);
            }
        }
        if let Some(slider) = &this.slider {
            for (driven, in_indicator) in slider.drivens() {
                let weak: Weak<Self> = Rc::downgrade(this);
                let target = Rc::downgrade(&driven);
                let notify: Rc<dyn Fn(Option<Animation>)> =
                    Rc::new(move |animation: Option<Animation>| {
                        if let (Some(chrome), Some(target)) = (weak.upgrade(), target.upgrade()) {
                            chrome.retarget_slider_driven(
                                &*target,
                                in_indicator,
                                Change::Animated(animation.as_ref()),
                            );
                        }
                    });
                if let Some(guard) = driven.subscribe(notify) {
                    guards.push(guard);
                }
            }
            let weak: Weak<Self> = Rc::downgrade(this);
            guards.push(Box::new(slider.source.value.watch(move |context| {
                if let Some(chrome) = weak.upgrade() {
                    let (value, metadata) = context.into_parts();
                    chrome.set_value(value, ChangeAnimation::of(&metadata));
                }
            })));
        }
        *this.guards.borrow_mut() = guards;
    }

    const fn inputs(&self) -> Inputs<'static, ()> {
        Inputs {
            frame: self.frame,
            state: self.state.get(),
            value: &(),
            drag: None,
            press_point: self.press_point.get(),
        }
    }

    /// The press scale factor in `state`.
    fn scale_of(spec: &Chrome, state: InteractionState) -> f64 {
        match spec.press().get_scale() {
            Some(amount) if state.contains(InteractionState::PRESSED) => 1.0 + amount,
            _ => 1.0,
        }
    }

    /// Scaling by `scale` about `frame`'s centre.
    fn scale_about(frame: Rect, scale: f64) -> Affine {
        let center = frame.center().to_vec2();
        Affine::translate(center) * Affine::scale(scale) * Affine::translate(-center)
    }

    /// The silhouette's rect and corners at `inputs`: the corners the
    /// shape drives, scaled with the frame by the press scale.
    fn silhouette_of(spec: &Chrome, inputs: &Inputs<'_, ()>) -> (Rect, Corners) {
        let corners = spec.surface().shape().corners().target(inputs);
        let scale = Self::scale_of(spec, inputs.state);
        if (scale - 1.0).abs() < f64::EPSILON {
            return (inputs.frame, corners);
        }
        let frame = inputs.frame;
        let radii = corners.radii_on(frame);
        let rect = Rect::from_center_size(frame.center(), frame.size() * scale);
        let radii = kurbo::RoundedRectRadii::new(
            radii.top_left * scale,
            radii.top_right * scale,
            radii.bottom_right * scale,
            radii.bottom_left * scale,
        );
        (rect, Corners::Radii(radii))
    }

    fn ring_paint(color: WorkingColor, state: InteractionState) -> Paint {
        let alpha = if state.contains(InteractionState::FOCUSED)
            && !state.contains(InteractionState::DISABLED)
        {
            color.components[3]
        } else {
            0.0
        };
        Paint::from(color.with_alpha(alpha))
    }

    /// The interaction state the chrome's operands show.
    #[cfg(test)]
    pub(crate) const fn state(&self) -> InteractionState {
        self.state.get()
    }

    /// How many times the mount recorded any of this chrome's live sources.
    #[cfg(test)]
    pub(crate) const fn recordings(&self) -> u32 {
        self.recordings.get()
    }

    /// The surface's live silhouette now.
    #[cfg(test)]
    pub(crate) fn shape(&self) -> ShapeData {
        nami::Signal::snapshot(&self.shape)
    }

    /// Whether this chrome realizes `spec` on `bounds`.
    fn realizes(
        &self,
        spec: &Chrome,
        content: &ChromeContent,
        bounds: Rect,
        state: InteractionState,
    ) -> bool {
        self.spec.same(spec)
            && self.content.realizes(content, state)
            && self.bounds == bounds
            && match (&self.slider, &content.slider) {
                (Some(live), Some(slider)) => live.source.same(slider),
                (None, None) => true,
                _ => false,
            }
    }

    /// The animation `driven` retargets with under `retargeting`.
    fn animation<'a>(
        &'a self,
        motion: Option<&'a Motion>,
        retargeting: Retargeting<'a>,
    ) -> Option<&'a Animation> {
        match retargeting {
            Retargeting::Interaction(old) => motion
                .unwrap_or_else(|| self.spec.motion())
                .interaction(old, self.state.get()),
            Retargeting::Animated(animation) => animation,
        }
    }

    fn driven_animation<'a, T: 'static>(
        &'a self,
        driven: &'a Driven<T>,
        retargeting: Retargeting<'a>,
    ) -> Option<&'a Animation> {
        self.animation(driven.get_motion(), retargeting)
    }

    /// Moves every operand to the chrome's current inputs.
    fn retarget(&self, retargeting: Retargeting<'_>) {
        let inputs = self.inputs();
        let state = inputs.state;
        let surface = self.spec.surface();
        let shape = surface.shape();
        let (silhouette, corners) = Self::silhouette_of(&self.spec, &inputs);
        let shape_animation = self.driven_animation(shape.corners(), retargeting);
        self.shape
            .set(shape.shape(silhouette, corners), shape_animation);
        self.content_scale.set(
            Self::scale_about(self.frame, Self::scale_of(&self.spec, state)),
            self.animation(None, retargeting),
        );
        match (&self.fill, surface.fill()) {
            (FillOperands::Paint(operand), Fill::Paint(paint)) => {
                operand.set(
                    paint.target(&inputs),
                    self.driven_animation(paint, retargeting),
                );
            }
            (FillOperands::Backdrop(operand), Fill::Backdrop(backdrop)) => {
                operand.set(
                    backdrop.effect().target(&inputs),
                    self.driven_animation(backdrop.effect(), retargeting),
                );
            }
            _ => unreachable!("a live chrome's fill operands follow its surface's fill"),
        }
        if let (Some(operands), Some(border)) = (&self.border, surface.get_border()) {
            let resolved = border.target(&inputs);
            let animation = self.driven_animation(border, retargeting);
            operands.shape.set(
                shape.outset(silhouette, corners, -resolved.width / 2.0),
                shape_animation,
            );
            operands.stroke.set(Stroke::new(resolved.width), animation);
            operands.paint.set(resolved.paint, animation);
        }
        if let Some((key, ambient)) = &self.elevation {
            let elevation = surface.get_elevation();
            let animation = self.driven_animation(elevation, retargeting);
            let Elevation {
                key: key_shadow,
                ambient: ambient_shadow,
            } = elevation.target(&inputs);
            key.set(key_shadow, animation);
            ambient.set(ambient_shadow, animation);
        }
        self.retarget_layers(state, silhouette, corners, shape_animation, retargeting);
        let change = self.change(retargeting);
        for driven in self.drivens() {
            driven.retarget(&inputs, change);
        }
        if let Some(slider) = &self.slider {
            for (driven, in_indicator) in slider.drivens() {
                self.retarget_slider_driven(&*driven, in_indicator, change);
            }
            if let Some(indicator) = &slider.indicator {
                indicator.opacity.set(
                    LiveSlider::indicator_opacity(state),
                    self.animation(None, retargeting),
                );
            }
        }
    }

    /// What retargets a look value under `retargeting`.
    fn change<'a>(&'a self, retargeting: Retargeting<'a>) -> Change<'a> {
        match retargeting {
            Retargeting::Interaction(old) => Change::Interaction {
                old,
                motion: self.spec.motion(),
            },
            Retargeting::Animated(animation) => Change::Animated(animation),
        }
    }

    /// Moves one of the slider's look values to the chrome's current inputs.
    fn retarget_slider_driven(
        &self,
        driven: &dyn Retarget<f64>,
        in_indicator: bool,
        change: Change<'_>,
    ) {
        let slider = self
            .slider
            .as_ref()
            .expect("only a slider chrome retargets slider values");
        let value = slider.value.get();
        let frame = if in_indicator {
            slider
                .source
                .indicator
                .as_ref()
                .expect("an indicator value belongs to a chrome with an indicator")
                .rest
        } else {
            self.frame
        };
        driven.retarget(
            &Inputs {
                frame,
                state: self.state.get(),
                value: &value,
                drag: slider.drag.get(),
                press_point: self.press_point.get(),
            },
            change,
        );
    }

    /// Moves the slider's parts to `value`, normalized to the range, under
    /// `animation` (`None` snaps).
    fn move_value(&self, value: f64, animation: Option<&Animation>) {
        let Some(slider) = &self.slider else {
            return;
        };
        if slider.value.replace(value).to_bits() == value.to_bits() {
            return;
        }
        let change = Change::Animated(animation);
        for (driven, in_indicator) in slider.drivens() {
            self.retarget_slider_driven(&*driven, in_indicator, change);
        }
        if let Some(indicator) = &slider.indicator {
            // The bubble tracks the knob it is held by.
            indicator.offset.set(
                LiveSlider::indicator_offset(&slider.source, self.frame, value),
                None,
            );
        }
    }

    /// The slider's value changed to `value`, normalized to the range, the
    /// way `change` asks to animate: the look's value motion decides.
    pub(crate) fn set_value(&self, value: f64, change: ChangeAnimation) {
        let animation = self.spec.motion().value.animation(change);
        self.move_value(value, animation.as_ref());
    }

    /// A drag moved the slider's value to `value`, normalized to the range:
    /// the parts follow the pointer, with no animation.
    pub(crate) fn track_value(&self, value: f64) {
        self.move_value(value, None);
    }

    /// The offset of an active drag from where it was pressed, `None`
    /// between drags.
    pub(crate) fn set_drag(&self, drag: Option<Vec2>) {
        let Some(slider) = &self.slider else {
            return;
        };
        if slider.drag.replace(drag) != drag {
            for (driven, in_indicator) in slider.drivens() {
                self.retarget_slider_driven(&*driven, in_indicator, Change::Animated(None));
            }
        }
    }

    /// The slider's value when the current press landed.
    pub(crate) const fn press_value(&self) -> f64 {
        self.slider
            .as_ref()
            .expect("only a slider chrome holds a value")
            .press_value
            .get()
    }

    /// The current press's point in the chrome's record space; `None`
    /// between presses and for a keyboard or synthetic press.
    pub(crate) const fn press_point(&self) -> Option<Point> {
        self.press_point.get()
    }

    /// Every look value the chrome realizes as an operand beyond its own:
    /// the custom parts' and the content's.
    fn drivens(&self) -> impl Iterator<Item = Rc<dyn Retarget<()>>> + '_ {
        self.customs
            .iter()
            .flat_map(|custom| custom.drivens.iter().cloned())
            .chain(self.content.drivens())
    }

    /// The group opacity operand, when the look's group opacity varies.
    pub(crate) fn group_opacity(&self) -> Option<Operand<f32>> {
        self.content
            .group_opacity
            .as_ref()
            .map(|live| live.operand().clone())
    }

    /// The label's paint, following the chrome's label colour operand, when
    /// the look's label colour varies between renders.
    pub(crate) fn label_paint(&self) -> Option<Computed<Paint>> {
        self.content
            .label_color
            .as_ref()
            .map(|live| Computed::new(nami::SignalExt::map(live.operand(), Paint::from)))
    }

    /// The operands that show a slider's value indicator while it is held
    /// and carry it along with the knob; `None` without an indicator.
    pub(crate) fn indicator_operands(&self) -> Option<(Operand<f32>, Operand<Affine>)> {
        self.slider
            .as_ref()?
            .indicator
            .as_ref()
            .map(|indicator| (indicator.opacity.clone(), indicator.offset.clone()))
    }

    /// The press scale about the frame's centre, in the widget's record
    /// space.
    pub(crate) fn content_scale_operand(&self) -> Operand<Affine> {
        self.content_scale.clone()
    }

    /// Whether a press scales the chrome and the content that follows it.
    pub(crate) fn scales_content(&self) -> bool {
        self.spec.press().get_scale().is_some()
    }

    /// Moves the interaction layers over the content — the tint, the press
    /// waves, the wash and the focus ring — to `state`, the ring around
    /// the `silhouette` of `corners` under `shape_animation`.
    fn retarget_layers(
        &self,
        state: InteractionState,
        silhouette: Rect,
        corners: Corners,
        shape_animation: Option<&Animation>,
        retargeting: Retargeting<'_>,
    ) {
        let shape = self.spec.surface().shape();
        let layer = self.spec.state_layer();
        let color = self.layer_color.snapshot();
        let layer_animation = self.animation(None, retargeting);
        self.tint.set(
            with_alpha(color, layer.opacity(state, self.spec.press())),
            layer_animation,
        );
        if let (Retargeting::Animated(animation), Some((_, waves))) = (retargeting, &self.waves) {
            for wave in waves {
                wave.paint
                    .set(with_alpha(color, wave.opacity.get()), animation);
            }
        }
        if let Some(wash) = &self.wash {
            let opacity = if state.contains(InteractionState::PRESSED) {
                1.0
            } else {
                0.0
            };
            wash.paint.set(
                wash_paint(
                    wash.center.get(),
                    wash.radius,
                    wash.color.snapshot(),
                    opacity,
                ),
                layer_animation,
            );
        }
        if let (Some(ring), FocusIndicator::Ring { width, offset, .. }) =
            (&self.ring, self.spec.focus())
        {
            ring.shape.set(
                shape.outset(silhouette, corners, offset + width / 2.0),
                shape_animation,
            );
            ring.paint.set(
                Self::ring_paint(ring.color.snapshot(), state),
                layer_animation,
            );
        }
    }

    /// Moves every operand to `state`, each under its motion's transition
    /// for the change; a change of the disabled or selected state is
    /// applied by a render and snaps.
    pub(crate) fn sync(&self, state: InteractionState) {
        let old = self.state.replace(state);
        if old == state {
            return;
        }
        if !state.contains(InteractionState::PRESSED) {
            self.press_point.set(None);
            if let Some(slider) = &self.slider {
                slider.drag.set(None);
            }
        }
        if self.report.snapshot() != state {
            self.report.set(state);
        }
        self.retarget(Retargeting::Interaction(old));
    }

    /// Sets or clears one live interaction flag (hover, press or focus).
    pub(crate) fn set_flag(&self, flag: InteractionState, on: bool) {
        let mut state = self.state.get();
        if state.contains(InteractionState::DISABLED) {
            return;
        }
        state.set(flag, on);
        self.sync(state);
    }

    /// Starts a press at `origin`, in the widget's record space (`None`
    /// presses at the frame's centre): the press point moves there, ripple
    /// wave `index` snaps to its starting circle and grows under the
    /// ripple's motion while it fades in, and the wash snaps to it.
    pub(crate) fn begin_wave(&self, index: usize, origin: Option<Point>) {
        self.press_point.set(origin);
        if let Some(slider) = &self.slider {
            slider.press_value.set(slider.value.get());
        }
        if let Some(wash) = &self.wash {
            let center = origin.unwrap_or_else(|| self.frame.center());
            wash.center.set(center);
            wash.paint.set(
                wash_paint(center, wash.radius, wash.color.snapshot(), 0.0),
                None,
            );
        }
        let Some((ripple, waves)) = &self.waves else {
            return;
        };
        let (start, end) = ripple.wave(self.frame, origin);
        let wave = &waves[index];
        wave.shape.set(ShapeData::Circle(start), None);
        wave.shape.set(ShapeData::Circle(end), Some(&ripple.grow));
        let opacity = self.spec.state_layer().pressed;
        wave.opacity.set(opacity);
        wave.paint.set(
            with_alpha(self.layer_color.snapshot(), opacity),
            Some(&ripple.fade_in),
        );
    }

    /// Fades ripple wave `index` out under the ripple's fade-out; its
    /// geometry holds.
    pub(crate) fn fade_wave(&self, index: usize) {
        let Some((ripple, waves)) = &self.waves else {
            return;
        };
        waves[index].opacity.set(0.0);
        waves[index].paint.set(
            with_alpha(self.layer_color.snapshot(), 0.0),
            Some(&ripple.fade_out),
        );
    }

    /// Drops every wave without animating.
    pub(crate) fn clear_waves(&self) {
        let Some((_, waves)) = &self.waves else {
            return;
        };
        for wave in waves {
            wave.opacity.set(0.0);
            wave.paint
                .set(with_alpha(self.layer_color.snapshot(), 0.0), None);
        }
    }

    /// The press scale about the frame's centre, for content that grows
    /// with the chrome.
    pub(crate) fn content_scale(&self) -> cherenkov_record::Live<Affine> {
        self.content_scale.live()
    }

    fn record_below(&self, recorder: &mut Recorder) {
        if let Some((key, ambient)) = &self.elevation {
            recorder.shadow(self.shape.live(), ambient.live());
            recorder.shadow(self.shape.live(), key.live());
        }
        if let FillOperands::Paint(paint) = &self.fill {
            recorder.fill(self.shape.live(), paint.live());
            self.record_border(recorder);
        }
    }

    fn record_border(&self, recorder: &mut Recorder) {
        if let Some(border) = &self.border {
            recorder.stroke(
                border.shape.live(),
                border.stroke.live(),
                border.paint.live(),
            );
        }
    }

    fn record_above(&self, recorder: &mut Recorder) {
        recorder.fill(self.shape.live(), self.tint.live());
        if self.waves.is_some() || self.wash.is_some() {
            recorder.clip(self.shape.live(), |recorder| {
                if let Some((_, waves)) = &self.waves {
                    for wave in waves {
                        recorder.fill(wave.shape.live(), wave.paint.live());
                    }
                }
                if let Some(wash) = &self.wash {
                    recorder.fill(self.shape.live(), wash.paint.live());
                }
            });
        }
        if let Some(ring) = &self.ring {
            recorder.stroke(
                ring.shape.live(),
                Fixed(ring.stroke.clone()),
                ring.paint.live(),
            );
        }
    }

    /// How far the chrome draws beyond its frame: the press scale, the
    /// elevation's shadows and the focus ring.
    pub(crate) fn visual_bounds(&self) -> Rect {
        let scale = Self::scale_of(&self.spec, InteractionState::PRESSED);
        let scaled = Rect::from_center_size(self.frame.center(), self.frame.size() * scale);
        let shadow = every_state()
            .map(|state| {
                self.spec
                    .surface()
                    .get_elevation()
                    .target(&Inputs::at_rest(self.frame, state, &()))
                    .extent()
            })
            .fold(0.0_f64, f64::max);
        let ring = match self.spec.focus() {
            FocusIndicator::Ring { width, offset, .. } => offset + width,
            FocusIndicator::Tint => 0.0,
        };
        let reach = shadow.max(ring);
        scaled
            .inflate(reach, reach)
            .union(self.spec.target(self.bounds))
    }

    /// The motion the renderer's interaction bookkeeping runs with for this
    /// chrome. Its scalars snap — the engine animates the chrome — so only
    /// the ripple's hold and fade-out and the hit region's press delay
    /// matter to it.
    pub(crate) fn handle_motion(spec: &Chrome) -> InteractionMotion {
        let snap = Animation::linear(core::time::Duration::ZERO);
        let ripple = spec.press().get_ripple();
        InteractionMotion {
            hover_opacity: spec.state_layer().hover,
            focus_opacity: spec.state_layer().focus,
            pressed_opacity: spec.state_layer().pressed,
            dragged_opacity: spec.state_layer().dragged,
            hover_enter: snap.clone(),
            hover_exit: snap.clone(),
            focus_enter: snap.clone(),
            focus_exit: snap.clone(),
            press_fade_in: ripple.map_or_else(|| snap.clone(), |ripple| ripple.fade_in.clone()),
            press_fade_out: ripple.map_or_else(|| snap.clone(), |ripple| ripple.fade_out.clone()),
            press_grow: ripple.map_or_else(|| snap.clone(), |ripple| ripple.grow.clone()),
            press_shape: spec.motion().press.enter.clone(),
            minimum_press_duration: ripple
                .map_or(core::time::Duration::ZERO, |ripple| ripple.minimum_duration),
            touch_delay: spec.get_hit().get_press_delay(),
        }
    }
}

/// Content that grows with a chrome's press scale: `inner` recorded under
/// the chrome's live scale, re-expressed in the content's own space.
pub struct ScaledContent {
    inner: Rc<dyn LiveSource>,
    chrome: Rc<LiveChrome>,
    /// The content's space relative to the widget's record space.
    placement: Affine,
}

impl ScaledContent {
    /// `inner`, drawn at `placement` relative to the widget's record space,
    /// growing with `chrome`'s press scale.
    pub(crate) fn new(
        inner: Rc<dyn LiveSource>,
        chrome: Rc<LiveChrome>,
        placement: Affine,
    ) -> Self {
        Self {
            inner,
            chrome,
            placement,
        }
    }
}

impl LiveSource for ScaledContent {
    fn record(
        &self,
        recorder: &mut Recorder,
        resources: &SceneResources,
    ) -> Option<LoweredResources> {
        let placement = self.placement;
        let into_content = placement.inverse();
        let scale = self
            .chrome
            .content_scale()
            .map(move |scale| into_content * scale * placement);
        let mut lowered = None;
        recorder.transform(scale, |recorder| {
            lowered = self.inner.record(recorder, resources);
        });
        lowered
    }

    fn recording(&self) -> Option<&crate::renderer::recording::Recording> {
        self.inner.recording()
    }
}

/// Which live source of a [`LiveChrome`] a source records.
#[derive(Clone, Copy)]
enum Half {
    /// Elevation, a paint fill and its border: beneath the widget's content.
    Below,
    /// The border over a backdrop fill.
    Edge,
    /// State layer, ripple waves, wash and focus ring: above the content.
    Above,
}

struct ChromeSource {
    chrome: Rc<LiveChrome>,
    half: Half,
}

impl LiveSource for ChromeSource {
    fn record(
        &self,
        recorder: &mut Recorder,
        _resources: &SceneResources,
    ) -> Option<LoweredResources> {
        #[cfg(test)]
        self.chrome.recordings.set(self.chrome.recordings.get() + 1);
        match self.half {
            Half::Below => self.chrome.record_below(recorder),
            Half::Edge => self.chrome.record_border(recorder),
            Half::Above => self.chrome.record_above(recorder),
        }
        None
    }
}

/// A widget part's live chrome and the sources its program lists: the
/// identity the mount keeps the recordings by.
#[derive(Clone)]
pub struct LiveChromeParts {
    chrome: Rc<LiveChrome>,
    below: Rc<dyn LiveSource>,
    edge: Option<Rc<dyn LiveSource>>,
    above: Rc<dyn LiveSource>,
}

impl LiveChromeParts {
    fn new(chrome: LiveChrome) -> Self {
        let chrome = Rc::new(chrome);
        LiveChrome::subscribe(&chrome);
        let source = |half| -> Rc<dyn LiveSource> {
            Rc::new(ChromeSource {
                chrome: Rc::clone(&chrome),
                half,
            })
        };
        let edge = (matches!(chrome.fill, FillOperands::Backdrop(_)) && chrome.border.is_some())
            .then(|| source(Half::Edge));
        Self {
            below: source(Half::Below),
            edge,
            above: source(Half::Above),
            chrome,
        }
    }

    /// The chrome `spec` wrapping `content` on `bounds`: the parts in
    /// `slot` when they already realize it — moved to `state` — otherwise
    /// fresh parts recorded at `state`, replacing them.
    pub(crate) fn ensure<'a>(
        slot: &'a mut Option<Self>,
        spec: &Chrome,
        content: &ChromeContent,
        bounds: Rect,
        state: InteractionState,
        env: &Environment,
        report: &nami::Binding<InteractionState>,
    ) -> &'a Self {
        match slot {
            Some(parts) if parts.chrome.realizes(spec, content, bounds, state) => {
                parts.chrome.sync(state);
            }
            _ => {
                *slot = Some(Self::new(LiveChrome::new(
                    spec.clone(),
                    content,
                    bounds,
                    state,
                    env,
                    report.clone(),
                )));
            }
        }
        slot.as_ref().expect("the chrome parts were just ensured")
    }

    pub(crate) const fn chrome(&self) -> &Rc<LiveChrome> {
        &self.chrome
    }

    /// Lists the chrome's layers beneath the widget's content in `ctx`:
    /// the surface, the backdrop material over it, the border, then the
    /// custom parts.
    pub(crate) fn push_below<E: crate::text::TextEngine>(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
    ) {
        renderer.push_live(ctx, &self.below);
        let chrome = &self.chrome;
        if let (FillOperands::Backdrop(effect), Fill::Backdrop(backdrop)) =
            (&chrome.fill, chrome.spec.surface().fill())
        {
            renderer.draw_context(ctx, |recorder| {
                recorder.backdrop_material(
                    chrome.shape.live(),
                    backdrop.shader(),
                    backdrop.capture(),
                    effect.live(),
                );
            });
        }
        if let Some(edge) = &self.edge {
            renderer.push_live(ctx, edge);
        }
        if !chrome.customs.is_empty() {
            renderer.draw_context(ctx, |recorder| {
                for custom in &chrome.customs {
                    custom.record(recorder, chrome.frame);
                }
            });
        }
        if let Some(slider) = &chrome.slider {
            renderer.draw_context(ctx, |recorder| {
                let frame = chrome.frame;
                let record = |recorder: &mut Recorder| {
                    for part in &slider.parts {
                        part.record(recorder, frame);
                    }
                };
                if slider.source.mirrored {
                    // The look draws left to right; a right-to-left layout
                    // mirrors it about the row.
                    recorder.transform(
                        Fixed(Affine::new([-1.0, 0.0, 0.0, 1.0, frame.x0 + frame.x1, 0.0])),
                        record,
                    );
                } else {
                    record(recorder);
                }
            });
        }
    }

    /// Lists the slider's value indicator bubble in `ctx`, at rest: the
    /// caller opens it in a scope that follows [`Self::indicator_operands`].
    pub(crate) fn push_indicator<E: crate::text::TextEngine>(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
    ) {
        let Some(indicator) = self.chrome.slider.as_ref().and_then(|slider| {
            slider
                .indicator
                .as_ref()
                .zip(slider.source.indicator.as_ref())
        }) else {
            return;
        };
        let (live, content) = indicator;
        renderer.draw_context(ctx, |recorder| {
            live.container.record(recorder, content.rest);
        });
    }

    /// Lists the chrome's interaction layers above the widget's content.
    pub(crate) fn push_above<E: crate::text::TextEngine>(
        &self,
        renderer: &mut HydrolysisRenderer<E>,
        ctx: RenderContext,
    ) {
        renderer.push_live(ctx, &self.above);
    }
}
