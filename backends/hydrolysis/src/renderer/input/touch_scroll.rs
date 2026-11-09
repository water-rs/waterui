//! Touch-drag scrolling: the slop claim, one-to-one tracking and the
//! release fling.
//!
//! A [`PointerKind::Touch`] press inside a scroll view still delivers its
//! press to the content under it. Once the contact moves farther than the
//! host's touch slop, the innermost enclosing scroll view able to scroll
//! along the movement's dominant axis claims the gesture — the content's
//! press is cancelled, never completed — and every later move scrolls the
//! view's [`ScrollHandle`] one to one. On release, a [`VelocityTracker`]
//! fit over the last samples starts a fling that decelerates with the
//! platform's deceleration model; a new touch down inside the scroll view
//! stops the fling where it is. The fling, and the overscroll effect it and
//! the drag leave behind, belong to that scroll view's [`ScrollMotion`].
//!
//! What an edge does is the physics family's [`Overscroll`]: clamped
//! edges stop the drag and the fling on them; rubber-band edges let a drag
//! past them move with resistance and spring a release or a fling past them
//! back onto them, the offset held unclamped meanwhile — rendered and
//! reported where it is, as `UIScrollView`'s `contentOffset` is; stretch
//! edges keep the offset in the extents and stretch the rendered content
//! instead — the drag carries the [`ScrollStretch`] and hands it to the
//! scroll view's motion, whose fling arms the absorb of an edge it reaches
//! and whose stretch relaxes on its own spring, past the fling's end.
//!
//! The gesture's parameters are the host's [`TouchScrollConfig`], pushed
//! into the renderer by value from `PlatformWindow::touch_scroll_config` —
//! no platform values are invented here.
use super::edge_stretch::{AxisStretch, Edge, EdgeImpact, ScrollStretch};
use super::scroll_physics::{DecayFling, SpringFling};
// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::platform::{EdgeStretch, FlingDeceleration, Overscroll, TouchScrollConfig};
use std::collections::VecDeque;
use waterui_layout::scroll::Axis;

/// The trailing window the release velocity is fit over — the span
/// Android's `VelocityTracker` weighs its motion samples against.
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
/// The most move samples a tracker keeps — far more than a vsync-paced
/// stream produces inside the window.
const VELOCITY_MAX_SAMPLES: usize = 32;

/// The in-flight touch-drag scroll gesture — one at a time, for the single
/// active pointer the hit-test state tracks.
#[derive(Default)]
pub enum TouchScrollGesture<E: crate::text::TextEngine> {
    /// No touch sequence is driving a scroll.
    #[default]
    Idle,
    /// A touch is down; the first move past the touch slop decides whether
    /// a scroll view claims the sequence.
    Pending(TouchScrollPending),
    /// A scroll view claimed the drag: every move scrolls it one to one.
    /// Boxed: the drag carries the view's stretch inline, many times the
    /// size of the other states.
    Dragging(Box<TouchScrollDrag<E>>),
}

/// A touch press still inside the slop, waiting on the claim check.
#[derive(Debug)]
pub struct TouchScrollPending {
    /// Where the contact went down, in window hit-test space — the claim
    /// tests displacement and the scroll candidates at this point.
    pub(crate) origin: kurbo::Point,
    /// The motion samples the release's fling is fit over.
    pub(crate) tracker: VelocityTracker,
    /// The motions of the scroll views under the touch-down point, which
    /// the touch stopped where they stand: a drag of one of them takes its
    /// motion over — the offset where it stands, a caught bounce included,
    /// and the stretch it holds — and every other one is let go again from
    /// rest, so an offset it held past an edge springs back and a stretch
    /// relaxes. At most one per scroll view.
    pub(crate) caught: Vec<ScrollMotion>,
}

/// A scroll view's claimed drag, scrolled one to one by later moves.
pub struct TouchScrollDrag<E: crate::text::TextEngine> {
    /// The claimed scroll view's offset handle.
    pub(crate) handle: crate::scroll::ScrollHandle,
    /// The claim the gesture took on the offset when the drag was
    /// recognised: every move writes through it, the release hands it to
    /// the fling, and only a newer claim — never a `rebind` for rows
    /// measured mid-drag — takes the offset from it.
    pub(crate) claim: crate::scroll::GestureClaim,
    /// The cell that registered the claimed scroll target — mark
    /// attribution for the drag's offset writes.
    pub(crate) owner: std::rc::Weak<crate::renderer::NodeCell<E>>,
    /// The point the last applied delta ended at, in window hit-test space.
    pub(crate) last: kurbo::Point,
    /// The motion samples the release's fling is fit over.
    pub(crate) tracker: VelocityTracker,
    /// What the claimed view's edges do to the drag, from the physics the
    /// claim was taken under.
    pub(crate) overscroll: Overscroll,
    /// The claimed view's stretch under stretch edges — taken over from
    /// the motion the touch caught on the same view, or at rest — and
    /// `None` under every other overscroll.
    pub(super) stretch: Option<ScrollStretch>,
}

