//! Scroll offset/viewport math owned by each semantic scroll view.
//!
//! A [`ScrollHandle`] retains one scroll view's state directly. Rebinding the
//! same handle after layout preserves its offset; a layout change advances its
//! generation so input closures captured by an earlier frame become inert.
//! There is deliberately no renderer slot registry or body-order identity.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use nami::Binding;
use waterui_core::animation::Animation;
use waterui_core::layout::Point;
use waterui_layout::scroll::Axis;

use crate::time::Instant;

const SCROLL_EPSILON: f64 = 0.000_01;
/// Logical pixels one wheel/accessibility "line" scrolls.
pub const SCROLL_LINE_STEP: f64 = 40.0;
/// Rows an animated programmatic list scroll glides over.
///
/// A target further than this from the first visible row is jumped to
/// within this many rows first, and only that final stretch animates — the
/// motion stays legible and never drags the list through every row in
/// between.
pub const ANIMATED_ROW_SCROLL_APPROACH: usize = 100;

/// The row an animated list scroll toward `target` first jumps to.
///
/// `current` is the row at the viewport's top. A `target` further than
/// [`ANIMATED_ROW_SCROLL_APPROACH`] rows from it answers the row that many
/// rows short of `target` on `current`'s side — where the unanimated jump
/// lands before the animation takes over; a nearer `target` answers `None`
/// and animates the whole way.
#[must_use]
pub const fn animated_row_scroll_approach(current: usize, target: usize) -> Option<usize> {
    if target.abs_diff(current) <= ANIMATED_ROW_SCROLL_APPROACH {
        return None;
    }
    Some(if target > current {
        target - ANIMATED_ROW_SCROLL_APPROACH
    } else {
        target + ANIMATED_ROW_SCROLL_APPROACH
    })
}

/// Time constant (seconds) of the exponential approach that eases the offset
/// toward a smooth-scroll target: ~63% of the remaining gap per τ, visually
/// settled (>95%) after ~3τ ≈ 180ms — the smooth-wheel feel of browsers and
/// native lists instead of an instant 40px teleport per tick.
const SMOOTH_SCROLL_TAU: f64 = 0.06;
/// Remaining gap below which a smoothed scroll snaps to its target and the
/// animation ends.
const SMOOTH_SCROLL_SETTLE_EPSILON: f64 = 0.1;

/// Cloneable reference to one scroll view's offset state, valid for the
/// layout generation it was bound against.
///
/// Input closures (wheel/trackpad handlers) capture a handle at dispatch
/// time; if the scroll view's layout changed in a later rebuild, the stale
/// handle's generation no longer matches and its input is dropped.
#[derive(Clone, Debug)]
pub struct ScrollHandle {
    state: Rc<RefCell<ScrollState>>,
    generation: u64,
}

/// Snapshot of one scroll view's offsets and extents, in f64 logical pixels.
#[derive(Debug, Clone, Copy)]
pub struct ScrollMetrics {
    /// Current horizontal offset, clamped to `0.0..=max_x`.
    pub offset_x: f64,
    /// Current vertical offset, clamped to `0.0..=max_y`.
    pub offset_y: f64,
    /// Maximum horizontal offset: `(content_width - viewport_width).max(0.0)`.
    pub max_x: f64,
    /// Maximum vertical offset: `(content_height - viewport_height).max(0.0)`.
    pub max_y: f64,
    /// Width of the visible viewport.
    pub viewport_width: f64,
    /// Height of the visible viewport.
    pub viewport_height: f64,
    /// Total width of the scrollable content.
    pub content_width: f64,
    /// Total height of the scrollable content.
    pub content_height: f64,
}

/// Identifies one programmatic scroll animation armed on a [`ScrollHandle`].
///
/// [`ScrollHandle::scroll_to_animated`] hands it to the caller that armed
/// the run; [`ScrollHandle::scroll_run_outcome`] answers how it ended and
/// [`ScrollHandle::retarget_animated_scroll`] steers only that run, so a
/// requester can never take over a different owner's animation on the same
/// scroll surface.
///
/// The token carries the issuing scroll view's identity: tokens are minted
/// from the state's own claim counter, so two scroll views hand out
/// colliding values — a run queried or retargeted on another view's handle
/// is a programming error and panics, as is a token beyond the claim
/// counter.
#[derive(Clone, Debug)]
pub struct ScrollRun {
    /// The issuing scroll view's state: the `Weak` pins its allocation, so
    /// the address cannot be recycled into a false match — a run minted by
    /// another scroll view always fails the handle assert.
    state: Weak<RefCell<ScrollState>>,
    /// The claim-counter value the run took at arming.
    token: u64,
}

/// How a [`ScrollRun`] ended — or whether it still drives the offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollRunOutcome {
    /// The run is still animating.
    Running,
    /// The run completed and landed on its target.
    Landed,
    /// The run ended early: user input claimed the offset (pixel deltas,
    /// line deltas, touch drags, the scrollbar's drag, an accessibility
    /// scroll-into-view), a jump landed, or a newer programmatic request
    /// replaced it. Also reported for a run whose single outcome slot a
    /// later run already took, and for a recorded `Landed` a later user
    /// scroll ended — the request is spent once the user takes the offset.
    /// A token beyond the claim counter is a programming error and panics
    /// instead of answering here.
    Interrupted,
}

/// The claim a touch gesture holds on the scroll offset.
///
/// [`ScrollHandle::begin_gesture`] mints it when a touch drag is
/// recognised; the drag moves the offset through
/// [`ScrollHandle::apply_gesture_delta`], and the release hands the same
/// claim to its fling — [`ScrollHandle::begin_fling`], then
/// [`ScrollHandle::apply_fling_offset`] each tick. Any newer claim — a new
/// touch, the wheel, the scrollbar, accessibility, a programmatic request,
/// keyboard clearance — owns the offset instead: the gesture's refused
/// writes leave its successor's offset alone, and a refused fling write
/// ends the fling. A [`ScrollHandle::rebind`] never revokes it: neither an
/// extent change from rows measured mid-gesture nor a membership anchor's
/// coordinate shift claims the offset, so the gesture's handle going stale
/// does not stop it. Like [`ScrollRun`], the claim carries the issuing
/// scroll view's identity — a `Weak` pinning its state's allocation — and
/// applying it on another view's handle panics.
#[derive(Clone, Debug)]
pub struct GestureClaim {
    /// The issuing scroll view's state, kept alive enough that its address
    /// can never be recycled into a false match.
    state: Weak<RefCell<ScrollState>>,
    /// The claim counter's value at `begin_gesture`: the gesture owns the
    /// offset exactly while it is still the newest claim.
    offset_epoch: u64,
}

/// One in-flight programmatic scroll animation.
///
/// Arming records the current offset, the clamped target, the request's
/// [`Animation`] and the frame instant it was armed at;
/// [`ScrollState::advance_programmatic_scroll`] samples
/// [`Animation::progress`] for the elapsed time each tick and lands exactly on
/// the target when the animation completes. It is exclusive with the wheel
/// glide: whichever is armed last clears the other. This is a local tween
/// rather than core's `AnimationTrack` because a track cannot retarget a live
/// run without restarting its clock.
#[derive(Debug)]
struct ScrollAnimation {
    /// The token [`ScrollHandle::scroll_to_animated`] handed the armer.
    token: u64,
    animation: Animation,
    /// The frame instant the request was armed at, so motion begins on the
    /// very next tick instead of spending a frame starting the clock.
    started: Instant,
    from_x: f64,
    from_y: f64,
    target_x: f64,
    target_y: f64,
}

#[derive(Debug)]
struct ScrollState {
    generation: u64,
    axis: Axis,
    viewport_width: f64,
    viewport_height: f64,
    content_width: f64,
    content_height: f64,
    offset_x: f64,
    offset_y: f64,
    /// Smooth-scroll destination per axis. Discrete wheel ticks (line deltas)
    /// retarget these instead of moving the offset directly;
    /// [`ScrollState::tick_smooth_scroll`] then eases the offset toward them
    /// frame by frame. Trackpad pixel deltas and [`ScrollState::scroll_to`]
    /// cancel them — direct manipulation (with the OS's own momentum stream)
    /// always wins.
    smooth_target_x: Option<f64>,
    smooth_target_y: Option<f64>,
    /// Last smooth-scroll tick, for a frame-rate-independent blend factor.
    smooth_last_tick: Option<Instant>,
    /// The programmatic scroll animation currently driving the offset, armed
    /// by [`ScrollState::scroll_to_animated`] and cancelled by a jump, user
    /// deltas/drags, or a wheel glide.
    programmatic: Option<ScrollAnimation>,
    /// The claim counter the offset's owners draw from: a programmatic run
    /// takes the next value as its token, a touch gesture captures it as its
    /// claim, and every request, jump or user delta bumps it — so a holder
    /// is the offset's owner exactly while no newer claim exists.
    offset_epoch: u64,
    /// The outcome of the most recently ended programmatic run, remembered
    /// until another run ends so the armer can still ask
    /// [`ScrollHandle::scroll_run_outcome`] about it after it is gone. The
    /// single slot reports an earlier run as `Interrupted` once a newer one
    /// has ended.
    last_run_outcome: Option<(u64, ScrollRunOutcome)>,
    /// The coordinate shift [`ScrollHandle::rebind`] has applied since the
    /// current fling began — added to its sampled positions so the fling's
    /// origin moves with the content. Reset by [`ScrollHandle::begin_gesture`]
    /// and [`ScrollHandle::begin_fling`], never by a claim: a refused fling
    /// writes nothing anyway.
    fling_shift_x: f64,
    fling_shift_y: f64,
    /// Binding a `ScrollView::report_offset` connected to this scroll view.
    /// Written — never read — whenever the content offset changes, every
    /// frame of a smooth glide included; `reported_offset` keeps the writes
    /// on-change-only.
    offset_report: Option<Binding<Point>>,
    /// Offset last queued for `offset_report`.
    reported_offset: Option<(f64, f64)>,
    /// Offset queued by [`ScrollState::report_offset`], written once the
    /// state borrow is released so a binding watcher may re-enter the state.
    pending_report: Option<Point>,
}

