//! Haptic and sensory feedback primitives.

use executor_core::spawn_local;
pub use waterkit_haptic::{
    Haptic, HapticCapabilities, HapticError, HapticPattern, HapticPatternBuilder, HapticStep,
    Intensity,
};

/// Feedback kind played by `ViewExt::sensory_feedback`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Feedback {
    /// Notification feedback indicating a successful outcome.
    Success,
    /// Notification feedback indicating a warning.
    Warning,
    /// Notification feedback indicating an error.
    Error,
    /// Selection feedback indicating a discrete UI choice changed.
    Selection,
    /// Impact feedback with an explicit [`Intensity`].
    Impact(Intensity),
}

impl Feedback {
    /// Triggers this feedback on the platform's haptic engine without blocking the caller.
    #[doc(hidden)]
    pub fn trigger(self) {
        spawn_local(async move {
            let result = match self {
                Self::Success => Haptic::notification_success().await,
                Self::Warning => Haptic::notification_warning().await,
                Self::Error => Haptic::notification_error().await,
                Self::Selection => Haptic::selection().await,
                Self::Impact(intensity) => Haptic::impact(intensity).await,
            };
            if let Err(error) = result {
                tracing::debug!(%error, ?self, "failed to trigger sensory feedback");
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{Feedback, Haptic, Intensity};

    #[test]
    fn feedback_variants_cover_all_kinds() {
        let variants = [
            Feedback::Success,
            Feedback::Warning,
            Feedback::Error,
            Feedback::Selection,
            Feedback::Impact(Intensity::LOW),
        ];
        assert_eq!(variants[0], Feedback::Success);
        assert_ne!(variants[0], variants[4]);
        let _ = Haptic::capabilities;
    }
}