impl<E: crate::text::TextEngine> TouchScrollDrag<E> {
    /// Moves the claimed offset by `(dx, dy)` — the finger's travel,
    /// negated — at `now`, and returns whether the offset or the stretch
    /// it shows changed: clamped edges stop it, rubber-band edges let it
    /// past with their resistance, stretch edges stop the offset and pull
    /// the content's stretch instead. Refused once a newer claim owns the
    /// offset.
    pub(crate) fn scroll_by(&mut self, dx: f64, dy: f64, now: Instant) -> bool {
        match self.overscroll {
            Overscroll::Clamped => self.handle.apply_gesture_delta(&self.claim, dx, dy),
            Overscroll::RubberBand(band) => {
                let metrics = self.handle.metrics();
                self.handle.apply_gesture_offset(
                    &self.claim,
                    Some(band.drag(metrics.offset_x, dx, metrics.max_x, metrics.viewport_width)),
                    Some(band.drag(metrics.offset_y, dy, metrics.max_y, metrics.viewport_height)),
                )
            }
            Overscroll::Stretch(_) => {
                // A newer claim owns the offset: the drag moves nothing,
                // its stretch included.
                if !self.handle.holds(&self.claim) {
                    return false;
                }
                let stretch = self
                    .stretch
                    .as_mut()
                    .expect("a drag under stretch edges carries its stretch");
                let metrics = self.handle.metrics();
                let before = stretch.vector();
                let axis = self.handle.axis();
                let x = if matches!(axis, Axis::Horizontal | Axis::All) {
                    stretch.x.drag(
                        metrics.offset_x,
                        dx,
                        metrics.max_x,
                        metrics.viewport_width,
                        now,
                    )
                } else {
                    metrics.offset_x
                };
                let y = if matches!(axis, Axis::Vertical | Axis::All) {
                    stretch.y.drag(
                        metrics.offset_y,
                        dy,
                        metrics.max_y,
                        metrics.viewport_height,
                        now,
                    )
                } else {
                    metrics.offset_y
                };
                let stretched = stretch.vector() != before;
                let moved = self.handle.apply_gesture_delta(
                    &self.claim,
                    x - metrics.offset_x,
                    y - metrics.offset_y,
                );
                moved || stretched
            }
        }
    }
}

/// The release velocity a fling starts from: a least-squares slope over
/// the most recent [`VELOCITY_WINDOW`] of move samples, the same shape
/// Android's `VelocityTracker` computes for a touch's `computeCurrentVelocity`.
#[derive(Debug)]
pub struct VelocityTracker {
    samples: VecDeque<(Instant, kurbo::Point)>,
}

impl VelocityTracker {
    /// Starts the track with the pointer-down sample.
    pub(crate) fn new(at: Instant, origin: kurbo::Point) -> Self {
        let mut samples = VecDeque::with_capacity(VELOCITY_MAX_SAMPLES);
        samples.push_back((at, origin));
        Self { samples }
    }

    /// Records a move sample.
    pub(crate) fn record(&mut self, at: Instant, point: kurbo::Point) {
        if self.samples.len() == VELOCITY_MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back((at, point));
    }

    /// The fitted pointer velocity at `now`, in logical units per second —
    /// the regression slope of the samples still inside the trailing
    /// window, or zero when fewer than two samples or no elapsed time
    /// support a slope.
    pub(crate) fn velocity(&self, now: Instant) -> kurbo::Vec2 {
        let cutoff = now.checked_sub(VELOCITY_WINDOW);
        let mut count = 0_usize;
        let mut mean_t = 0.0;
        let mut mean_x = 0.0;
        let mut mean_y = 0.0;
        for &(at, point) in &self.samples {
            if cutoff.is_some_and(|cutoff| at < cutoff) {
                continue;
            }
            let t = -now.saturating_duration_since(at).as_secs_f64();
            count += 1;
            mean_t += t;
            mean_x += point.x;
            mean_y += point.y;
        }
        if count < 2 {
            return kurbo::Vec2::ZERO;
        }
        let count = crate::num_cast::usize_as_f64(count);
        mean_t /= count;
        mean_x /= count;
        mean_y /= count;
        let mut numerator_x = 0.0;
        let mut numerator_y = 0.0;
        let mut denominator = 0.0;
        for &(at, point) in &self.samples {
            if cutoff.is_some_and(|cutoff| at < cutoff) {
                continue;
            }
            let dt = -now.saturating_duration_since(at).as_secs_f64() - mean_t;
            numerator_x = dt.mul_add(point.x - mean_x, numerator_x);
            numerator_y = dt.mul_add(point.y - mean_y, numerator_y);
            denominator = dt.mul_add(dt, denominator);
        }
        if denominator < f64::EPSILON {
            return kurbo::Vec2::ZERO;
        }
        kurbo::Vec2::new(numerator_x / denominator, numerator_y / denominator)
    }
}

