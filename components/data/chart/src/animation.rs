//! Animation system for chart transitions.

use core::time::Duration;

/// Easing function type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum EasingType {
    /// Linear interpolation (constant speed).
    Linear = 0,
    /// Ease-in (starts slow, ends fast).
    EaseIn = 1,
    /// Ease-out (starts fast, ends slow).
    EaseOut = 2,
    /// Ease-in-out (slow at both ends).
    #[default]
    EaseInOut = 3,
    /// Spring physics (overshoots then settles).
    Spring = 4,
}

impl EasingType {
    /// Applies the easing function to a progress value (0.0 to 1.0).
    #[must_use]
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t * t,
            Self::EaseOut => {
                let inv = 1.0 - t;
                (inv * inv).mul_add(-inv, 1.0)
            }
            Self::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let inv = (-2.0f32).mul_add(t, 2.0);
                    1.0 - inv * inv * inv / 2.0
                }
            }
            Self::Spring => {
                // Damped spring approximation
                let omega = 14.0_f32; // Angular frequency
                let decay = (-5.0 * t).exp();
                decay.mul_add(-(omega * t).cos(), 1.0)
            }
        }
    }
}

/// Animation state passed to GPU shaders.
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct ChartAnimation {
    /// Current time since animation start (seconds).
    pub time: f32,
    /// Animation progress (0.0 to 1.0, with easing applied).
    pub progress: f32,
    /// Easing type as u32 for shader.
    pub easing: u32,
    /// Whether entry animation is active (1 = yes, 0 = no).
    pub entry_active: u32,
}

impl Default for ChartAnimation {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartAnimation {
    /// Creates a new animation state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            time: 0.0,
            progress: 1.0, // Start at completed state
            easing: EasingType::EaseInOut as u32,
            entry_active: 0,
        }
    }

    /// Returns true if the animation is complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.progress >= 1.0
    }
}

/// Manages animation state for chart transitions.
#[derive(Debug, Clone)]
pub struct ChartAnimator {
    /// Start instant (as duration since some epoch).
    start_time: Option<Duration>,
    /// Animation duration.
    duration: Duration,
    /// Easing function.
    easing: EasingType,
    /// Whether this is an entry animation.
    is_entry: bool,
    /// Current animation state.
    current: ChartAnimation,
}

impl Default for ChartAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartAnimator {
    /// Creates a new animator.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            start_time: None,
            duration: Duration::from_millis(300),
            easing: EasingType::EaseInOut,
            is_entry: false,
            current: ChartAnimation::new(),
        }
    }

    /// Starts a new animation.
    pub fn start(&mut self, now: Duration, duration: Duration, easing: EasingType, is_entry: bool) {
        if duration.is_zero() {
            self.start_time = None;
            self.duration = duration;
            self.easing = easing;
            self.is_entry = is_entry;
            self.current = ChartAnimation {
                time: 0.0,
                progress: 1.0,
                easing: easing as u32,
                entry_active: 0,
            };
            return;
        }

        self.start_time = Some(now);
        self.duration = duration;
        self.easing = easing;
        self.is_entry = is_entry;
        self.current = ChartAnimation {
            time: 0.0,
            progress: 0.0,
            easing: easing as u32,
            entry_active: u32::from(is_entry),
        };
    }

    /// Starts an entry animation (for when chart first appears).
    pub fn start_entry(&mut self, now: Duration, duration: Duration, easing: EasingType) {
        self.start(now, duration, easing, true);
    }

    /// Starts a data transition animation.
    pub fn start_transition(&mut self, now: Duration, duration: Duration, easing: EasingType) {
        self.start(now, duration, easing, false);
    }

    /// Updates animation state and returns current state for shader.
    pub fn update(&mut self, now: Duration) -> ChartAnimation {
        let Some(start) = self.start_time else {
            // No animation in progress
            return self.current;
        };

        let elapsed = now.saturating_sub(start);
        if self.duration.is_zero() {
            self.start_time = None;
            self.current = ChartAnimation {
                time: elapsed.as_secs_f32(),
                progress: 1.0,
                easing: self.easing as u32,
                entry_active: 0,
            };
            return self.current;
        }
        let raw_progress = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let progress = raw_progress.clamp(0.0, 1.0);

        // Apply easing
        let eased_progress = self.easing.apply(progress);

        self.current = ChartAnimation {
            time: elapsed.as_secs_f32(),
            progress: eased_progress,
            easing: self.easing as u32,
            entry_active: u32::from(self.is_entry),
        };

        // Clear animation when complete
        if progress >= 1.0 {
            self.start_time = None;
        }

        self.current
    }

    /// Returns true if an animation is in progress.
    #[must_use]
    pub const fn is_animating(&self) -> bool {
        self.start_time.is_some()
    }

    /// Returns the current animation state (without updating).
    #[must_use]
    pub const fn current(&self) -> ChartAnimation {
        self.current
    }

    /// Skips to end of animation.
    pub const fn finish(&mut self) {
        self.start_time = None;
        self.current.progress = 1.0;
        self.current.entry_active = 0;
    }
}

/// Animation configuration for charts.
#[derive(Debug, Clone)]
pub struct AnimationConfig {
    /// Duration of the animation.
    pub duration: Duration,
    /// Easing function.
    pub easing: EasingType,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            duration: Duration::from_millis(300),
            easing: EasingType::EaseInOut,
        }
    }
}

impl AnimationConfig {
    /// Creates a linear animation.
    #[must_use]
    pub const fn linear(duration: Duration) -> Self {
        Self {
            duration,
            easing: EasingType::Linear,
        }
    }

    /// Creates an ease-in animation.
    #[must_use]
    pub const fn ease_in(duration: Duration) -> Self {
        Self {
            duration,
            easing: EasingType::EaseIn,
        }
    }

    /// Creates an ease-out animation.
    #[must_use]
    pub const fn ease_out(duration: Duration) -> Self {
        Self {
            duration,
            easing: EasingType::EaseOut,
        }
    }

    /// Creates an ease-in-out animation.
    #[must_use]
    pub const fn ease_in_out(duration: Duration) -> Self {
        Self {
            duration,
            easing: EasingType::EaseInOut,
        }
    }

    /// Creates a spring animation.
    #[must_use]
    pub const fn spring(duration: Duration) -> Self {
        Self {
            duration,
            easing: EasingType::Spring,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_duration_transition_completes_immediately() {
        let mut animator = ChartAnimator::new();
        let now = Duration::from_millis(100);
        animator.start_transition(now, Duration::ZERO, EasingType::EaseInOut);
        let state = animator.update(now);
        assert!((state.progress - 1.0).abs() <= f32::EPSILON);
        assert!(!animator.is_animating());
    }
}
