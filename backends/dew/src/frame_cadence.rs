//! Deadline-based frame cadence for Dew host and embedded loops.

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

/// Advances an absolute frame deadline without accumulating render-time drift.
#[derive(Debug, Clone, Copy)]
pub struct FrameCadence {
    interval: Duration,
    next_deadline: Instant,
}

impl FrameCadence {
    pub fn new(start: Instant, frames_per_second: NonZeroU32) -> Self {
        Self {
            interval: Duration::from_secs_f64(1.0 / f64::from(frames_per_second.get())),
            next_deadline: start,
        }
    }

    pub fn sixty_hz(start: Instant) -> Self {
        Self::new(
            start,
            NonZeroU32::new(60).expect("60 FPS cadence must be non-zero"),
        )
    }

    /// Returns the next absolute deadline after work finishing at `now`.
    ///
    /// Missed slots are skipped so an overloaded frame does not cause a burst
    /// of catch-up frames, while later frames remain aligned to the original
    /// cadence instead of drifting by their render duration.
    pub fn deadline_after_work(&mut self, now: Instant) -> Instant {
        self.next_deadline += self.interval;
        if self.next_deadline <= now {
            let overdue = now.duration_since(self.next_deadline);
            let skipped = overdue.as_nanos() / self.interval.as_nanos() + 1;
            self.next_deadline += self.interval
                * u32::try_from(skipped).expect("Dew frame cadence skipped-slot count overflow");
        }
        self.next_deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_time_does_not_accumulate_into_frame_period() {
        let start = Instant::now();
        let mut cadence = FrameCadence::sixty_hz(start);

        let deadline = cadence.deadline_after_work(start + Duration::from_millis(5));

        assert_eq!(
            deadline.duration_since(start),
            Duration::from_secs_f64(1.0 / 60.0)
        );
    }

    #[test]
    fn overloaded_frame_skips_elapsed_slots_without_drift() {
        let start = Instant::now();
        let mut cadence = FrameCadence::sixty_hz(start);

        let deadline = cadence.deadline_after_work(start + Duration::from_millis(40));

        assert_eq!(
            deadline.duration_since(start),
            Duration::from_secs_f64(1.0 / 60.0) * 3
        );
    }
}