/// What keeps one scroll view moving once no touch holds it: the fling that
/// drives its offset, and — under stretch edges — the stretch it shows.
///
/// Each scroll view owns its motion, as each `ScrollView` owns its
/// `OverScroller` and its `EdgeEffect`s and each `UIScrollView` its
/// deceleration and bounce: motions of different scroll views run side by
/// side, and nothing one of them does — a touch, a release, a fling —
/// reaches another. The two parts end independently. The fling is the
/// offset's writer and lives exactly as long as its claim: a newer claim
/// on the offset — the wheel, a programmatic scroll, the scrollbar — ends
/// it, and the edge it was still to reach is never absorbed. The stretch
/// is a draw-time effect, not an offset writer, so no claim ends it: it
/// relaxes under its own spring whoever moves the offset meanwhile, as an
/// `EdgeEffect` keeps receding through `ScrollView.scrollTo` and a mouse
/// wheel's `ACTION_SCROLL`. The rubber band has no separate effect — its
/// overscroll is the offset itself, held past the edge by the fling's
/// claim — so a newer claim takes it with the offset and clamps it back
/// into the extents.
#[derive(Debug)]
pub struct ScrollMotion {
    /// The scroll view's offset handle.
    handle: crate::scroll::ScrollHandle,
    /// The fling driving the offset; `None` once it settled or a newer
    /// claim took the offset while the stretch still relaxes.
    fling: Option<TouchFling>,
    /// The scroll view's stretch under stretch edges, `None` otherwise.
    stretch: Option<ScrollStretch>,
}

/// A fling running after the touch released: per-axis deceleration — and,
/// under rubber-band edges, the spring back onto an edge — driving its
/// [`ScrollMotion`]'s handle until it comes to rest inside the content
/// bounds, a new touch down stops it, or a newer claim takes the offset.
#[derive(Debug)]
struct TouchFling {
    /// The drag's claim, carried into the fling — the ownership record: it
    /// writes while it is still the newest claim, and the motion ends it
    /// once a newer one — a request, a jump, user input — took the offset,
    /// rather than overwriting its successor. A `rebind` — shifted by a
    /// membership anchor or for rows measured mid-gesture — never touches
    /// it.
    claim: crate::scroll::GestureClaim,
    /// What the edges do: clamped and stretch edges write through the
    /// clamping fling path, rubber-band edges through the unclamped one.
    overscroll: Overscroll,
    x: Option<AxisFling>,
    y: Option<AxisFling>,
}

/// One axis of a [`TouchFling`].
#[derive(Debug)]
enum AxisFling {
    /// Android `OverScroller`'s spline.
    Spline(SplineFling),
    /// `UIScrollView`'s decay, with its bounce at an edge it reaches.
    Decay(DecayFling),
    /// The return from a release past an edge, and — for a release moving
    /// toward the content — the fling it hands over to on the edge, from
    /// the moment it starts.
    Release {
        spring: SpringFling,
        then: Option<(Instant, Box<Self>)>,
    },
}

impl AxisFling {
    /// A fling under `physics` inside the extents `0..=extent`, from
    /// `start` at `velocity`, starting at `from` — `None` when it moves
    /// nothing.
    fn inside(
        physics: &crate::platform::ScrollPhysics,
        velocity: f64,
        start: f64,
        extent: f64,
        from: Instant,
    ) -> Option<Self> {
        let bounce = match physics.overscroll() {
            // An axis with nothing to scroll does not bounce — the band's
            // `drag` documents why.
            Overscroll::RubberBand(band) => (extent > 0.0).then_some(band.bounce()),
            Overscroll::Clamped | Overscroll::Stretch(_) => None,
        };
        match physics.deceleration() {
            FlingDeceleration::OverScrollerSpline(spline) => Some(SplineFling::new(
                velocity,
                start,
                extent,
                spline.physical_coeff(),
                spline.friction(),
                from,
            ))
            // A run whose clamped destination is already the offset —
            // released at the edge it headed toward — is no fling.
            .filter(|fling| (fling.final_offset - fling.start).abs() > SCROLL_END_EPSILON)
            .map(Self::Spline),
            FlingDeceleration::ExponentialDecay(decay) => {
                DecayFling::new(decay, velocity, start, extent, bounce, from).map(Self::Decay)
            }
        }
    }

