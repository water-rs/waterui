//! Scroll offset/viewport math owned by each semantic scroll view.
//!
//! A [`ScrollHandle`] retains one scroll view's state directly. Rebinding the
//! same handle after layout preserves its offset; a layout change advances its
//! generation so input closures captured by an earlier frame become inert.
//! There is deliberately no renderer slot registry or body-order identity.

use std::cell::RefCell;
use std::rc::Rc;

use nami::Binding;
use waterui_core::animation::Animation;
use waterui_core::layout::Point;
use waterui_layout::scroll::Axis;

use crate::time::Instant;

const SCROLL_EPSILON: f64 = 0.000_01;
/// Logical pixels one wheel/accessibility "line" scrolls.
pub const SCROLL_LINE_STEP: f64 = 40.0;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollRun(u64);

/// How a [`ScrollRun`] ended — or whether it still drives the offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollRunOutcome {
    /// The run is still animating.
    Running,
    /// The run completed and landed on its target.
    Landed,
    /// The run ended early: user input claimed the offset (pixel deltas,
    /// line deltas, touch drags, scrollbar jumps), a jump landed, or a
    /// newer programmatic request replaced it. Also reported for a token
    /// the handle never issued, for a stale handle, and for a run whose
    /// outcome slot a later run already took — a run that cannot be found
    /// is certainly not running.
    Interrupted,
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
    /// The token the next armed programmatic run is issued.
    next_run_token: u64,
    /// The token and landing of the most recently ended programmatic run,
    /// remembered until another run ends so the armer can ask
    /// [`ScrollHandle::scroll_run_outcome`] about it after it is gone.
    last_run_outcome: Option<(u64, bool)>,
    /// Bumped by every write path that is not the fling's own — requests,
    /// jumps, user deltas. A touch fling captures it at start through
    /// [`ScrollHandle::offset_epoch`] and ends itself once it differs:
    /// something else claimed the offset.
    offset_epoch: u64,
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
    #[must_use]
    pub fn rebind(
        &mut self,
        axis: Axis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> Self {
        self.generation = self.state.borrow_mut().prepare_generation(
            axis,
            viewport_width,
            viewport_height,
            content_width,
            content_height,
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
        Some(ScrollRun(token))
    }

    /// How the run `run` identifies ended — or whether it still drives the
    /// offset. A stale handle and a token the handle never issued report
    /// [`ScrollRunOutcome::Interrupted`].
    #[must_use]
    pub fn scroll_run_outcome(&self, run: ScrollRun) -> ScrollRunOutcome {
        let state = self.state.borrow();
        if state.generation != self.generation {
            return ScrollRunOutcome::Interrupted;
        }
        state.scroll_run_outcome(run)
    }

    /// Refines the destination of the run `run` identifies without restarting
    /// its clock: a virtualized list re-issues its row target every frame as
    /// extents are measured, and the run keeps converging on the refined
    /// destination instead of re-arming. Refuses — returns `false` — when
    /// `run` is not the live run, so a requester cannot steer another owner's
    /// animation on the same surface. Stale handles are inert.
    #[must_use]
    pub fn retarget_animated_scroll(&self, run: ScrollRun, x: f64, y: f64) -> bool {
        let mut state = self.state.borrow_mut();
        if state.generation != self.generation {
            return false;
        }
        state.retarget_animated_scroll(run, x, y)
    }

    /// The epoch at which the offset was last claimed — bumped by every write
    /// path that is not the fling's own ([`Self::scroll_to`],
    /// [`Self::scroll_to_animated`], [`Self::apply_scroll_delta`]). A touch
    /// fling captures it when it starts and feeds it back to
    /// [`Self::apply_fling_offset`]; once it differs, a request or user input
    /// has claimed the offset and the fling ends.
    #[must_use]
    pub fn offset_epoch(&self) -> u64 {
        self.state.borrow().offset_epoch
    }

    /// Applies a running touch fling's offset for this tick while the fling
    /// still owns the offset — the epoch captured at fling start must still
    /// be [`Self::offset_epoch`]. Refuses (returns `false`) once a
    /// programmatic request or user input has claimed the offset since, so
    /// the fling ends instead of writing over its successor.
    #[must_use]
    pub fn apply_fling_offset(&self, epoch: u64, x: f64, y: f64) -> bool {
        let changed = {
            let mut state = self.state.borrow_mut();
            if state.generation != self.generation {
                return false;
            }
            state.apply_fling_offset(epoch, x, y)
        };
        self.flush_offset_report();
        changed
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
            next_run_token: 1,
            last_run_outcome: None,
            offset_epoch: 0,
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
    ) -> u64 {
        let layout_changed = self.axis != axis
            || value_changed(self.viewport_width, viewport_width)
            || value_changed(self.viewport_height, viewport_height)
            || value_changed(self.content_width, content_width)
            || value_changed(self.content_height, content_height);
        let old_offset_x = self.offset_x;
        let old_offset_y = self.offset_y;
        self.axis = axis;
        self.viewport_width = viewport_width;
        self.viewport_height = viewport_height;
        self.content_width = content_width;
        self.content_height = content_height;
        self.clamp_offsets();
        let offset_changed = value_changed(old_offset_x, self.offset_x)
            || value_changed(old_offset_y, self.offset_y);
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

    #[allow(
        clippy::similar_names,
        reason = "`old_x`/`old_y` and `scaled_dx`/`scaled_dy` are conventional 2D scroll-delta names"
    )]
    fn apply_scroll_delta(&mut self, dx: f64, dy: f64, is_line_delta: bool) -> bool {
        // User input claims the offset — a fling comparing epochs ends itself.
        self.offset_epoch = self
            .offset_epoch
            .checked_add(1)
            .expect("scroll offset epoch overflow");
        let changed = if is_line_delta {
            self.retarget_smooth_scroll(dx * SCROLL_LINE_STEP, dy * SCROLL_LINE_STEP)
        } else {
            // Pixel deltas are direct manipulation (trackpads deliver their own
            // OS momentum stream); they cancel any in-flight smooth-wheel
            // target or programmatic animation.
            self.smooth_target_x = None;
            self.smooth_target_y = None;
            self.end_programmatic(false);
            let metrics = self.metrics();
            let old_x = self.offset_x;
            let old_y = self.offset_y;

            match self.axis {
                Axis::Horizontal => {
                    self.offset_x = clamp_scroll_offset(old_x - dx, metrics.max_x);
                }
                Axis::Vertical => {
                    self.offset_y = clamp_scroll_offset(old_y - dy, metrics.max_y);
                }
                Axis::All => {
                    self.offset_x = clamp_scroll_offset(old_x - dx, metrics.max_x);
                    self.offset_y = clamp_scroll_offset(old_y - dy, metrics.max_y);
                }
                _ => panic!("scroll axis variant is not supported by hydrolysis"),
            }

            (self.offset_x - old_x).abs() > SCROLL_EPSILON
                || (self.offset_y - old_y).abs() > SCROLL_EPSILON
        };
        self.report_offset();
        changed
    }

    fn scroll_to(&mut self, x: f64, y: f64) -> bool {
        self.offset_epoch = self
            .offset_epoch
            .checked_add(1)
            .expect("scroll offset epoch overflow");
        let metrics = self.metrics();
        let old_x = self.offset_x;
        let old_y = self.offset_y;
        self.smooth_target_x = None;
        self.smooth_target_y = None;
        self.smooth_last_tick = None;
        self.end_programmatic(false);
        match self.axis {
            Axis::Horizontal => {
                self.offset_x = clamp_scroll_offset(x, metrics.max_x);
            }
            Axis::Vertical => {
                self.offset_y = clamp_scroll_offset(y, metrics.max_y);
            }
            Axis::All => {
                self.offset_x = clamp_scroll_offset(x, metrics.max_x);
                self.offset_y = clamp_scroll_offset(y, metrics.max_y);
            }
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        }
        let changed = value_changed(old_x, self.offset_x) || value_changed(old_y, self.offset_y);
        self.report_offset();
        changed
    }

    /// Arms a programmatic scroll animation toward an absolute content offset
    /// and returns the token identifying the run. The run starts from the
    /// offset current right now — a second request while one is in flight
    /// restarts from wherever the first had reached, and its token reports
    /// `Interrupted` — and replaces any wheel glide. `now` is the frame
    /// instant the request is applied at, so the run's clock is already
    /// running when the next tick advances it. A request that needs no travel
    /// lands in place and its token reports `Landed` at once.
    fn scroll_to_animated(&mut self, x: f64, y: f64, animation: Animation, now: Instant) -> u64 {
        self.offset_epoch = self
            .offset_epoch
            .checked_add(1)
            .expect("scroll offset epoch overflow");
        self.smooth_target_x = None;
        self.smooth_target_y = None;
        self.smooth_last_tick = None;
        self.end_programmatic(false);
        let token = self.next_run_token;
        self.next_run_token = self
            .next_run_token
            .checked_add(1)
            .expect("scroll run token overflow");
        let metrics = self.metrics();
        let (target_x, target_y) = match self.axis {
            Axis::Horizontal => (clamp_scroll_offset(x, metrics.max_x), self.offset_y),
            Axis::Vertical => (self.offset_x, clamp_scroll_offset(y, metrics.max_y)),
            Axis::All => (
                clamp_scroll_offset(x, metrics.max_x),
                clamp_scroll_offset(y, metrics.max_y),
            ),
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        };
        if !value_changed(self.offset_x, target_x) && !value_changed(self.offset_y, target_y) {
            // Nothing to travel: land exactly rather than pump a duration's
            // worth of no-op frames — the token reports `Landed` at once.
            self.offset_x = target_x;
            self.offset_y = target_y;
            self.last_run_outcome = Some((token, true));
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
    /// token and whether it landed so the armer can still ask
    /// [`ScrollState::scroll_run_outcome`] about it.
    const fn end_programmatic(&mut self, landed: bool) {
        if let Some(run) = self.programmatic.take() {
            self.last_run_outcome = Some((run.token, landed));
        }
    }

    /// How the run `run` identifies ended — or whether it still drives the
    /// offset. A token this state never issued reports `Interrupted`: its
    /// run is certainly not live.
    fn scroll_run_outcome(&self, run: ScrollRun) -> ScrollRunOutcome {
        if self
            .programmatic
            .as_ref()
            .is_some_and(|live| live.token == run.0)
        {
            return ScrollRunOutcome::Running;
        }
        match self.last_run_outcome {
            Some((token, landed)) if token == run.0 => {
                if landed {
                    ScrollRunOutcome::Landed
                } else {
                    ScrollRunOutcome::Interrupted
                }
            }
            _ => ScrollRunOutcome::Interrupted,
        }
    }

    /// Writes the fling's sampled offset while the fling still owns the
    /// offset: any claim newer than `epoch` — a request, a jump, user input —
    /// owns it instead, and the refusal ends the fling rather than writing
    /// over its successor.
    fn apply_fling_offset(&mut self, epoch: u64, x: f64, y: f64) -> bool {
        if self.offset_epoch != epoch {
            return false;
        }
        let metrics = self.metrics();
        let old_x = self.offset_x;
        let old_y = self.offset_y;
        match self.axis {
            Axis::Horizontal => {
                self.offset_x = clamp_scroll_offset(x, metrics.max_x);
            }
            Axis::Vertical => {
                self.offset_y = clamp_scroll_offset(y, metrics.max_y);
            }
            Axis::All => {
                self.offset_x = clamp_scroll_offset(x, metrics.max_x);
                self.offset_y = clamp_scroll_offset(y, metrics.max_y);
            }
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        }
        let changed = value_changed(old_x, self.offset_x) || value_changed(old_y, self.offset_y);
        self.report_offset();
        changed
    }

    /// Moves the destination of the in-flight programmatic animation without
    /// touching its clock; reports whether `run` names the live run — the
    /// only one a requester may steer.
    fn retarget_animated_scroll(&mut self, run: ScrollRun, x: f64, y: f64) -> bool {
        let metrics = self.metrics();
        let Some(live) = &mut self.programmatic else {
            return false;
        };
        if live.token != run.0 {
            return false;
        }
        match self.axis {
            Axis::Horizontal => live.target_x = clamp_scroll_offset(x, metrics.max_x),
            Axis::Vertical => live.target_y = clamp_scroll_offset(y, metrics.max_y),
            Axis::All => {
                live.target_x = clamp_scroll_offset(x, metrics.max_x);
                live.target_y = clamp_scroll_offset(y, metrics.max_y);
            }
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        }
        true
    }

    /// Accumulates a discrete wheel tick into the smooth-scroll targets and
    /// reports whether an animation toward them is (still) needed. Successive
    /// ticks retarget the same animation, so fast wheel spins add up instead
    /// of restarting.
    #[allow(
        clippy::similar_names,
        reason = "`scaled_dx`/`scaled_dy` are conventional 2D scroll-delta names"
    )]
    fn retarget_smooth_scroll(&mut self, scaled_dx: f64, scaled_dy: f64) -> bool {
        let metrics = self.metrics();
        // A wheel glide replaces a programmatic animation in flight.
        self.end_programmatic(false);
        match self.axis {
            Axis::Horizontal => {
                let target = self.smooth_target_x.unwrap_or(self.offset_x);
                self.smooth_target_x = Some(clamp_scroll_offset(target - scaled_dx, metrics.max_x));
            }
            Axis::Vertical => {
                let target = self.smooth_target_y.unwrap_or(self.offset_y);
                self.smooth_target_y = Some(clamp_scroll_offset(target - scaled_dy, metrics.max_y));
            }
            Axis::All => {
                let target_x = self.smooth_target_x.unwrap_or(self.offset_x);
                self.smooth_target_x =
                    Some(clamp_scroll_offset(target_x - scaled_dx, metrics.max_x));
                let target_y = self.smooth_target_y.unwrap_or(self.offset_y);
                self.smooth_target_y =
                    Some(clamp_scroll_offset(target_y - scaled_dy, metrics.max_y));
            }
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
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
        let metrics = self.metrics();
        // At `is_complete` the eased progress is exactly 1.0 for every curve
        // (`Animation::progress` clamps the phase to [0, 1] and each curve
        // pins t = 1), but the target is applied directly so the landing
        // stays bit-exact.
        let applied = |from: f64, target: f64, max: f64| {
            if complete {
                clamp_scroll_offset(target, max)
            } else {
                clamp_scroll_offset((target - from).mul_add(progress, from), max)
            }
        };
        match self.axis {
            Axis::Horizontal => self.offset_x = applied(from_x, target_x, metrics.max_x),
            Axis::Vertical => self.offset_y = applied(from_y, target_y, metrics.max_y),
            Axis::All => {
                self.offset_x = applied(from_x, target_x, metrics.max_x);
                self.offset_y = applied(from_y, target_y, metrics.max_y);
            }
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        }
        if complete {
            self.end_programmatic(true);
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
        assert_eq!(handle.scroll_run_outcome(run), ScrollRunOutcome::Running);
        assert!(!handle.tick_smooth_scroll(start + Duration::from_millis(100)));
        assert_eq!(handle.scroll_run_outcome(run), ScrollRunOutcome::Landed);

        // A run cancelled by user input reports Interrupted.
        let cancelled = handle
            .scroll_to_animated(0.0, 0.0, Animation::default(), start)
            .expect("the run must arm");
        let _ = handle.apply_scroll_delta(0.0, -10.0, false);
        assert_eq!(
            handle.scroll_run_outcome(cancelled),
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
            handle.scroll_run_outcome(replaced),
            ScrollRunOutcome::Interrupted
        );
    }

    #[test]
    fn a_fling_write_is_refused_once_the_offset_changes_hands() {
        let handle = vertical_handle();
        let epoch = handle.offset_epoch();
        // While nothing else claimed the offset, the fling's write applies.
        assert!(handle.apply_fling_offset(epoch, 0.0, 50.0));
        assert_eq!(handle.metrics().offset_y, 50.0);

        // A programmatic request claims the offset: the fling's next write is
        // refused instead of writing over its successor.
        let _ = handle.scroll_to(0.0, 100.0);
        assert!(!handle.apply_fling_offset(epoch, 0.0, 80.0));
        assert_eq!(handle.metrics().offset_y, 100.0);

        // Same for user input and for an animated request.
        let epoch = handle.offset_epoch();
        assert!(handle.apply_fling_offset(epoch, 0.0, 60.0));
        let _ = handle.apply_scroll_delta(0.0, -10.0, false);
        assert!(!handle.apply_fling_offset(epoch, 0.0, 40.0));
        assert_eq!(handle.metrics().offset_y, 70.0);

        let epoch = handle.offset_epoch();
        assert!(
            handle
                .scroll_to_animated(0.0, 200.0, Animation::default(), Instant::now())
                .is_some()
        );
        assert!(!handle.apply_fling_offset(epoch, 0.0, 30.0));
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
        assert!(handle.retarget_animated_scroll(run, 0.0, 150.0));
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
        assert!(!handle.retarget_animated_scroll(run, 0.0, 75.0));
        let other = handle
            .scroll_to_animated(0.0, 100.0, Animation::default(), start)
            .expect("the second run must arm");
        assert!(!handle.retarget_animated_scroll(run, 0.0, 75.0));
        assert_eq!(handle.scroll_run_outcome(other), ScrollRunOutcome::Running);
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
        let rebound = owner.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 300.0);
        assert_eq!(rebound.metrics().offset_y, 50.0);
        // The previous handle still targets the same generation.
        assert!(handle.apply_scroll_delta(0.0, -10.0, false));
    }

    #[test]
    fn layout_change_invalidates_stale_handles() {
        let mut owner = vertical_handle();
        let handle = owner.clone();
        let rebound = owner.rebind(Axis::Vertical, 100.0, 100.0, 100.0, 500.0);
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
        let rebound = owner.rebind(Axis::Vertical, 100.0, 300.0, 100.0, 300.0);
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
}