impl ScrollHandle {
    /// Creates the state owned by one semantic scroll view.
    ///
    /// `offset_report` is the binding a `ScrollView::report_offset` connected
    /// to this scroll view, or `None` when the view reports nothing — every
    /// handle must decide at birth, so no caller can forget to attach it.
    #[must_use]
    pub fn new(
        axis: Axis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
        offset_report: Option<Binding<Point>>,
    ) -> Self {
        let handle = Self {
            state: Rc::new(RefCell::new(ScrollState::new(
                axis,
                viewport_width,
                viewport_height,
                content_width,
                content_height,
            ))),
            generation: 1,
        };
        handle.set_offset_report(offset_report);
        handle
    }

    /// Rebinds this scroll view to its latest layout and returns the handle
    /// generation that input registered for the current frame must capture.
    ///
    /// `shift` first translates the coordinate system on the scrolled axes —
    /// a list's membership anchor passes how far the content under the
    /// viewport moved — without claiming the offset: the offset, the
    /// wheel-glide targets, a live run's origin and destination and the
    /// fling's origin all move together, so whatever owned the offset keeps
    /// owning it. Everything then clamps to the new extents once. Pass
    /// `(0.0, 0.0)` when the content did not move.
    #[must_use]
    pub fn rebind(
        &mut self,
        axis: Axis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
        shift: (f64, f64),
    ) -> Self {
        self.generation = self.state.borrow_mut().prepare_generation(
            axis,
            viewport_width,
            viewport_height,
            content_width,
            content_height,
            shift,
        );
        self.flush_offset_report();
        self.clone()
    }

    /// Connects the binding `ScrollView::report_offset` produced for this
    /// scroll view. The current content offset is written into it at once
    /// and then on every change — each frame of a smooth glide included.
    /// `None` disconnects it. The binding is written, never read.
    pub fn set_offset_report(&self, report: Option<Binding<Point>>) {
        {
            let mut state = self.state.borrow_mut();
            state.offset_report = report;
            state.reported_offset = None;
            state.pending_report = None;
            state.report_offset();
        }
        self.flush_offset_report();
    }

    /// Writes a queued offset into the connected report binding. The write
    /// happens outside the state borrow so a binding watcher may safely
    /// re-enter this handle.
    fn flush_offset_report(&self) {
        let write = {
            let mut state = self.state.borrow_mut();
            state.pending_report.take().zip(state.offset_report.clone())
        };
        if let Some((point, report)) = write {
            report.set(point);
        }
    }
    /// Returns a key identifying the underlying scroll slot, stable for the
    /// slot's lifetime across rebuilds (the address of the shared state).
    #[must_use]
    pub fn cache_key(&self) -> usize {
        Rc::as_ptr(&self.state) as usize
    }

    /// Returns the current offsets and extents of the bound scroll view.
    #[must_use]
    pub fn metrics(&self) -> ScrollMetrics {
        let state = self.state.borrow();
        state.metrics()
    }

    /// The axis the bound scroll view scrolls along.
    #[must_use]
    pub fn axis(&self) -> Axis {
        let state = self.state.borrow();
        state.axis
    }

    /// Applies a wheel/trackpad delta along the scroll view's axis and
    /// returns whether it changed anything that needs a frame.
    ///
    /// Positive deltas scroll content toward its start (offsets decrease, the
    /// platform wheel convention). Pixel deltas (trackpads, which carry the
    /// OS's own momentum stream) move the offset directly. Line deltas
    /// (discrete wheel ticks) are scaled by 40 logical pixels per line and
    /// accumulated into a smooth-scroll target instead — the offset then eases
    /// toward it via [`Self::tick_smooth_scroll`], so wheel scrolling glides
    /// instead of teleporting one step per tick. Input from a handle whose
    /// generation is stale is dropped and returns `false`.
    #[must_use]
    pub fn apply_scroll_delta(&self, dx: f32, dy: f32, is_line_delta: bool) -> bool {
        let changed = {
            let mut state = self.state.borrow_mut();
            if state.generation != self.generation {
                return false;
            }
            state.apply_scroll_delta(f64::from(dx), f64::from(dy), is_line_delta)
        };
        self.flush_offset_report();
        changed
    }

    /// Advances an in-flight wheel glide or programmatic scroll animation by
    /// one frame and returns `true` while more animation frames are needed.
    /// Stale handles are inert.
    #[must_use]
    pub fn tick_smooth_scroll(&self, now: Instant) -> bool {
        let active = {
            let mut state = self.state.borrow_mut();
            if state.generation != self.generation {
                return false;
            }
            state.tick_smooth_scroll(now)
        };
        self.flush_offset_report();
        active
    }

    /// Whether a wheel glide or a programmatic scroll animation is still
    /// moving the offset, without advancing it. Stale handles are inert.
    #[must_use]
    pub fn is_smooth_scrolling(&self) -> bool {
        let state = self.state.borrow();
        state.generation == self.generation
            && (state.smooth_target_x.is_some()
                || state.smooth_target_y.is_some()
                || state.programmatic.is_some())
    }

    /// Jumps immediately to an absolute content offset, clamped to the current
    /// scrollable extents. Any in-flight wheel glide or programmatic scroll
    /// animation is cancelled.
    #[must_use]
    pub fn scroll_to(&self, x: f64, y: f64) -> bool {
        let changed = {
            let mut state = self.state.borrow_mut();
            if state.generation != self.generation {
                return false;
            }
            state.scroll_to(x, y)
        };
        self.flush_offset_report();
        changed
    }

    /// Jumps to an absolute content offset on the user's behalf — the
    /// scrollbar's drag and accessibility scroll-into-view route here rather
    /// than through [`Self::scroll_to`]: like any user input the write claims
    /// the offset and spends a recorded run outcome, so a pending request's
    /// post-landing correction cannot jump back over the user's scroll.
    #[must_use]
    pub fn user_scroll_to(&self, x: f64, y: f64) -> bool {
        let changed = {
            let mut state = self.state.borrow_mut();
            if state.generation != self.generation {
                return false;
            }
            state.user_scroll_to(x, y)
        };
        self.flush_offset_report();
        changed
    }

    /// Starts a programmatic scroll toward an absolute content offset along
    /// `animation`'s curve and duration, and returns the token identifying
    /// the run — `None` only for a stale handle, which is inert. `now` is the
    /// frame instant the request is applied at: the run's clock starts here,
    /// so the next tick already shows motion. The run is advanced by
    /// [`Self::tick_smooth_scroll`] — the same pump that drives the wheel
    /// glide — and lands exactly on the target. A jump ([`Self::scroll_to`]),
    /// a user pixel delta or drag, or a wheel glide cancels it; a second call
    /// restarts from the current offset. [`Self::scroll_run_outcome`] reports
    /// how the run ended.
    #[must_use]
    pub fn scroll_to_animated(
        &self,
        x: f64,
        y: f64,
        animation: Animation,
        now: Instant,
    ) -> Option<ScrollRun> {
        let token = {
            let mut state = self.state.borrow_mut();
            if state.generation != self.generation {
                return None;
            }
            state.scroll_to_animated(x, y, animation, now)
        };
        self.flush_offset_report();
        Some(ScrollRun {
            state: Rc::downgrade(&self.state),
            token,
        })
    }

    /// How the run `run` identifies ended — or whether it still drives the
    /// offset. The token itself scopes the query: a handle from an earlier
    /// generation still answers for the run it armed — a previous frame's
    /// handle must not report a live run as `Interrupted` — while a token
    /// beyond the claim counter, or one another scroll view issued, is a
    /// programming error and panics.
    ///
    /// # Panics
    ///
    /// When `run` was issued by a different scroll view's handle, or names a
    /// claim beyond the state's counter.
    #[must_use]
    pub fn scroll_run_outcome(&self, run: &ScrollRun) -> ScrollRunOutcome {
        self.assert_same_state(&run.state, "scroll run");
        self.state.borrow().scroll_run_outcome(run)
    }

    /// Refines the destination of the run `run` identifies without restarting
    /// its clock: a virtualized list re-issues its row target every frame as
    /// extents are measured, and the run keeps converging on the refined
    /// destination instead of re-arming. Refuses — returns `false` — when
    /// `run` is not the live run, so a requester cannot steer another owner's
    /// animation on the same surface. A run minted by another scroll view
    /// panics.
    ///
    /// # Panics
    ///
    /// When `run` was issued by a different scroll view's handle, or names a
    /// claim beyond the state's counter.
    #[must_use]
    pub fn retarget_animated_scroll(&self, run: &ScrollRun, x: f64, y: f64) -> bool {
        self.assert_same_state(&run.state, "scroll run");
        self.state.borrow_mut().retarget_animated_scroll(run, x, y)
    }

    /// Mints the claim a touch gesture holds on the offset, once its drag is
    /// recognised: the gesture takes the offset for user input — a live run
    /// ends here, a wheel glide's targets are dropped, and a recorded run
    /// outcome is spent — and the coordinate shift a fling's positions
    /// accumulate restarts. The drag then moves the offset through
    /// [`Self::apply_gesture_delta`] and the release hands the same claim to
    /// [`Self::begin_fling`]; only a newer claim, never a [`Self::rebind`],
    /// takes the offset from it.
    #[must_use]
    pub fn begin_gesture(&self) -> GestureClaim {
        let offset_epoch = self.state.borrow_mut().begin_gesture();
        GestureClaim {
            state: Rc::downgrade(&self.state),
            offset_epoch,
        }
    }

    /// Moves the offset by `(dx, dy)` on the scrolled axes for the touch
    /// gesture `claim` names, clamped to the extents live right now, and
    /// returns whether it moved. Refused — nothing is written and `false`
    /// returned — once a newer claim owns the offset. The claim is the
    /// ownership record, not the handle's generation: a `rebind` for rows
    /// measured mid-drag does not stop the drag, and the delta claims
    /// nothing again.
    ///
    /// # Panics
    ///
    /// When `claim` was minted by a different scroll view's handle.
    #[must_use]
    pub fn apply_gesture_delta(&self, claim: &GestureClaim, dx: f64, dy: f64) -> bool {
        self.assert_same_state(&claim.state, "gesture claim");
        let changed = {
            let mut state = self.state.borrow_mut();
            if state.offset_epoch != claim.offset_epoch {
                return false;
            }
            state.apply_gesture_delta(dx, dy)
        };
        self.flush_offset_report();
        changed
    }