    /// The axis's offset at `now` and whether it still moves.
    fn position(&self, now: Instant) -> (f64, bool) {
        match self {
            Self::Spline(fling) => fling.position(now),
            Self::Decay(fling) => fling.position(now),
            Self::Release { spring, then } => match then {
                Some((from, fling)) if now >= *from => fling.position(now),
                _ => spring.position(now),
            },
        }
    }

    /// Where a fling stops on an edge: when it reaches it, the edge — the
    /// one it heads toward — and its speed there. `None` for a fling that
    /// comes to rest inside, and for the rubber band's runs, which never
    /// meet a stretch.
    fn edge_impact(&self) -> Option<EdgeImpact> {
        let (at, velocity) = match self {
            Self::Spline(fling) => fling.edge_arrival()?,
            Self::Decay(fling) => fling.edge_arrival()?,
            Self::Release { .. } => return None,
        };
        Some(EdgeImpact {
            at,
            edge: if velocity < 0.0 {
                Edge::Leading
            } else {
                Edge::Trailing
            },
            speed: velocity.abs(),
        })
    }
}

impl TouchFling {
    /// Starts a fling on `handle` from the fitted pointer velocity, in
    /// logical units per second, on the released drag's `claim`: the
    /// offset's velocity is the finger's negated, clamped to the host's
    /// maximum. Under rubber-band edges an axis released past an edge
    /// returns onto it whatever its speed, starting at that velocity — the
    /// finger's, as `UIScrollView` reports it — and, moving toward the
    /// content, flings on into it from the edge; otherwise an axis under
    /// the minimum — or already at the edge it heads toward, under clamped
    /// and stretch edges — does not fling.
    ///
    /// Under stretch edges `stretch` is the drag's, released as
    /// `ScrollView`'s `ACTION_UP` releases it
    /// (`flingWithNestedDispatch`, then `endDrag`): a release that cannot
    /// fling — it pushes against the edge it sits on — is absorbed by the
    /// edge showing a stretch, every other pulled edge relaxes from rest,
    /// and a fling that starts toward an edge it reaches arms the absorb
    /// there. `None` when no axis flings or a newer claim already owns the
    /// offset; the stretch is released either way.
    fn start(
        handle: &crate::scroll::ScrollHandle,
        claim: crate::scroll::GestureClaim,
        finger_velocity: kurbo::Vec2,
        config: &TouchScrollConfig,
        mut stretch: Option<&mut ScrollStretch>,
        at: Instant,
    ) -> Option<Self> {
        let metrics = handle.metrics();
        let max = f64::from(config.max_fling_velocity());
        let min = f64::from(config.min_fling_velocity());
        let physics = config.physics();
        assert_eq!(
            matches!(physics.overscroll(), Overscroll::Stretch(_)),
            stretch.is_some(),
            "a release carries a stretch exactly under stretch edges"
        );
        // Each axis's fling, and the edge it reaches under stretch edges —
        // armed only once the fling has actually started on the claim.
        let axis_fling = |velocity: f64,
                          start: f64,
                          extent: f64,
                          stretch: Option<(&mut AxisStretch, &EdgeStretch)>|
         -> (Option<AxisFling>, Option<EdgeImpact>) {
            // The content offset moves opposite the finger: a finger flung
            // up pushes the offset down into the content.
            let offset_velocity = (-velocity).clamp(-max, max);
            let inside = |velocity: f64, start: f64, from: Instant| {
                AxisFling::inside(&physics, velocity, start, extent, from)
            };
            if let Some((axis, config)) = stretch {
                let fling = axis
                    .release(start, offset_velocity, extent, min, config, at)
                    .then(|| inside(offset_velocity, start, at))
                    .flatten();
                // `OVER_SCROLL_IF_CONTENT_SCROLLS`: only an axis with
                // something to scroll stretches.
                let impact = fling
                    .as_ref()
                    .and_then(AxisFling::edge_impact)
                    .filter(|_| extent > 0.0);
                return (fling, impact);
            }
            if let Overscroll::RubberBand(band) = physics.overscroll()
                && !(0.0..=extent).contains(&start)
            {
                let edge = start.clamp(0.0, extent);
                let overscroll = start - edge;
                let then = band
                    .release()
                    .handover(overscroll, offset_velocity)
                    .and_then(|handover| {
                        let from = at + Duration::from_secs_f64(handover.seconds);
                        inside(handover.velocity, edge, from).map(|fling| (from, Box::new(fling)))
                    });
                let release = AxisFling::Release {
                    spring: SpringFling::new(edge, overscroll, offset_velocity, band.release(), at),
                    then,
                };
                return (Some(release), None);
            }
            if offset_velocity.abs() < min {
                return (None, None);
            }
            (inside(offset_velocity, start, at), None)
        };
        let (stretch_x, stretch_y) = match stretch.as_deref_mut() {
            Some(ScrollStretch { config, x, y }) => (Some((x, &*config)), Some((y, &*config))),
            None => (None, None),
        };
        let (x, impact_x) = if matches!(handle.axis(), Axis::Horizontal | Axis::All) {
            axis_fling(
                finger_velocity.x,
                metrics.offset_x,
                metrics.max_x,
                stretch_x,
            )
        } else {
            (None, None)
        };
        let (y, impact_y) = if matches!(handle.axis(), Axis::Vertical | Axis::All) {
            axis_fling(
                finger_velocity.y,
                metrics.offset_y,
                metrics.max_y,
                stretch_y,
            )
        } else {
            (None, None)
        };
        if (x.is_none() && y.is_none()) || !handle.begin_fling(&claim) {
            return None;
        }
        if let Some(stretch) = stretch {
            if let Some(impact) = impact_x {
                stretch.x.expect_impact(impact);
            }
            if let Some(impact) = impact_y {
                stretch.y.expect_impact(impact);
            }
        }
        Some(Self {
            claim,
            overscroll: physics.overscroll(),
            x,
            y,
        })
    }

