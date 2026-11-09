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
use kurbo::{Affine, Point, Rect};
use nami::{Computed, Signal};
use waterui_core::Environment;
use waterui_core::animation::Animation;
use waterui_core::interaction::InteractionState;

use crate::renderer::recording::{LoweredResources, SceneResources};
use crate::renderer::{HydrolysisRenderer, RenderContext};
use crate::theme::live::Operand;
use crate::theme::{
    Change, Chrome, Corners, Driven, Elevation, Fill, FocusIndicator, Inputs, InteractionMotion,
    MAX_PRESS_WAVES, Motion, RealizedCustom, Ripple,
};

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
        bounds: Rect,
        state: InteractionState,
        env: &Environment,
        report: nami::Binding<InteractionState>,
    ) -> Self {
        let frame = spec.frame(bounds);
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
        let wash = spec.press().get_wash().map(|wash| {
            let wash_color = wash.color.resolve(env);
            let center = frame.center();
            WashOperands {
                paint: Operand::new(wash_paint(center, wash.radius, wash_color.snapshot(), 0.0)),
                color: wash_color,
                radius: wash.radius,
                center: Cell::new(center),
            }
        });
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
                notify(context.metadata().try_get::<Animation>());
            }))
        };
        guards.push(watch_color(&this.layer_color));
        if let Some(ring) = &this.ring {
            guards.push(watch_color(&ring.color));
        }
        if let Some(wash) = &this.wash {
            guards.push(watch_color(&wash.color));
        }
        for custom in &this.customs {
            for driven in &custom.drivens {
                let weak: Weak<Self> = Rc::downgrade(this);
                let target = Rc::downgrade(driven);
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
    fn realizes(&self, spec: &Chrome, bounds: Rect) -> bool {
        self.spec.same(spec) && self.bounds == bounds
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
        let change = match retargeting {
            Retargeting::Interaction(old) => Change::Interaction {
                old,
                motion: self.spec.motion(),
            },
            Retargeting::Animated(animation) => Change::Animated(animation),
        };
        for custom in &self.customs {
            for driven in &custom.drivens {
                driven.retarget(&inputs, change);
            }
        }
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

    /// The chrome `spec` on `bounds`: the parts in `slot` when they already
    /// realize it — moved to `state` — otherwise fresh parts recorded at
    /// `state`, replacing them.
    pub(crate) fn ensure<'a>(
        slot: &'a mut Option<Self>,
        spec: &Chrome,
        bounds: Rect,
        state: InteractionState,
        env: &Environment,
        report: &nami::Binding<InteractionState>,
    ) -> &'a Self {
        match slot {
            Some(parts) if parts.chrome.realizes(spec, bounds) => {
                parts.chrome.sync(state);
            }
            _ => {
                *slot = Some(Self::new(LiveChrome::new(
                    spec.clone(),
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