    /// Starts the released gesture's fling on `claim` and returns whether
    /// the claim still owns the offset; when it does not, no fling starts.
    /// The fling then feeds the claim to [`Self::apply_fling_offset`] each
    /// tick. The coordinate shift its positions accumulate restarts here:
    /// the fling starts from the release offset, which already carries any
    /// shift applied during the drag.
    ///
    /// # Panics
    ///
    /// When `claim` was minted by a different scroll view's handle.
    #[must_use]
    pub fn begin_fling(&self, claim: &GestureClaim) -> bool {
        self.assert_same_state(&claim.state, "gesture claim");
        let mut state = self.state.borrow_mut();
        if state.offset_epoch != claim.offset_epoch {
            return false;
        }
        state.reset_fling_shift();
        true
    }

    /// Applies a running touch fling's offset for this tick while `claim`
    /// still owns the offset. Refuses (returns `false`) once a programmatic
    /// request or user input has claimed the offset since `claim` was minted,
    /// so the fling ends instead of writing over its successor. The claim is
    /// the ownership record — not the handle's generation: a `rebind` for
    /// rows measured mid-fling does not end it, and the write clamps to the
    /// extents live at that frame. `None` names an axis the fling does not
    /// sample: that axis keeps its current offset untouched.
    ///
    /// # Panics
    ///
    /// When `claim` was minted by a different scroll view's handle.
    #[must_use]
    pub fn apply_fling_offset(&self, claim: &GestureClaim, x: Option<f64>, y: Option<f64>) -> bool {
        self.assert_same_state(&claim.state, "gesture claim");
        let changed = {
            let mut state = self.state.borrow_mut();
            if state.offset_epoch != claim.offset_epoch {
                return false;
            }
            state.apply_fling_offset(x, y)
        };
        self.flush_offset_report();
        changed
    }

    /// Asserts `issuer` names this handle's state. Runs and gesture claims
    /// carry the issuing state's `Weak`, which pins its allocation, so an
    /// `Rc` address can never be recycled into a false match.
    fn assert_same_state(&self, issuer: &Weak<RefCell<ScrollState>>, what: &str) {
        assert!(
            core::ptr::eq(issuer.as_ptr(), Rc::as_ptr(&self.state)),
            "a {what} may only act on the handle that issued it"
        );
    }
}