    /// Whether the fling still owns `handle`'s offset — no newer claim has
    /// taken it.
    fn holds(&self, handle: &crate::scroll::ScrollHandle) -> bool {
        handle.holds(&self.claim)
    }

    /// Advances the fling to `now` and applies its offset through `handle`,
    /// the scroll view whose claim it holds — the caller asked
    /// [`Self::holds`] first. A fling that comes to rest ends its gesture.
    fn tick(&self, handle: &crate::scroll::ScrollHandle, now: Instant) -> TouchFlingTick {
        // Only the axes with a run are sampled: an axis without one keeps
        // its offset, so a membership shift is never applied to it twice.
        let x = self.x.as_ref().map(|fling| fling.position(now));
        let y = self.y.as_ref().map(|fling| fling.position(now));
        let (offset_x, offset_y) = (x.map(|(offset, _)| offset), y.map(|(offset, _)| offset));
        let changed = match self.overscroll {
            Overscroll::Clamped | Overscroll::Stretch(_) => {
                handle.apply_fling_offset(&self.claim, offset_x, offset_y)
            }
            Overscroll::RubberBand(_) => {
                handle.apply_overscroll_fling_offset(&self.claim, offset_x, offset_y)
            }
        };
        let active = |axis: Option<(f64, bool)>| axis.is_some_and(|(_, active)| active);
        let running = active(x) || active(y);
        // A fling that ends ends its gesture: an offset it still holds past
        // an edge — the content changed under a bounce — clamps into the
        // extents live now.
        let ended = !running && handle.end_gesture(&self.claim);
        TouchFlingTick {
            changed: changed || ended,
            running,
        }
    }
}

impl ScrollMotion {
    /// Releases the claim `handle` and `claim` name — a drag let go, or a
    /// fling a touch caught and let go again — with the `stretch` it
    /// carries: the fling the release earns under `config`, or, when it
    /// earns none, the gesture's end, which clamps an offset the claim held
    /// past an edge; and the released stretch, which relaxes on its own.
    pub(super) fn release(
        handle: &crate::scroll::ScrollHandle,
        claim: &crate::scroll::GestureClaim,
        finger_velocity: kurbo::Vec2,
        config: &TouchScrollConfig,
        mut stretch: Option<ScrollStretch>,
        at: Instant,
    ) -> TouchRelease {
        let fling = TouchFling::start(
            handle,
            claim.clone(),
            finger_velocity,
            config,
            stretch.as_mut(),
            at,
        );
        let clamped = fling.is_none() && handle.end_gesture(claim);
        let stretching = stretch.as_ref().is_some_and(ScrollStretch::is_active);
        TouchRelease {
            motion: (fling.is_some() || stretching).then(|| Self {
                handle: handle.clone(),
                fling,
                stretch,
            }),
            clamped,
        }
    }

    /// Lets this caught motion go again from rest at `at`: a fling that
    /// still holds its claim is released as a still drag would be — see
    /// [`Self::release`] — and a stretch whose fling is already gone
    /// relaxes from where the touch held it.
    pub(super) fn let_go(self, config: &TouchScrollConfig, at: Instant) -> TouchRelease {
        if let Some(fling) = self.fling {
            return Self::release(
                &self.handle,
                &fling.claim,
                kurbo::Vec2::ZERO,
                config,
                self.stretch,
                at,
            );
        }
        let mut stretch = self.stretch;
        if let Some(stretch) = &mut stretch {
            stretch.release(at);
        }
        let handle = self.handle;
        TouchRelease {
            motion: stretch
                .filter(ScrollStretch::is_active)
                .map(|stretch| Self {
                    handle,
                    fling: None,
                    stretch: Some(stretch),
                }),
            clamped: false,
        }
    }

