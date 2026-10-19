//! Platform-agnostic input event vocabulary shared by self-drawn backends.

/// Gesture/touch lifecycle phase mapped from the platform event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed touch lifecycle that every platform event stream maps onto"
)]
pub enum TouchPhase {
    /// The pointer made contact or a button was pressed.
    Started,
    /// The pointer moved while active.
    Moved,
    /// The pointer lifted or the button was released.
    Ended,
    /// The platform cancelled the touch sequence (e.g. system gesture takeover).
    Cancelled,
}
