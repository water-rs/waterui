//! Scroll offset/viewport math owned by each semantic scroll view.
//!
//! A [`ScrollHandle`] retains one scroll view's state directly. Rebinding the
//! same handle after layout preserves its offset; a layout change advances its
//! generation so input closures captured by an earlier frame become inert.
//! There is deliberately no renderer slot registry or body-order identity.

use std::cell::RefCell;
use std::rc::Rc;

use crate::time::Instant;
use waterui_layout::scroll::Axis;

const SCROLL_EPSILON: f64 = 0.000_01;
/// Logical pixels one wheel/accessibility "line" scrolls.
pub const SCROLL_LINE_STEP: f64 = 40.0;
/// Time constant (seconds) of the exponential approach that smooths discrete
/// wheel ticks toward their accumulated target: ~63% of the remaining gap per
/// τ, visually settled (>95%) after ~3τ ≈ 180ms — the smooth-wheel feel of
/// browsers and native lists instead of an instant 40px teleport per tick.
const WHEEL_SMOOTHING_TAU: f64 = 0.06;
/// Remaining gap below which a smoothed wheel scroll snaps to its target and
/// the animation ends.
const WHEEL_SETTLE_EPSILON: f64 = 0.1;

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
    /// Accumulated smooth-wheel destination per axis. Discrete wheel ticks
    /// (line deltas) retarget these instead of moving the offset directly;
    /// [`ScrollState::tick_smooth_scroll`] then eases the offset toward them
    /// frame by frame. Trackpad pixel deltas cancel them — direct
    /// manipulation (with the OS's own momentum stream) always wins.
    wheel_target_x: Option<f64>,
    wheel_target_y: Option<f64>,
    /// Last smooth-scroll tick, for a frame-rate-independent blend factor.
    wheel_last_tick: Option<Instant>,
}

impl ScrollHandle {
    /// Creates the state owned by one semantic scroll view.
    #[must_use]
    pub fn new(
        axis: Axis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> Self {
        Self {
            state: Rc::new(RefCell::new(ScrollState::new(
                axis,
                viewport_width,
                viewport_height,
                content_width,
                content_height,
            ))),
            generation: 1,
        }
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
        self.clone()
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
        let mut state = self.state.borrow_mut();
        if state.generation != self.generation {
            return false;
        }
        state.apply_scroll_delta(f64::from(dx), f64::from(dy), is_line_delta)
    }

    /// Advances any in-flight smoothed wheel scroll toward its target with a
    /// frame-rate-independent exponential approach; returns `true` while more
    /// animation frames are needed. Stale handles are inert.
    #[must_use]
    pub fn tick_smooth_scroll(&self, now: Instant) -> bool {
        let mut state = self.state.borrow_mut();
        if state.generation != self.generation {
            return false;
        }
        state.tick_smooth_scroll(now)
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
            wheel_target_x: None,
            wheel_target_y: None,
            wheel_last_tick: None,
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
        self.generation
    }

    #[allow(
        clippy::similar_names,
        reason = "`old_x`/`old_y` and `scaled_dx`/`scaled_dy` are conventional 2D scroll-delta names"
    )]
    fn apply_scroll_delta(&mut self, dx: f64, dy: f64, is_line_delta: bool) -> bool {
        if is_line_delta {
            return self.retarget_wheel(dx * SCROLL_LINE_STEP, dy * SCROLL_LINE_STEP);
        }
        // Pixel deltas are direct manipulation (trackpads deliver their own
        // OS momentum stream); they cancel any in-flight smooth-wheel target.
        self.wheel_target_x = None;
        self.wheel_target_y = None;
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
    }

    /// Accumulates a discrete wheel tick into the smooth-scroll targets and
    /// reports whether an animation toward them is (still) needed. Successive
    /// ticks retarget the same animation, so fast wheel spins add up instead
    /// of restarting.
    #[allow(
        clippy::similar_names,
        reason = "`scaled_dx`/`scaled_dy` are conventional 2D scroll-delta names"
    )]
    fn retarget_wheel(&mut self, scaled_dx: f64, scaled_dy: f64) -> bool {
        let metrics = self.metrics();
        match self.axis {
            Axis::Horizontal => {
                let target = self.wheel_target_x.unwrap_or(self.offset_x);
                self.wheel_target_x = Some(clamp_scroll_offset(target - scaled_dx, metrics.max_x));
            }
            Axis::Vertical => {
                let target = self.wheel_target_y.unwrap_or(self.offset_y);
                self.wheel_target_y = Some(clamp_scroll_offset(target - scaled_dy, metrics.max_y));
            }
            Axis::All => {
                let target_x = self.wheel_target_x.unwrap_or(self.offset_x);
                self.wheel_target_x =
                    Some(clamp_scroll_offset(target_x - scaled_dx, metrics.max_x));
                let target_y = self.wheel_target_y.unwrap_or(self.offset_y);
                self.wheel_target_y =
                    Some(clamp_scroll_offset(target_y - scaled_dy, metrics.max_y));
            }
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        }
        self.settle_reached_wheel_targets();
        self.wheel_target_x.is_some() || self.wheel_target_y.is_some()
    }

    /// Advances the smoothed wheel offsets toward their targets with a
    /// frame-rate-independent exponential approach and returns whether the
    /// animation still needs more frames.
    fn tick_smooth_scroll(&mut self, now: Instant) -> bool {
        if self.wheel_target_x.is_none() && self.wheel_target_y.is_none() {
            self.wheel_last_tick = None;
            return false;
        }
        let dt = self
            .wheel_last_tick
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64());
        self.wheel_last_tick = Some(now);
        let blend = 1.0 - (-dt / WHEEL_SMOOTHING_TAU).exp();
        if let Some(target) = self.wheel_target_x {
            self.offset_x += (target - self.offset_x) * blend;
        }
        if let Some(target) = self.wheel_target_y {
            self.offset_y += (target - self.offset_y) * blend;
        }
        self.settle_reached_wheel_targets();
        if self.wheel_target_x.is_none() && self.wheel_target_y.is_none() {
            self.wheel_last_tick = None;
            return false;
        }
        true
    }

    /// Snaps offsets that are within the settle epsilon of their wheel target
    /// and clears the finished targets.
    fn settle_reached_wheel_targets(&mut self) {
        if let Some(target) = self.wheel_target_x
            && (target - self.offset_x).abs() < WHEEL_SETTLE_EPSILON
        {
            self.offset_x = target;
            self.wheel_target_x = None;
        }
        if let Some(target) = self.wheel_target_y
            && (target - self.offset_y).abs() < WHEEL_SETTLE_EPSILON
        {
            self.offset_y = target;
            self.wheel_target_y = None;
        }
    }

    fn clamp_offsets(&mut self) {
        let metrics = self.metrics();
        self.offset_x = clamp_scroll_offset(self.offset_x, metrics.max_x);
        self.offset_y = clamp_scroll_offset(self.offset_y, metrics.max_y);
        self.wheel_target_x = self
            .wheel_target_x
            .map(|target| clamp_scroll_offset(target, metrics.max_x));
        self.wheel_target_y = self
            .wheel_target_y
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
    use core::time::Duration;

    fn vertical_handle() -> ScrollHandle {
        ScrollHandle::new(Axis::Vertical, 100.0, 100.0, 100.0, 300.0)
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
}