    /// A touch down caught this motion: its fling stops where it is — one
    /// a newer claim already superseded is gone — and its stretch holds
    /// where it is, as `ScrollView`'s `ACTION_DOWN` aborts the scroller and
    /// holds an unfinished edge.
    pub(crate) fn catch(&mut self) {
        if self
            .fling
            .as_ref()
            .is_some_and(|fling| !fling.holds(&self.handle))
        {
            self.fling = None;
        }
        if let Some(stretch) = &mut self.stretch {
            stretch.catch();
        }
    }

    /// Takes the stretch out of this caught motion, for the drag of the
    /// same scroll view that takes the motion over.
    pub(super) fn into_stretch(self) -> Option<ScrollStretch> {
        self.stretch
    }

    /// The scroll view's offset handle — the pump needs it to attribute
    /// the tick's write to the `ScrollTarget` owner.
    pub(crate) const fn handle(&self) -> &crate::scroll::ScrollHandle {
        &self.handle
    }

    /// The stretch the scroll view shows.
    pub(super) const fn stretch(&self) -> Option<&ScrollStretch> {
        self.stretch.as_ref()
    }

    /// Advances the motion to `now`: the fling writes its offset while it
    /// holds its claim, and the stretch steps whoever owns the offset.
    pub(crate) fn tick(&mut self, now: Instant) -> TouchFlingTick {
        let mut changed = false;
        if let Some(fling) = &self.fling {
            if fling.holds(&self.handle) {
                let tick = fling.tick(&self.handle, now);
                changed = tick.changed;
                if !tick.running {
                    self.fling = None;
                }
            } else {
                // A programmatic request — jump or animated — or any user
                // input claimed the offset past the fling's claim: the
                // fling ends on the spot instead of writing over whatever
                // replaced it, and the edge it was still to reach is never
                // reached. Ownership is asked, never inferred from a write
                // that moved nothing — the tick on the release's own frame
                // samples the release offset itself.
                self.fling = None;
                if let Some(stretch) = &mut self.stretch {
                    stretch.cancel_impacts();
                }
            }
        }
        let (stretched, stretching) = self.stretch.as_mut().map_or((false, false), |stretch| {
            let metrics = self.handle.metrics();
            let stretched = stretch.step(now, metrics.viewport_width, metrics.viewport_height);
            (stretched, stretch.is_active())
        });
        TouchFlingTick {
            changed: changed || stretched,
            running: self.fling.is_some() || stretching,
        }
    }
}

/// What a released touch claim turned into — [`ScrollMotion::release`].
#[derive(Debug)]
pub struct TouchRelease {
    /// The motion the release leaves running on its scroll view — a fling,
    /// a relaxing stretch, or both — or `None` when nothing moves.
    pub(crate) motion: Option<ScrollMotion>,
    /// The release earned no fling, and ending its gesture clamped an
    /// offset held past an edge back into the extents — a frame must show
    /// it.
    pub(crate) clamped: bool,
}

/// The outcome of one [`ScrollMotion::tick`].
#[derive(Debug)]
pub struct TouchFlingTick {
    /// The motion wrote a new offset or stretch this tick — a frame must
    /// present it, including the tick it settles on.
    pub(crate) changed: bool,
    /// The motion still animates — keep it alive and keep pumping frames.
    pub(crate) running: bool,
}

/// One axis of an [`OverScroller`]-model fling: the spline deceleration
/// from AOSP `frameworks/base/core/java/android/widget/OverScroller.java`
/// (`SplineScroller`, SPLINE state) — a cubic-Bézier distance curve over a
/// fixed sample table, with `OverScroller`'s own edge/overscroll states out
/// of scope: the run ends on the clamped edge.
#[derive(Debug)]
struct SplineFling {
    started_at: Instant,
    /// The run's length — `mDuration`: `getSplineFlingDuration`'s value,
    /// shrunk by `adjustDuration` when the unclamped spline overshoots the
    /// content edge.
    duration: Duration,
    /// `mSplineDuration` — the unadjusted duration the position curve's
    /// time fraction divides by.
    spline_duration: Duration,
    /// Offset at release — `mStart`.
    start: f64,
    /// The unclamped signed spline distance the position curve travels —
    /// `mSplineDistance`. The run ends at the clamped `final`, which the
    /// shortened `duration` places exactly on the edge.
    spline_distance: f64,
    /// The clamped offset the run ends at — `mFinal` after the edge clamp.
    final_offset: f64,
    /// Whether the edge clamp cut the run short: it ends on an edge with
    /// speed left, rather than at rest.
    reaches_edge: bool,
}