impl ScrollState {
    fn new(
        axis: Axis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> Self {
        let mut state = Self {
            generation: 1,
            axis,
            viewport_width,
            viewport_height,
            content_width,
            content_height,
            offset_x: 0.0,
            offset_y: 0.0,
            smooth_target_x: None,
            smooth_target_y: None,
            smooth_last_tick: None,
            programmatic: None,
            offset_epoch: 0,
            last_run_outcome: None,
            fling_shift_x: 0.0,
            fling_shift_y: 0.0,
            offset_report: None,
            reported_offset: None,
            pending_report: None,
        };
        state.clamp_offsets();
        state
    }

    fn prepare_generation(
        &mut self,
        axis: Axis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
        (shift_x, shift_y): (f64, f64),
    ) -> u64 {
        let layout_changed = self.axis != axis
            || value_changed(self.viewport_width, viewport_width)
            || value_changed(self.viewport_height, viewport_height)
            || value_changed(self.content_width, content_width)
            || value_changed(self.content_height, content_height);
        self.axis = axis;
        self.viewport_width = viewport_width;
        self.viewport_height = viewport_height;
        self.content_width = content_width;
        self.content_height = content_height;
        // Translate first, clamp once: clamping against the new extents
        // before the shift would remove what the shift removes a second
        // time. The shift keeps the same content under the viewport, so only
        // the clamp's correction counts as an offset change.
        self.translate(shift_x, shift_y);
        let translated_x = self.offset_x;
        let translated_y = self.offset_y;
        self.clamp_offsets();
        let offset_changed = value_changed(translated_x, self.offset_x)
            || value_changed(translated_y, self.offset_y);
        if layout_changed || offset_changed {
            self.generation = self
                .generation
                .checked_add(1)
                .expect("scroll controller generation overflow");
        }
        self.report_offset();
        self.generation
    }

    /// Queues the current offset for [`ScrollState::offset_report`] when it
    /// differs from what was last written. The write itself is deferred into
    /// `pending_report` because it happens under a `RefCell` borrow the
    /// caller still holds.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the report binding is in points — the f32 precision of a scroll offset"
    )]
    fn report_offset(&mut self) {
        if self.offset_report.is_none() {
            return;
        }
        let offset = (self.offset_x, self.offset_y);
        if self.reported_offset == Some(offset) {
            return;
        }
        self.reported_offset = Some(offset);
        self.pending_report = Some(Point::new(self.offset_x as f32, self.offset_y as f32));
    }

    /// Takes the next claim on the offset for a writer — a request, a jump,
    /// a user delta — ending any programmatic run as `Interrupted` and
    /// returning the claim's value: a programmatic run's token, or the epoch
    /// a touch gesture must still hold to own the offset.
    const fn claim(&mut self) -> u64 {
        self.offset_epoch = self
            .offset_epoch
            .checked_add(1)
            .expect("scroll offset epoch overflow");
        self.end_programmatic(ScrollRunOutcome::Interrupted);
        self.offset_epoch
    }

    /// Takes the offset for a programmatic writer — a jump or a run —
    /// returning the claim: a live run ends `Interrupted` and the wheel
    /// glide, its targets and its frame clock, is dropped.
    const fn take_for_request(&mut self) -> u64 {
        let claim = self.claim();
        self.smooth_target_x = None;
        self.smooth_target_y = None;
        self.smooth_last_tick = None;
        claim
    }

    /// Takes the offset for user input: the request's take plus the spend of
    /// a recorded `Landed`, so a pending request cannot correct its
    /// post-landing position back over the user's scroll.
    const fn take_for_user(&mut self) -> u64 {
        let claim = self.take_for_request();
        self.spend_recorded_outcome();
        claim
    }

    /// Mints a touch gesture's claim: the gesture takes the offset as user
    /// input, and the coordinate shift a fling's positions accumulate
    /// restarts.
    const fn begin_gesture(&mut self) -> u64 {
        let claim = self.take_for_user();
        self.reset_fling_shift();
        claim
    }

    /// Restarts the coordinate shift a fling's sampled positions carry.
    const fn reset_fling_shift(&mut self) {
        self.fling_shift_x = 0.0;
        self.fling_shift_y = 0.0;
    }

    /// Translates the coordinate system by `(dx, dy)` on the scrolled axes
    /// without claiming the offset or clamping: the offset, the wheel-glide
    /// targets, a live run's origin and destination, and the fling's
    /// accumulated origin shift all move together.
    fn translate(&mut self, dx: f64, dy: f64) {
        let (scroll_x, scroll_y) = self.scrolled_axes();
        let dx = if scroll_x { dx } else { 0.0 };
        let dy = if scroll_y { dy } else { 0.0 };
        self.offset_x += dx;
        self.offset_y += dy;
        self.smooth_target_x = self.smooth_target_x.map(|target| target + dx);
        self.smooth_target_y = self.smooth_target_y.map(|target| target + dy);
        if let Some(run) = &mut self.programmatic {
            run.from_x += dx;
            run.from_y += dy;
            run.target_x += dx;
            run.target_y += dy;
        }
        self.fling_shift_x += dx;
        self.fling_shift_y += dy;
    }

    /// Spends the recorded run outcome, if any: user input after a run ended
    /// flips a remembered `Landed` to `Interrupted`, so a pending request
    /// cannot use the landing to correct back over the user's scroll.
    const fn spend_recorded_outcome(&mut self) {
        if let Some((_, outcome)) = &mut self.last_run_outcome {
            *outcome = ScrollRunOutcome::Interrupted;
        }
    }

    /// The axes this state scrolls, as `(x, y)` flags — the pair the
    /// per-axis matches all reduce to.
    const fn scrolled_axes(&self) -> (bool, bool) {
        match self.axis {
            Axis::Horizontal => (true, false),
            Axis::Vertical => (false, true),
            Axis::All => (true, true),
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        }
    }

    /// Clamps `(x, y)` to the current scrollable extents on the axes this
    /// state scrolls; a non-scrolled axis keeps its offset, so callers write
    /// the returned pair unconditionally.
    fn clamped(&self, x: f64, y: f64) -> (f64, f64) {
        let metrics = self.metrics();
        let (scroll_x, scroll_y) = self.scrolled_axes();
        (
            if scroll_x {
                clamp_scroll_offset(x, metrics.max_x)
            } else {
                self.offset_x
            },
            if scroll_y {
                clamp_scroll_offset(y, metrics.max_y)
            } else {
                self.offset_y
            },
        )
    }

    /// Asserts `token` is within the claim counter: a value beyond it was
    /// never handed out by this scroll view, so naming one is a programming
    /// error, not a dead run to report `Interrupted`.
    fn assert_issued_token(&self, token: u64) {
        assert!(
            token > 0 && token <= self.offset_epoch,
            "scroll run token {token} is beyond the claim counter — never issued by this scroll view"
        );
    }

    fn apply_scroll_delta(&mut self, dx: f64, dy: f64, is_line_delta: bool) -> bool {
        if is_line_delta {
            return self.retarget_smooth_scroll(dx * SCROLL_LINE_STEP, dy * SCROLL_LINE_STEP);
        }
        // Pixel deltas are direct manipulation (trackpads deliver their own
        // OS momentum stream). The claim comes after the move is known: a
        // zero delta, a dead-axis delta or a push at an extent changes
        // nothing, falls through to the enclosing view, and must not end
        // the run or fling that owns the offset.
        let (offset_x, offset_y) = self.clamped(self.offset_x - dx, self.offset_y - dy);
        if !value_changed(self.offset_x, offset_x) && !value_changed(self.offset_y, offset_y) {
            return false;
        }
        // The input moved something, so it claims the offset — a live run
        // ends, a fling's next write is refused, and a recorded `Landed`
        // left standing is spent rather than letting the request's
        // post-landing correction jump back over this scroll.
        self.take_for_user();
        self.offset_x = offset_x;
        self.offset_y = offset_y;
        self.report_offset();
        true
    }

    fn scroll_to(&mut self, x: f64, y: f64) -> bool {
        self.take_for_request();
        self.write_offset(x, y)
    }

    /// An absolute offset write driven by the user — the scrollbar's drag or
    /// an accessibility scroll-into-view: the same jump as [`Self::scroll_to`]
    /// plus the user-input spend, so a recorded `Landed` cannot let a pending
    /// request correct its post-landing position back over the user's scroll.
    fn user_scroll_to(&mut self, x: f64, y: f64) -> bool {
        self.take_for_user();
        self.write_offset(x, y)
    }

    /// Writes the clamped absolute offset for whoever just took it and
    /// reports whether it moved.
    fn write_offset(&mut self, x: f64, y: f64) -> bool {
        let (offset_x, offset_y) = self.clamped(x, y);
        let changed =
            value_changed(self.offset_x, offset_x) || value_changed(self.offset_y, offset_y);
        self.offset_x = offset_x;
        self.offset_y = offset_y;
        self.report_offset();
        changed
    }

    /// Arms a programmatic scroll animation toward an absolute content offset
    /// and returns the token identifying the run — the claim counter's value,
    /// so a run's token is the same ownership record a fling holds. The run
    /// starts from the offset current right now — a second request while one
    /// is in flight restarts from wherever the first had reached, and its
    /// token reports `Interrupted` — and replaces any wheel glide. `now` is
    /// the frame instant the request is applied at, so the run's clock is
    /// already running when the next tick advances it. A request that needs
    /// no travel lands in place and its token reports `Landed` at once.
    fn scroll_to_animated(&mut self, x: f64, y: f64, animation: Animation, now: Instant) -> u64 {
        let token = self.take_for_request();
        let (target_x, target_y) = self.clamped(x, y);
        if !value_changed(self.offset_x, target_x) && !value_changed(self.offset_y, target_y) {
            // Nothing to travel: land exactly rather than pump a duration's
            // worth of no-op frames — the token reports `Landed` at once.
            self.offset_x = target_x;
            self.offset_y = target_y;
            self.last_run_outcome = Some((token, ScrollRunOutcome::Landed));
            self.report_offset();
            return token;
        }
        self.programmatic = Some(ScrollAnimation {
            token,
            animation,
            started: now,
            from_x: self.offset_x,
            from_y: self.offset_y,
            target_x,
            target_y,
        });
        token
    }

    /// Ends the programmatic run in flight, if there is one, recording its
    /// token and outcome so the armer can still ask
    /// [`ScrollState::scroll_run_outcome`] about it.
    const fn end_programmatic(&mut self, outcome: ScrollRunOutcome) {
        if let Some(run) = self.programmatic.take() {
            self.last_run_outcome = Some((run.token, outcome));
        }
    }

    /// How the run `run` identifies ended — or whether it still drives the
    /// offset. Only a single outcome slot is kept, so an earlier run reports
    /// `Interrupted` once a newer one has ended; a token beyond the claim
    /// counter is a programming error and panics.
    fn scroll_run_outcome(&self, run: &ScrollRun) -> ScrollRunOutcome {
        self.assert_issued_token(run.token);
        if self
            .programmatic
            .as_ref()
            .is_some_and(|live| live.token == run.token)
        {
            return ScrollRunOutcome::Running;
        }
        match self.last_run_outcome {
            Some((token, outcome)) if token == run.token => outcome,
            _ => ScrollRunOutcome::Interrupted,
        }
    }

    /// Writes the fling's sampled offset while the fling still owns the
    /// offset — the claim's epoch was already checked by the handle. The
    /// coordinate shift a shifted rebind accumulated since the fling began
    /// applies to the positions it samples, so an anchored content move does
    /// not end the fling; an axis it does not sample (`None`) keeps its
    /// offset, which already carries the shift.
    fn apply_fling_offset(&mut self, x: Option<f64>, y: Option<f64>) -> bool {
        let (offset_x, offset_y) = self.clamped(
            x.map_or(self.offset_x, |x| x + self.fling_shift_x),
            y.map_or(self.offset_y, |y| y + self.fling_shift_y),
        );
        let changed =
            value_changed(self.offset_x, offset_x) || value_changed(self.offset_y, offset_y);
        self.offset_x = offset_x;
        self.offset_y = offset_y;
        self.report_offset();
        changed
    }

    /// Moves the offset by a touch gesture's delta on the scrolled axes,
    /// clamped, for the gesture that still owns it — the claim was already
    /// checked by the handle — and reports whether it moved.
    fn apply_gesture_delta(&mut self, dx: f64, dy: f64) -> bool {
        self.write_offset(self.offset_x + dx, self.offset_y + dy)
    }

    /// Moves the destination of the in-flight programmatic animation without
    /// touching its clock; reports whether `run` names the live run — the
    /// only one a requester may steer.
    fn retarget_animated_scroll(&mut self, run: &ScrollRun, x: f64, y: f64) -> bool {
        self.assert_issued_token(run.token);
        let (target_x, target_y) = self.clamped(x, y);
        let Some(live) = &mut self.programmatic else {
            return false;
        };
        if live.token != run.token {
            return false;
        }
        live.target_x = target_x;
        live.target_y = target_y;
        true
    }

    /// Accumulates a discrete wheel tick into the smooth-scroll targets and
    /// reports whether an animation toward them is (still) needed. Successive
    /// ticks retarget the same animation, so fast wheel spins add up instead
    /// of restarting. A tick that changes no target — a zero delta, a
    /// dead-axis delta, or a push at an extent — claims nothing and reports
    /// `false`, so it falls through to the enclosing scroll view without
    /// ending what owns the offset; only while a glide is still live on the
    /// pushed axis is such a tick consumed, since the content is moving.
    #[allow(
        clippy::similar_names,
        reason = "`scaled_dx`/`scaled_dy` are conventional 2D scroll-delta names"
    )]
    fn retarget_smooth_scroll(&mut self, scaled_dx: f64, scaled_dy: f64) -> bool {
        let metrics = self.metrics();
        let (scroll_x, scroll_y) = self.scrolled_axes();
        let base_x = self.smooth_target_x.unwrap_or(self.offset_x);
        let base_y = self.smooth_target_y.unwrap_or(self.offset_y);
        let target_x = clamp_scroll_offset(base_x - scaled_dx, metrics.max_x);
        let target_y = clamp_scroll_offset(base_y - scaled_dy, metrics.max_y);
        let changed_x = scroll_x && value_changed(base_x, target_x);
        let changed_y = scroll_y && value_changed(base_y, target_y);
        if !changed_x && !changed_y {
            // A push past the edge a live glide is still heading for belongs
            // to that glide: falling through would scroll the enclosing view
            // while this one is still moving. The glide already owns the
            // offset, so nothing is claimed.
            return (scroll_x && scaled_dx != 0.0 && self.smooth_target_x.is_some())
                || (scroll_y && scaled_dy != 0.0 && self.smooth_target_y.is_some());
        }
        // A tick that starts a glide claims the offset: a programmatic run
        // ends — a wheel glide replaces it — and a recorded `Landed` is spent
        // with the other user input. A live glide already holds the claim —
        // every other writer drops the glide when it takes the offset — so a
        // tick extending it keeps the glide's frame clock running.
        if self.smooth_target_x.is_none() && self.smooth_target_y.is_none() {
            self.take_for_user();
        }
        if changed_x {
            self.smooth_target_x = Some(target_x);
        }
        if changed_y {
            self.smooth_target_y = Some(target_y);
        }
        self.settle_reached_smooth_targets();
        self.report_offset();
        self.smooth_target_x.is_some() || self.smooth_target_y.is_some()
    }

    /// Advances whichever scroll animation is in flight and reports whether
    /// it still needs more frames.
    fn tick_smooth_scroll(&mut self, now: Instant) -> bool {
        let active = if self.programmatic.is_some() {
            self.advance_programmatic_scroll(now)
        } else {
            self.advance_smooth_scroll(now)
        };
        self.report_offset();
        active
    }

    /// Advances the programmatic scroll animation along its curve and returns
    /// whether it still needs more frames. Each tick samples
    /// [`Animation::progress`] for the elapsed time — a spring can overshoot
    /// or pull back — and clamps the applied offset to the extents live at
    /// that frame, since content can change mid-flight. On completion the
    /// offset lands exactly on the target and the run's token reports
    /// `Landed`.
    fn advance_programmatic_scroll(&mut self, now: Instant) -> bool {
        let Some(run) = &mut self.programmatic else {
            return false;
        };
        let elapsed = now.saturating_duration_since(run.started);
        let complete = run.animation.is_complete(elapsed);
        let progress = f64::from(run.animation.progress(elapsed));
        let (from_x, from_y, target_x, target_y) =
            (run.from_x, run.from_y, run.target_x, run.target_y);
        // At `is_complete` the eased progress is exactly 1.0 for every curve
        // (`Animation::progress` clamps the phase to [0, 1] and each curve
        // pins t = 1), but the target is applied directly so the landing
        // stays bit-exact.
        let (offset_x, offset_y) = if complete {
            self.clamped(target_x, target_y)
        } else {
            self.clamped(
                (target_x - from_x).mul_add(progress, from_x),
                (target_y - from_y).mul_add(progress, from_y),
            )
        };
        self.offset_x = offset_x;
        self.offset_y = offset_y;
        if complete {
            self.end_programmatic(ScrollRunOutcome::Landed);
        }
        !complete
    }

    /// Advances the smoothed wheel offsets toward their targets with a
    /// frame-rate-independent exponential approach and returns whether the
    /// animation still needs more frames.
    fn advance_smooth_scroll(&mut self, now: Instant) -> bool {
        if self.smooth_target_x.is_none() && self.smooth_target_y.is_none() {
            self.smooth_last_tick = None;
            return false;
        }
        let dt = self
            .smooth_last_tick
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64());
        self.smooth_last_tick = Some(now);
        let blend = 1.0 - (-dt / SMOOTH_SCROLL_TAU).exp();
        if let Some(target) = self.smooth_target_x {
            self.offset_x = (target - self.offset_x).mul_add(blend, self.offset_x);
        }
        if let Some(target) = self.smooth_target_y {
            self.offset_y = (target - self.offset_y).mul_add(blend, self.offset_y);
        }
        self.settle_reached_smooth_targets();
        if self.smooth_target_x.is_none() && self.smooth_target_y.is_none() {
            self.smooth_last_tick = None;
            return false;
        }
        true
    }

    /// Snaps offsets that are within the settle epsilon of their wheel target
    /// and clears the finished targets.
    fn settle_reached_smooth_targets(&mut self) {
        if let Some(target) = self.smooth_target_x
            && (target - self.offset_x).abs() < SMOOTH_SCROLL_SETTLE_EPSILON
        {
            self.offset_x = target;
            self.smooth_target_x = None;
        }
        if let Some(target) = self.smooth_target_y
            && (target - self.offset_y).abs() < SMOOTH_SCROLL_SETTLE_EPSILON
        {
            self.offset_y = target;
            self.smooth_target_y = None;
        }
    }

    fn clamp_offsets(&mut self) {
        let metrics = self.metrics();
        self.offset_x = clamp_scroll_offset(self.offset_x, metrics.max_x);
        self.offset_y = clamp_scroll_offset(self.offset_y, metrics.max_y);
        self.smooth_target_x = self
            .smooth_target_x
            .map(|target| clamp_scroll_offset(target, metrics.max_x));
        self.smooth_target_y = self
            .smooth_target_y
            .map(|target| clamp_scroll_offset(target, metrics.max_y));
        // Arming and retargeting keep a run's destination inside the extents;
        // a shift or a shrink must too, or the run reaches the edge early and
        // parks there for the rest of its duration.
        if let Some(run) = &mut self.programmatic {
            run.target_x = clamp_scroll_offset(run.target_x, metrics.max_x);
            run.target_y = clamp_scroll_offset(run.target_y, metrics.max_y);
        }
    }

    fn metrics(&self) -> ScrollMetrics {
        let max_x = (self.content_width - self.viewport_width).max(0.0);
        let max_y = (self.content_height - self.viewport_height).max(0.0);
        ScrollMetrics {
            offset_x: self.offset_x,
            offset_y: self.offset_y,
            max_x,
            max_y,
            viewport_width: self.viewport_width,
            viewport_height: self.viewport_height,
            content_width: self.content_width,
            content_height: self.content_height,
        }
    }
}