impl SplineFling {
    /// `SplineScroller.fling`: the spline's duration and distance derive
    /// from the clamped velocity through the platform's physical
    /// coefficient and friction — `getSplineFlingDuration`,
    /// `getSplineFlingDistance` and the `adjustDuration` the edge clamp
    /// applies. `velocity` is the offset's signed speed in logical units
    /// per second, `extent` its maximum.
    fn new(
        velocity: f64,
        start: f64,
        extent: f64,
        physical_coeff: f64,
        friction: f64,
        at: Instant,
    ) -> Self {
        // AOSP `getSplineDeceleration` — `DECELERATION_RATE = ln(0.78)/ln(0.9)`.
        let deceleration_rate = 0.78_f64.log(0.9);
        let deceleration_minus_one = deceleration_rate - 1.0;
        let spline_deceleration = (INFLEXION * velocity.abs() / (friction * physical_coeff)).ln();
        let spline_duration =
            Duration::from_secs_f64((spline_deceleration / deceleration_minus_one).exp());
        let mut duration = spline_duration;
        // `getSplineFlingDistance` returns the run's magnitude; the sign
        // applies separately (`mSplineDistance = totalDistance * signum`).
        let magnitude = friction
            * physical_coeff
            * (deceleration_rate / deceleration_minus_one * spline_deceleration).exp();
        let spline_distance = magnitude.copysign(velocity);
        let unclamped_final = start + spline_distance;
        let final_offset = unclamped_final.clamp(0.0, extent);
        let reaches_edge = !(0.0..=extent).contains(&unclamped_final);
        if reaches_edge {
            // `adjustDuration`: a clamped run ends where the edge is, at
            // the spline-time fraction of the clamped distance fraction —
            // not by crawling through the unclamped spline's full span.
            let edge_fraction = ((final_offset - start) / spline_distance).abs();
            duration = spline_duration.mul_f64(spline_time(edge_fraction));
        }
        Self {
            started_at: at,
            duration,
            spline_duration,
            start,
            spline_distance,
            final_offset,
            reaches_edge,
        }
    }

    /// The offset's velocity `elapsed` into the run, in logical units per
    /// second — `SplineOverScroller.update`'s SPLINE `mCurrVelocity`: the
    /// `SPLINE_POSITION` slope over the sample `t` falls in, times
    /// `mSplineDistance / mSplineDuration`.
    fn velocity_at(&self, elapsed: Duration) -> f64 {
        let spline_duration = self.spline_duration.as_secs_f64();
        let samples = crate::num_cast::usize_as_f64(SPLINE_SAMPLES);
        let index =
            crate::num_cast::f64_as_usize(elapsed.as_secs_f64() / spline_duration * samples);
        if index >= SPLINE_SAMPLES {
            return 0.0;
        }
        let slope = (SPLINE_POSITION[index + 1] - SPLINE_POSITION[index]) * samples;
        slope * self.spline_distance / spline_duration
    }

    /// When a run the edge clamp cut short reaches its edge, and the
    /// offset's velocity there — the `getCurrVelocity` a container hands
    /// `EdgeEffect.onAbsorb` when the fling hits the edge (`ScrollView`'s
    /// `computeScroll`). `None` for a run that comes to rest inside.
    fn edge_arrival(&self) -> Option<(Instant, f64)> {
        self.reaches_edge.then(|| {
            (
                self.started_at + self.duration,
                self.velocity_at(self.duration),
            )
        })
    }

    /// The axis's offset at `now` and whether the run still animates —
    /// `SplineOverScroller.update`'s SPLINE state: the sampled
    /// `SPLINE_POSITION` fraction of the unclamped spline distance, until
    /// `mDuration` (adjusted for the edge) reports the run finished at the
    /// clamped `mFinal`.
    fn position(&self, now: Instant) -> (f64, bool) {
        let elapsed = now.saturating_duration_since(self.started_at);
        if elapsed >= self.duration {
            return (self.final_offset, false);
        }
        let t = elapsed.as_secs_f64() / self.spline_duration.as_secs_f64();
        (
            spline_position(t).mul_add(self.spline_distance, self.start),
            true,
        )
    }
}

/// `SplineScroller`'s `INFLEXION` — the tension curves cross at
/// (INFLEXION, 1).
const INFLEXION: f64 = 0.35;
const START_TENSION: f64 = 0.5;
const END_TENSION: f64 = 1.0;
const P1: f64 = START_TENSION * INFLEXION;
const P2: f64 = 1.0 - END_TENSION * (1.0 - INFLEXION);
/// `SPLINE_POSITION`/`SPLINE_TIME` sample count.
const SPLINE_SAMPLES: usize = 100;
/// Remaining fling distance under which a started run is dropped.
const SCROLL_END_EPSILON: f64 = 0.000_1;

/// `SplineScroller`'s `SPLINE_POSITION` and `SPLINE_TIME` tables — the
/// distance fraction at each time fraction, and its inverse — built by the
/// same bisection the static initializer in `OverScroller.java` runs.
const SPLINE_POSITION: [f64; SPLINE_SAMPLES + 1] = build_spline_tables().0;
const SPLINE_TIME: [f64; SPLINE_SAMPLES + 1] = build_spline_tables().1;

const fn build_spline_tables() -> ([f64; SPLINE_SAMPLES + 1], [f64; SPLINE_SAMPLES + 1]) {
    let mut position = [0.0; SPLINE_SAMPLES + 1];
    let mut time = [0.0; SPLINE_SAMPLES + 1];
    let mut x_min = 0.0;
    let mut y_min = 0.0;
    let mut i = 0;
    while i < SPLINE_SAMPLES {
        let alpha =
            crate::num_cast::usize_as_f64(i) / crate::num_cast::usize_as_f64(SPLINE_SAMPLES);
        let mut x_max = 1.0;
        let mut x;
        loop {
            x = x_min + (x_max - x_min) / 2.0;
            let coef = 3.0 * x * (1.0 - x);
            let tx = coef * ((1.0 - x) * P1 + x * P2) + x * x * x;
            if (if tx < alpha { alpha - tx } else { tx - alpha }) < 1e-5 {
                break;
            }
            if tx > alpha {
                x_max = x;
            } else {
                x_min = x;
            }
        }
        let coef = 3.0 * x * (1.0 - x);
        position[i] = coef * ((1.0 - x) * START_TENSION + x) + x * x * x;
        let mut y_max = 1.0;
        let mut y;
        loop {
            y = y_min + (y_max - y_min) / 2.0;
            let coef = 3.0 * y * (1.0 - y);
            let dy = coef * ((1.0 - y) * START_TENSION + y) + y * y * y;
            if (if dy < alpha { alpha - dy } else { dy - alpha }) < 1e-5 {
                break;
            }
            if dy > alpha {
                y_max = y;
            } else {
                y_min = y;
            }
        }
        let coef = 3.0 * y * (1.0 - y);
        time[i] = coef * ((1.0 - y) * P1 + y * P2) + y * y * y;
        i += 1;
    }
    position[SPLINE_SAMPLES] = 1.0;
    time[SPLINE_SAMPLES] = 1.0;
    (position, time)
}

/// `SplineScroller.update`'s `distanceCoef`: the `SPLINE_POSITION` value at
/// `t` with the same linear interpolation between adjacent samples.
fn spline_position(t: f64) -> f64 {
    let scaled = t * crate::num_cast::usize_as_f64(SPLINE_SAMPLES);
    let index = crate::num_cast::f64_as_usize(scaled);
    if index >= SPLINE_SAMPLES {
        return 1.0;
    }
    let low = SPLINE_POSITION[index];
    let high = SPLINE_POSITION[index + 1];
    (scaled - crate::num_cast::usize_as_f64(index)).mul_add(high - low, low)
}

/// `SplineScroller.adjustDuration`'s `timeCoef`: the `SPLINE_TIME` value at
/// the clamped distance fraction `x`, interpolated the same way.
fn spline_time(x: f64) -> f64 {
    let scaled = x * crate::num_cast::usize_as_f64(SPLINE_SAMPLES);
    let index = crate::num_cast::f64_as_usize(scaled);
    if index >= SPLINE_SAMPLES {
        return 1.0;
    }
    let low = SPLINE_TIME[index];
    let high = SPLINE_TIME[index + 1];
    (scaled - crate::num_cast::usize_as_f64(index)).mul_add(high - low, low)
}

impl<E: crate::text::TextEngine> core::fmt::Debug for TouchScrollDrag<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TouchScrollDrag")
            .field("handle", &self.handle)
            .field("claim", &self.claim)
            .field("owner", &self.owner)
            .field("last", &self.last)
            .field("tracker", &self.tracker)
            .field("overscroll", &self.overscroll)
            .field("stretch", &self.stretch)
            .finish()
    }
}

impl<E: crate::text::TextEngine> core::fmt::Debug for TouchScrollGesture<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Idle => f.write_str("Idle"),
            Self::Pending(v) => f.debug_tuple("Pending").field(v).finish(),
            Self::Dragging(v) => f.debug_tuple("Dragging").field(v).finish(),
        }
    }
}