const fn clamp_scroll_offset(value: f64, max: f64) -> f64 {
    value.clamp(0.0, max)
}

fn value_changed(old: f64, new: f64) -> bool {
    (old - new).abs() > SCROLL_EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use core::time::Duration;
    use nami::Signal as _;

    fn vertical_handle() -> ScrollHandle {
        ScrollHandle::new(Axis::Vertical, 100.0, 100.0, 100.0, 300.0, None)
    }

    /// A touch gesture released straight into its fling, with no drag in
    /// between.
    fn touch_fling(handle: &ScrollHandle) -> GestureClaim {
        let claim = handle.begin_gesture();
        assert!(handle.begin_fling(&claim), "a fresh claim owns the offset");
        claim
    }

    #[test]
    fn a_gesture_delta_lands_after_an_extent_changing_rebind() {
        let mut handle = vertical_handle();
        let drag_handle = handle.clone();
        let claim = drag_handle.begin_gesture();
        assert!(drag_handle.apply_gesture_delta(&claim, 0.0, 50.0));
        assert_eq!(handle.metrics().offset_y, 50.0);

        // A row measured taller than its estimate grows the content: the
        // rebind advances the generation, so the handle the drag captured is
        // stale for per-frame input ...
        let _ = handle.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 312.0, (0.0, 0.0));
        assert!(!drag_handle.apply_scroll_delta(0.0, -10.0, false));
        assert_eq!(handle.metrics().offset_y, 50.0);

        // ... but the gesture still owns the offset, and its delta lands,
        // clamped to the grown extent.
        assert!(drag_handle.apply_gesture_delta(&claim, 0.0, 30.0));
        assert_eq!(handle.metrics().offset_y, 80.0);
        assert!(drag_handle.apply_gesture_delta(&claim, 0.0, 400.0));
        assert_eq!(handle.metrics().offset_y, 212.0);
        // A delta on the axis the view does not scroll moves nothing.
        assert!(!drag_handle.apply_gesture_delta(&claim, 25.0, 0.0));
        assert_eq!(handle.metrics().offset_x, 0.0);
    }

    /// A newer offset owner, named for the assertion messages, and how it
    /// takes the offset.
    type Competitor = (&'static str, fn(&ScrollHandle));

    #[test]
    fn a_newer_claim_refuses_the_gestures_next_delta() {
        let competitors: [Competitor; 5] = [
            ("scroll_to", |handle| {
                let _ = handle.scroll_to(0.0, 100.0);
            }),
            ("user_scroll_to", |handle| {
                let _ = handle.user_scroll_to(0.0, 100.0);
            }),
            ("a wheel tick", |handle| {
                assert!(handle.apply_scroll_delta(0.0, -1.0, true));
            }),
            ("a trackpad pixel delta", |handle| {
                assert!(handle.apply_scroll_delta(0.0, -10.0, false));
            }),
            ("scroll_to_animated", |handle| {
                assert!(
                    handle
                        .scroll_to_animated(0.0, 200.0, Animation::default(), Instant::now())
                        .is_some()
                );
            }),
        ];
        for (name, compete) in competitors {
            let handle = vertical_handle();
            let claim = handle.begin_gesture();
            assert!(handle.apply_gesture_delta(&claim, 0.0, 40.0), "{name}");
            compete(&handle);
            let after_competitor = handle.metrics().offset_y;
            assert!(
                !handle.apply_gesture_delta(&claim, 0.0, 20.0),
                "{name} must refuse the gesture's next delta"
            );
            assert_eq!(
                handle.metrics().offset_y,
                after_competitor,
                "a refused gesture delta must write nothing after {name}"
            );
            // A new gesture's claim owns the offset again.
            let next = handle.begin_gesture();
            assert!(handle.apply_gesture_delta(&next, 0.0, -10.0), "{name}");
        }
    }

    #[test]
    fn begin_fling_on_a_stale_claim_starts_no_fling() {
        let handle = vertical_handle();
        let claim = handle.begin_gesture();
        assert!(handle.apply_gesture_delta(&claim, 0.0, 40.0));
        let _ = handle.scroll_to(0.0, 100.0);
        assert!(!handle.begin_fling(&claim));
        assert!(!handle.apply_fling_offset(&claim, None, Some(150.0)));
        assert_eq!(handle.metrics().offset_y, 100.0);

        // A newer gesture's claim supersedes an older one's.
        let first = handle.begin_gesture();
        let second = handle.begin_gesture();
        assert!(!handle.begin_fling(&first));
        assert!(handle.begin_fling(&second));
    }

    #[test]
    fn a_shift_during_the_drag_is_not_added_to_the_flings_positions() {
        let mut handle = vertical_handle();
        let claim = handle.begin_gesture();
        assert!(handle.apply_gesture_delta(&claim, 0.0, 50.0));
        // 40pt of rows land above the viewport mid-drag: the anchor shifts
        // the offset with the content and the drag keeps owning it.
        let _ = handle.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 340.0, (0.0, 40.0));
        assert_eq!(handle.metrics().offset_y, 90.0);
        assert!(handle.apply_gesture_delta(&claim, 0.0, 10.0));
        assert_eq!(handle.metrics().offset_y, 100.0);

        // The fling starts from the release offset, which already carries
        // the drag's shift: its positions are not shifted a second time.
        assert!(handle.begin_fling(&claim));
        assert!(handle.apply_fling_offset(&claim, None, Some(120.0)));
        assert_eq!(handle.metrics().offset_y, 120.0);

        // A shift after the fling began still applies to its positions.
        let _ = handle.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 360.0, (0.0, 20.0));
        assert!(handle.apply_fling_offset(&claim, None, Some(130.0)));
        assert_eq!(handle.metrics().offset_y, 150.0);
    }

    /// A `report_offset` sink that counts every write, for asserting the
    /// backend writes on change only.
    fn counting_report() -> (Binding<Point>, Rc<Cell<usize>>) {
        let writes = Rc::new(Cell::new(0usize));
        let offset = nami::binding(Point::zero());
        let counted = offset.filter({
            let writes = Rc::clone(&writes);
            move |_| {
                writes.set(writes.get() + 1);
                true
            }
        });
        (counted, writes)
    }

    #[test]
    fn scroll_offsets_clamp_to_content_extent() {
        let handle = vertical_handle();
        assert!(handle.apply_scroll_delta(0.0, -50.0, false));
        assert_eq!(handle.metrics().offset_y, 50.0);

        // Overscroll clamps to max_y = content - viewport = 200.
        assert!(handle.apply_scroll_delta(0.0, -10_000.0, false));
        assert_eq!(handle.metrics().offset_y, 200.0);

        // Clamped at the end: a further push changes nothing.
        assert!(!handle.apply_scroll_delta(0.0, -1.0, false));

        // Scrolling back past the top clamps to zero.
        assert!(handle.apply_scroll_delta(0.0, 10_000.0, false));
        assert_eq!(handle.metrics().offset_y, 0.0);
    }

    #[test]
    fn vertical_axis_ignores_horizontal_delta() {
        let handle = vertical_handle();
        assert!(!handle.apply_scroll_delta(-30.0, 0.0, false));
        assert_eq!(handle.metrics().offset_x, 0.0);
    }

    #[test]
    fn line_deltas_smooth_toward_the_scaled_target() {
        let handle = vertical_handle();
        let start = Instant::now();
        // Two wheel ticks accumulate one 80px target without moving yet.
        assert!(handle.apply_scroll_delta(0.0, -2.0, true));
        assert_eq!(handle.metrics().offset_y, 0.0);

        // The first tick only arms the clock; later ticks converge on the
        // target and the animation reports completion.
        assert!(handle.tick_smooth_scroll(start));
        let mut now = start;
        let mut active = true;
        for _ in 0..600 {
            now += Duration::from_millis(8);
            active = handle.tick_smooth_scroll(now);
            if !active {
                break;
            }
        }
        assert!(!active, "smooth wheel scroll must settle");
        assert_eq!(handle.metrics().offset_y, 80.0);
    }

    #[test]
    fn animated_scroll_eases_instead_of_teleporting() {
        let handle = vertical_handle();
        let start = Instant::now();

        // Arming the animation must not move the offset: that is the whole
        // difference from `scroll_to`, which lands on the target immediately.
        assert!(
            handle
                .scroll_to_animated(0.0, 200.0, Animation::default(), start)
                .is_some()
        );
        assert_eq!(handle.metrics().offset_y, 0.0);

        // The run's clock starts at the arm instant, so a tick at that same
        // instant still shows zero elapsed.
        assert!(handle.tick_smooth_scroll(start));
        assert_eq!(handle.metrics().offset_y, 0.0);

        // Partway through the duration the offset is strictly between the
        // start and the destination — an instant jump would already be at 200.
        let mid = start + Duration::from_millis(120);
        assert!(handle.tick_smooth_scroll(mid));
        let midpoint = handle.metrics().offset_y;
        assert!(
            midpoint > 0.0 && midpoint < 200.0,
            "animated scroll should be in flight at 120ms, was at {midpoint}"
        );

        let mut now = mid;
        let mut active = true;
        for _ in 0..600 {
            now += Duration::from_millis(8);
            active = handle.tick_smooth_scroll(now);
            if !active {
                break;
            }
        }
        assert!(!active, "animated scroll must settle");
        assert_eq!(handle.metrics().offset_y, 200.0);
    }

    #[test]
    fn the_first_tick_after_arming_already_shows_motion() {
        let handle = vertical_handle();
        let armed = Instant::now();
        // The clock starts at the arm instant, not the first tick: one frame
        // later the offset has already moved — there is no dead frame.
        assert!(
            handle
                .scroll_to_animated(0.0, 200.0, Animation::default(), armed)
                .is_some()
        );
        assert!(handle.tick_smooth_scroll(armed + Duration::from_millis(16)));
        assert!(
            handle.metrics().offset_y > 0.0,
            "one frame after arming the offset must already be moving"
        );
    }

    #[test]
    fn scroll_run_outcome_reports_running_landed_and_interrupted() {
        let handle = vertical_handle();
        let start = Instant::now();
        let run = handle
            .scroll_to_animated(
                0.0,
                200.0,
                Animation::linear(Duration::from_millis(100)),
                start,
            )
            .expect("the run must arm");
        assert_eq!(handle.scroll_run_outcome(&run), ScrollRunOutcome::Running);
        assert!(!handle.tick_smooth_scroll(start + Duration::from_millis(100)));
        assert_eq!(handle.scroll_run_outcome(&run), ScrollRunOutcome::Landed);

        // A run cancelled by user input reports Interrupted — the delta has
        // to move the offset: a push at the extent changes nothing and
        // claims nothing, so the run still drives.
        let cancelled = handle
            .scroll_to_animated(0.0, 0.0, Animation::default(), start)
            .expect("the run must arm");
        assert!(!handle.apply_scroll_delta(0.0, -10.0, false));
        assert!(!handle.apply_scroll_delta(0.0, -1.0, true));
        assert_eq!(
            handle.scroll_run_outcome(&cancelled),
            ScrollRunOutcome::Running,
            "a push at the edge must not end the run"
        );
        let _ = handle.apply_scroll_delta(0.0, 10.0, false);
        assert_eq!(
            handle.scroll_run_outcome(&cancelled),
            ScrollRunOutcome::Interrupted
        );

        // A run replaced by a newer request reports Interrupted too.
        let replaced = handle
            .scroll_to_animated(0.0, 150.0, Animation::default(), start)
            .expect("the run must arm");
        assert!(
            handle
                .scroll_to_animated(0.0, 100.0, Animation::default(), start)
                .is_some()
        );
        assert_eq!(
            handle.scroll_run_outcome(&replaced),
            ScrollRunOutcome::Interrupted
        );

        // A fling's mint takes the offset from a live run.
        let flung = handle
            .scroll_to_animated(0.0, 0.0, Animation::default(), start)
            .expect("the run must arm");
        let claim = touch_fling(&handle);
        assert_eq!(
            handle.scroll_run_outcome(&flung),
            ScrollRunOutcome::Interrupted
        );

        // A push at the edge claims nothing, so the fling still owns the
        // offset and its next write applies.
        assert!(handle.apply_fling_offset(&claim, None, Some(200.0)));
        assert!(!handle.apply_scroll_delta(0.0, -10.0, false));
        assert!(!handle.apply_scroll_delta(0.0, -1.0, true));
        assert!(handle.apply_fling_offset(&claim, None, Some(150.0)));
        assert_eq!(handle.metrics().offset_y, 150.0);
    }

    #[test]
    fn a_fling_write_is_refused_once_the_offset_changes_hands() {
        let handle = vertical_handle();
        let claim = touch_fling(&handle);
        // While nothing else claimed the offset, the fling's write applies.
        assert!(handle.apply_fling_offset(&claim, None, Some(50.0)));
        assert_eq!(handle.metrics().offset_y, 50.0);

        // A programmatic request claims the offset: the fling's next write is
        // refused instead of writing over its successor.
        let _ = handle.scroll_to(0.0, 100.0);
        assert!(!handle.apply_fling_offset(&claim, None, Some(80.0)));
        assert_eq!(handle.metrics().offset_y, 100.0);

        // Same for user input and for an animated request.
        let claim = touch_fling(&handle);
        assert!(handle.apply_fling_offset(&claim, None, Some(60.0)));
        let _ = handle.apply_scroll_delta(0.0, -10.0, false);
        assert!(!handle.apply_fling_offset(&claim, None, Some(40.0)));
        assert_eq!(handle.metrics().offset_y, 70.0);

        let claim = touch_fling(&handle);
        assert!(
            handle
                .scroll_to_animated(0.0, 200.0, Animation::default(), Instant::now())
                .is_some()
        );
        assert!(!handle.apply_fling_offset(&claim, None, Some(30.0)));
    }

    #[test]
    fn a_shifted_rebind_moves_everything_without_claiming_the_offset() {
        let mut handle = vertical_handle();
        let claim = touch_fling(&handle);
        assert!(handle.apply_fling_offset(&claim, None, Some(50.0)));

        // A membership anchor translates the coordinate system by +40 as 40pt
        // of rows land above: the fling's claim survives — it owned the
        // offset before and owns it still — and the shift moves both the
        // offset and the origin the fling's next positions are measured from.
        let _ = handle.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 340.0, (0.0, 40.0));
        assert_eq!(handle.metrics().offset_y, 90.0);
        assert!(handle.apply_fling_offset(&claim, None, Some(60.0)));
        assert_eq!(handle.metrics().offset_y, 100.0);

        // Same for a programmatic run: the run keeps driving, and its origin
        // and destination moved with the rows — the sampled offset advances
        // from the shifted origin toward the shifted target.
        let start = Instant::now();
        let run = handle
            .scroll_to_animated(
                0.0,
                160.0,
                Animation::linear(Duration::from_millis(100)),
                start,
            )
            .expect("the run must arm");
        let _ = handle.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 380.0, (0.0, 40.0));
        assert_eq!(handle.scroll_run_outcome(&run), ScrollRunOutcome::Running);
        assert!(handle.tick_smooth_scroll(start + Duration::from_millis(50)));
        assert_eq!(
            handle.metrics().offset_y,
            170.0,
            "the shift must move the run's sampled offset: origin 140, target 200"
        );

        // A wheel-glide target translates too, and stays clamped: the tick
        // aims at 250, the +40 shift past the 280 end clamps it there.
        assert!(handle.apply_scroll_delta(0.0, -2.0, true));
        assert_eq!(handle.state.borrow().smooth_target_y, Some(250.0));
        let _ = handle.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 380.0, (0.0, 40.0));
        assert_eq!(handle.metrics().offset_y, 210.0);
        assert_eq!(
            handle.state.borrow().smooth_target_y,
            Some(280.0),
            "the shifted glide target must clamp to the scrollable end"
        );
    }

    #[test]
    fn a_shifted_rebind_translates_before_it_clamps() {
        // Scrolled to the end (200 of 300), 60pt of rows above the viewport
        // are deleted: the end moves to 140 and the shift is −60. Translating
        // first lands on 140; clamping first (to 140) and then shifting would
        // remove the deleted height twice, landing on 80.
        let mut handle = vertical_handle();
        let _ = handle.scroll_to(0.0, 200.0);
        let _ = handle.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 240.0, (0.0, -60.0));
        assert_eq!(handle.metrics().offset_y, 140.0);

        // Same with a run in flight: 150 of its way to 200, the deletion
        // translates it to 90 — inside the new extents — and it keeps running
        // toward its shifted target, the new end.
        let mut handle = vertical_handle();
        let start = Instant::now();
        let run = handle
            .scroll_to_animated(
                0.0,
                200.0,
                Animation::linear(Duration::from_millis(100)),
                start,
            )
            .expect("the run must arm");
        assert!(handle.tick_smooth_scroll(start + Duration::from_millis(75)));
        assert_eq!(handle.metrics().offset_y, 150.0);
        let _ = handle.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 240.0, (0.0, -60.0));
        assert_eq!(handle.metrics().offset_y, 90.0);
        assert_eq!(handle.scroll_run_outcome(&run), ScrollRunOutcome::Running);
        assert!(!handle.tick_smooth_scroll(start + Duration::from_millis(100)));
        assert_eq!(handle.metrics().offset_y, 140.0);
        assert_eq!(handle.scroll_run_outcome(&run), ScrollRunOutcome::Landed);
    }

    #[test]
    fn a_fling_shift_applies_only_to_the_axes_it_samples() {
        let mut handle = ScrollHandle::new(Axis::All, 100.0, 100.0, 300.0, 300.0, None);
        let _ = handle.scroll_to(50.0, 50.0);
        let claim = touch_fling(&handle);
        let _ = handle.rebind(Axis::All, 100.0, 100.0, 340.0, 340.0, (40.0, 40.0));
        assert_eq!(
            (handle.metrics().offset_x, handle.metrics().offset_y),
            (90.0, 90.0)
        );
        // A vertical-only fling: x keeps its already shifted offset, y's
        // sampled position gets the shift.
        assert!(handle.apply_fling_offset(&claim, None, Some(60.0)));
        assert_eq!(
            (handle.metrics().offset_x, handle.metrics().offset_y),
            (90.0, 100.0)
        );
    }

    #[test]
    fn a_line_delta_at_the_edge_is_consumed_while_the_glide_is_live() {
        let handle = vertical_handle();
        let start = Instant::now();
        // The glide's target reaches the end (200) while the offset is still
        // on its way there.
        assert!(handle.apply_scroll_delta(0.0, -10.0, true));
        assert_eq!(handle.state.borrow().smooth_target_y, Some(200.0));
        assert!(handle.tick_smooth_scroll(start));
        let epoch = handle.state.borrow().offset_epoch;
        // A further tick changes no target, but the list is still moving: it
        // is consumed rather than handed to the enclosing scroll view, and it
        // claims nothing.
        assert!(handle.apply_scroll_delta(0.0, -1.0, true));
        assert_eq!(handle.state.borrow().offset_epoch, epoch);
        // Once the glide settles, the same push falls through.
        let mut now = start;
        let mut active = true;
        for _ in 0..600 {
            now += Duration::from_millis(8);
            active = handle.tick_smooth_scroll(now);
            if !active {
                break;
            }
        }
        assert!(!active, "the glide must settle");
        assert_eq!(handle.metrics().offset_y, 200.0);
        assert!(!handle.apply_scroll_delta(0.0, -1.0, true));
    }

    #[test]
    fn a_shift_clamps_a_live_runs_target_to_the_new_extents() {
        let mut handle = vertical_handle();
        let _ = handle.scroll_to(0.0, 200.0);
        let start = Instant::now();
        let run = handle
            .scroll_to_animated(
                0.0,
                0.0,
                Animation::linear(Duration::from_millis(100)),
                start,
            )
            .expect("the run must arm");
        // Rows above the viewport deleted: the run's origin translates from
        // 200 to 32 and its target from 0 to -168, which the clamp pulls back
        // to the edge.
        let _ = handle.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 300.0, (0.0, -168.0));
        assert_eq!(handle.metrics().offset_y, 32.0);
        assert_eq!(
            handle
                .state
                .borrow()
                .programmatic
                .as_ref()
                .map(|run| run.target_y),
            Some(0.0)
        );
        // Mid-run the offset is still on its way: toward an unclamped target
        // it would already have reached the edge and parked there.
        assert!(handle.tick_smooth_scroll(start + Duration::from_millis(50)));
        let mid = handle.metrics().offset_y;
        assert!(
            mid > 0.0 && mid < 32.0,
            "the run must be between its origin 32 and the edge mid-run: {mid}"
        );
        assert!(handle.tick_smooth_scroll(start + Duration::from_millis(99)));
        assert!(handle.metrics().offset_y > 0.0);
        assert_eq!(handle.scroll_run_outcome(&run), ScrollRunOutcome::Running);
        // It reaches the edge exactly when its duration ends.
        assert!(!handle.tick_smooth_scroll(start + Duration::from_millis(100)));
        assert_eq!(handle.metrics().offset_y, 0.0);
        assert_eq!(handle.scroll_run_outcome(&run), ScrollRunOutcome::Landed);
    }

    #[test]
    fn user_input_after_landing_ends_the_run() {
        let handle = vertical_handle();
        let start = Instant::now();
        let run = handle
            .scroll_to_animated(
                0.0,
                200.0,
                Animation::linear(Duration::from_millis(50)),
                start,
            )
            .expect("the run must arm");
        assert!(!handle.tick_smooth_scroll(start + Duration::from_millis(50)));
        assert_eq!(handle.scroll_run_outcome(&run), ScrollRunOutcome::Landed);

        // The user scrolls after the run landed: the request is spent — its
        // owner must not correct over a user scroll — so it reports
        // Interrupted from here on.
        assert!(handle.apply_scroll_delta(0.0, 10.0, false));
        assert_eq!(
            handle.scroll_run_outcome(&run),
            ScrollRunOutcome::Interrupted
        );

        // A programmatic jump after a landing leaves the outcome standing —
        // only user input spends it — while the user's absolute write (the
        // scrollbar's drag) spends it like a delta.
        let landed = handle
            .scroll_to_animated(
                0.0,
                0.0,
                Animation::linear(Duration::from_millis(50)),
                start,
            )
            .expect("the run must arm");
        assert!(!handle.tick_smooth_scroll(start + Duration::from_millis(50)));
        assert!(handle.scroll_to(0.0, 100.0));
        assert_eq!(handle.scroll_run_outcome(&landed), ScrollRunOutcome::Landed);
        assert!(handle.user_scroll_to(0.0, 50.0));
        assert_eq!(
            handle.scroll_run_outcome(&landed),
            ScrollRunOutcome::Interrupted
        );
    }

    #[test]
    #[should_panic(expected = "beyond the claim counter")]
    fn a_token_the_state_never_issued_panics_instead_of_reporting_interrupted() {
        let handle = vertical_handle();
        // Token 1 may exist or not — either way the claim counter has not
        // reached 5, so this token is beyond the claim counter and the query
        // is a programming error, not a dead run.
        let forged = ScrollRun {
            state: Rc::downgrade(&handle.state),
            token: 5,
        };
        let _ = handle.scroll_run_outcome(&forged);
    }

    #[test]
    #[should_panic(expected = "handle that issued it")]
    fn a_run_from_another_scroll_view_panics_on_query() {
        let handle = vertical_handle();
        let other = vertical_handle();
        let foreign = other
            .scroll_to_animated(0.0, 50.0, Animation::default(), Instant::now())
            .expect("the run must arm");
        let _ = handle.scroll_run_outcome(&foreign);
    }

    #[test]
    fn spring_scroll_settles_on_the_target() {
        let handle = vertical_handle();
        let start = Instant::now();
        // An underdamped spring overshoots mid-flight; the applied offset must
        // stay clamped to the scrollable extent and the run must still land
        // exactly on the target when its duration ends.
        assert!(
            handle
                .scroll_to_animated(0.0, 200.0, Animation::spring(100.0, 10.0), start)
                .is_some()
        );
        let mut now = start;
        let mut active = true;
        let mut frames = 0usize;
        while active && frames < 600 {
            now += Duration::from_millis(8);
            active = handle.tick_smooth_scroll(now);
            assert!(
                handle.metrics().offset_y <= 200.0 + 1e-6,
                "spring overshoot must clamp to the scrollable extent"
            );
            frames += 1;
        }
        assert!(!active, "spring scroll must settle inside its duration");
        assert_eq!(handle.metrics().offset_y, 200.0);
        assert!(!handle.is_smooth_scrolling());
    }

    #[test]
    fn pixel_delta_cancels_an_in_flight_programmatic_animation() {
        let handle = vertical_handle();
        let start = Instant::now();
        assert!(
            handle
                .scroll_to_animated(0.0, 200.0, Animation::default(), start)
                .is_some()
        );
        assert!(handle.tick_smooth_scroll(start));
        let _ = handle.tick_smooth_scroll(start + Duration::from_millis(60));
        let mid = handle.metrics().offset_y;
        assert!(mid > 0.0 && mid < 200.0, "expected an in-flight offset");

        // A trackpad pixel delta takes over: direct move, animation dropped.
        assert!(handle.apply_scroll_delta(0.0, -10.0, false));
        assert!((handle.metrics().offset_y - (mid + 10.0)).abs() < 1e-9);
        assert!(
            !handle.tick_smooth_scroll(start + Duration::from_millis(76)),
            "programmatic animation must be cancelled by direct manipulation"
        );
        assert!((handle.metrics().offset_y - (mid + 10.0)).abs() < 1e-9);
    }

    #[test]
    fn wheel_glide_replaces_an_in_flight_programmatic_animation() {
        let handle = vertical_handle();
        let start = Instant::now();
        assert!(
            handle
                .scroll_to_animated(0.0, 200.0, Animation::default(), start)
                .is_some()
        );
        assert!(handle.tick_smooth_scroll(start));

        // A wheel tick cancels the programmatic run and arms its own glide.
        assert!(handle.apply_scroll_delta(0.0, -2.0, true));
        let mut now = start + Duration::from_millis(16);
        let mut active = true;
        for _ in 0..600 {
            active = handle.tick_smooth_scroll(now);
            if !active {
                break;
            }
            now += Duration::from_millis(8);
        }
        assert!(!active, "wheel glide must settle");
        // The glide settles on the wheel target (two 40px lines), not the
        // programmatic target of 200.
        assert_eq!(handle.metrics().offset_y, 80.0);
    }

    #[test]
    fn retarget_animated_scroll_refines_only_the_run_it_names() {
        let handle = vertical_handle();
        let start = Instant::now();
        let run = handle
            .scroll_to_animated(
                0.0,
                200.0,
                Animation::linear(Duration::from_millis(100)),
                start,
            )
            .expect("the run must arm");
        assert!(handle.tick_smooth_scroll(start + Duration::from_millis(50)));
        assert_eq!(handle.metrics().offset_y, 100.0);

        // Refining the destination keeps the run's clock: at 60ms the offset
        // is 60% of the way to the refined target — a restart would sit at
        // the 100.0 it had already reached.
        assert!(handle.retarget_animated_scroll(&run, 0.0, 150.0));
        assert!(handle.tick_smooth_scroll(start + Duration::from_millis(60)));
        let offset = handle.metrics().offset_y;
        assert!(
            (offset - 90.0).abs() < 0.01,
            "retargeted run should sample its original clock, offset {offset}"
        );

        // Completion lands exactly on the refined target.
        assert!(!handle.tick_smooth_scroll(start + Duration::from_millis(100)));
        assert_eq!(handle.metrics().offset_y, 150.0);

        // A token whose run already ended — or names a different live run —
        // cannot steer anything: a second owner's animation stays untouched.
        assert!(!handle.retarget_animated_scroll(&run, 0.0, 75.0));
        let other = handle
            .scroll_to_animated(0.0, 100.0, Animation::default(), start)
            .expect("the second run must arm");
        assert!(!handle.retarget_animated_scroll(&run, 0.0, 75.0));
        assert_eq!(handle.scroll_run_outcome(&other), ScrollRunOutcome::Running);
    }

    #[test]
    fn new_animated_scroll_restarts_from_the_current_offset() {
        let handle = vertical_handle();
        let start = Instant::now();
        assert!(
            handle
                .scroll_to_animated(
                    0.0,
                    200.0,
                    Animation::linear(Duration::from_millis(100)),
                    start,
                )
                .is_some()
        );
        assert!(handle.tick_smooth_scroll(start + Duration::from_millis(50)));
        assert_eq!(handle.metrics().offset_y, 100.0);

        // A second request restarts: it re-arms from the in-flight offset and
        // its clock starts at the new arm instant.
        let restart = start + Duration::from_millis(60);
        assert!(
            handle
                .scroll_to_animated(
                    0.0,
                    0.0,
                    Animation::linear(Duration::from_millis(100)),
                    restart,
                )
                .is_some()
        );
        assert!(handle.tick_smooth_scroll(restart));
        assert_eq!(handle.metrics().offset_y, 100.0);
        assert!(handle.tick_smooth_scroll(restart + Duration::from_millis(50)));
        assert_eq!(handle.metrics().offset_y, 50.0);
        assert!(!handle.tick_smooth_scroll(restart + Duration::from_millis(100)));
        assert_eq!(handle.metrics().offset_y, 0.0);
    }

    #[test]
    fn immediate_scroll_to_cancels_an_in_flight_animation() {
        let handle = vertical_handle();
        let start = Instant::now();
        assert!(
            handle
                .scroll_to_animated(0.0, 200.0, Animation::default(), start)
                .is_some()
        );
        assert!(handle.tick_smooth_scroll(start));
        assert!(handle.is_smooth_scrolling());

        assert!(handle.scroll_to(0.0, 50.0));
        assert_eq!(handle.metrics().offset_y, 50.0);
        assert!(!handle.is_smooth_scrolling());
        assert!(!handle.tick_smooth_scroll(start + Duration::from_millis(8)));
        assert_eq!(handle.metrics().offset_y, 50.0);
    }

    #[test]
    fn smooth_wheel_progress_is_frame_rate_independent() {
        // The same wall-clock time must land at the same offset whether it is
        // sampled at 120Hz or 30Hz.
        let fine = vertical_handle();
        let coarse = vertical_handle();

        let start = Instant::now();
        assert!(fine.apply_scroll_delta(0.0, -4.0, true));
        assert!(coarse.apply_scroll_delta(0.0, -4.0, true));
        let _ = fine.tick_smooth_scroll(start);
        let _ = coarse.tick_smooth_scroll(start);
        for step in 1..=12u64 {
            let _ = fine.tick_smooth_scroll(start + Duration::from_millis(step * 8));
        }
        for step in 1..=3u64 {
            let _ = coarse.tick_smooth_scroll(start + Duration::from_millis(step * 32));
        }
        let fine_offset = fine.metrics().offset_y;
        let coarse_offset = coarse.metrics().offset_y;
        assert!(
            (fine_offset - coarse_offset).abs() < 1.0,
            "offsets diverged: fine={fine_offset} coarse={coarse_offset}"
        );
    }

    #[test]
    fn pixel_deltas_cancel_in_flight_wheel_smoothing() {
        let handle = vertical_handle();
        let start = Instant::now();
        assert!(handle.apply_scroll_delta(0.0, -2.0, true));
        let _ = handle.tick_smooth_scroll(start);
        let _ = handle.tick_smooth_scroll(start + Duration::from_millis(16));

        // A trackpad pixel delta takes over: direct move, animation dropped.
        let mid = handle.metrics().offset_y;
        assert!(handle.apply_scroll_delta(0.0, -10.0, false));
        assert!((handle.metrics().offset_y - (mid + 10.0)).abs() < 1e-9);
        assert!(
            !handle.tick_smooth_scroll(start + Duration::from_millis(32)),
            "wheel animation must be cancelled by direct manipulation"
        );
    }

    #[test]
    fn rebinding_with_same_layout_keeps_offset_and_handle_validity() {
        let mut owner = vertical_handle();
        let handle = owner.clone();
        assert!(handle.apply_scroll_delta(0.0, -50.0, false));
        let rebound = owner.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 300.0, (0.0, 0.0));
        assert_eq!(rebound.metrics().offset_y, 50.0);
        // The previous handle still targets the same generation.
        assert!(handle.apply_scroll_delta(0.0, -10.0, false));
    }

    #[test]
    fn layout_change_invalidates_stale_handles() {
        let mut owner = vertical_handle();
        let handle = owner.clone();
        let rebound = owner.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 500.0, (0.0, 0.0));
        // The stale handle's generation no longer matches: input is dropped.
        assert!(!handle.apply_scroll_delta(0.0, -10.0, false));
        assert!(
            rebound.apply_scroll_delta(0.0, -10.0, false) || rebound.metrics().offset_y == 10.0
        );
        assert_eq!(rebound.metrics().offset_y, 10.0);
    }

    #[test]
    fn viewport_growth_reclamps_existing_offset() {
        let mut owner = vertical_handle();
        let handle = owner.clone();
        assert!(handle.apply_scroll_delta(0.0, -10_000.0, false));
        assert_eq!(handle.metrics().offset_y, 200.0);
        // The viewport now shows the whole content: the offset clamps home.
        let rebound = owner.rebind(Axis::Vertical, 100.0, 300.0, 100.0, 300.0, (0.0, 0.0));
        assert_eq!(rebound.metrics().offset_y, 0.0);
        assert_eq!(rebound.metrics().max_y, 0.0);
    }

    #[test]
    fn absolute_jump_clamps_and_can_repeat_after_manual_scroll() {
        let handle = vertical_handle();
        assert!(handle.scroll_to(50.0, 150.0));
        assert_eq!(handle.metrics().offset_x, 0.0);
        assert_eq!(handle.metrics().offset_y, 150.0);

        assert!(handle.apply_scroll_delta(0.0, 50.0, false));
        assert_eq!(handle.metrics().offset_y, 100.0);
        assert!(handle.scroll_to(0.0, 150.0));
        assert_eq!(handle.metrics().offset_y, 150.0);

        assert!(handle.scroll_to(0.0, 10_000.0));
        assert_eq!(handle.metrics().offset_y, 200.0);
    }

    #[test]
    fn absolute_jump_cancels_wheel_animation() {
        let handle = vertical_handle();
        let start = Instant::now();
        assert!(handle.apply_scroll_delta(0.0, -2.0, true));
        let _ = handle.tick_smooth_scroll(start);

        assert!(handle.scroll_to(0.0, 120.0));
        assert!(!handle.tick_smooth_scroll(start + Duration::from_millis(16)));
        assert_eq!(handle.metrics().offset_y, 120.0);
    }

    #[test]
    fn report_offset_writes_current_offset_on_attach() {
        let handle = vertical_handle();
        assert!(handle.apply_scroll_delta(0.0, -40.0, false));
        let (report, writes) = counting_report();
        handle.set_offset_report(Some(report.clone()));
        assert_eq!(writes.get(), 1);
        assert_eq!(report.snapshot(), Point::new(0.0, 40.0));
    }

    #[test]
    fn report_offset_writes_on_direct_and_absolute_scrolls() {
        let handle = vertical_handle();
        let (report, writes) = counting_report();
        handle.set_offset_report(Some(report.clone()));

        // A trackpad pixel delta moves the offset directly: one write.
        assert!(handle.apply_scroll_delta(0.0, -30.0, false));
        assert_eq!(report.snapshot(), Point::new(0.0, 30.0));
        assert_eq!(writes.get(), 2);

        // A controller `scroll_to` lands immediately: one write.
        assert!(handle.scroll_to(0.0, 150.0));
        assert_eq!(report.snapshot(), Point::new(0.0, 150.0));
        assert_eq!(writes.get(), 3);
    }

    #[test]
    fn report_offset_writes_every_glide_frame_and_stays_silent_when_idle() {
        let handle = vertical_handle();
        let (report, writes) = counting_report();
        handle.set_offset_report(Some(report.clone()));
        assert_eq!(writes.get(), 1);

        let start = Instant::now();
        // A wheel line delta only retargets the glide — no offset write yet.
        assert!(handle.apply_scroll_delta(0.0, -2.0, true));
        assert_eq!(writes.get(), 1);

        // Every animation frame that moves the offset writes once.
        let mut now = start;
        let mut frames = 0usize;
        while handle.tick_smooth_scroll(now) {
            frames += 1;
            let reported = report.snapshot();
            let metrics = handle.metrics();
            assert!(
                (f64::from(reported.y) - metrics.offset_y).abs() < 1e-4,
                "reported offset {reported:?} lags the true offset {metrics:?}"
            );
            now += Duration::from_millis(8);
        }
        assert_eq!(writes.get(), 1 + frames);
        assert_eq!(report.snapshot(), Point::new(0.0, 80.0));

        // Idle handles are silent: ticks and a zero delta write nothing.
        assert!(!handle.tick_smooth_scroll(now));
        assert!(!handle.apply_scroll_delta(0.0, 0.0, false));
        let _ = handle.tick_smooth_scroll(now + Duration::from_millis(8));
        assert_eq!(writes.get(), 1 + frames);
    }

    #[test]
    fn report_offset_stops_writing_once_disconnected() {
        let handle = vertical_handle();
        let (report, writes) = counting_report();
        handle.set_offset_report(Some(report.clone()));
        handle.set_offset_report(None);
        assert!(handle.apply_scroll_delta(0.0, -10.0, false));
        assert_eq!(writes.get(), 1);
        assert_eq!(report.snapshot(), Point::zero());
    }

    #[test]
    fn a_far_animated_row_scroll_stops_the_bound_short_of_the_target() {
        assert_eq!(
            animated_row_scroll_approach(0, 180),
            Some(180 - ANIMATED_ROW_SCROLL_APPROACH)
        );
        assert_eq!(
            animated_row_scroll_approach(400, 20),
            Some(20 + ANIMATED_ROW_SCROLL_APPROACH)
        );
    }

    #[test]
    fn the_animated_row_scroll_approach_starts_one_row_past_the_bound() {
        const BOUND: usize = ANIMATED_ROW_SCROLL_APPROACH;
        assert_eq!(animated_row_scroll_approach(0, BOUND + 1), Some(1));
        assert_eq!(animated_row_scroll_approach(0, BOUND), None);
        assert_eq!(animated_row_scroll_approach(BOUND + 1, 0), Some(BOUND));
        assert_eq!(animated_row_scroll_approach(BOUND, 0), None);
        assert_eq!(animated_row_scroll_approach(7, 7), None);
    }
}
