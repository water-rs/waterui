//! Shared types for the progress-indicator control twins.
//!
//! The platform indicators live in [`crate::appkit::Progress`] and
//! [`crate::uikit::Progress`]; this module carries the value their
//! signatures share.

/// Which shape the indicator draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgressVariant {
    /// A horizontal bar that fills toward a determinate reading.
    Linear,
    /// A round indicator: `AppKit`'s spinning-style `NSProgressIndicator`
    /// (a pie when determinate, a spinner when indeterminate), `UIKit`'s
    /// `UIActivityIndicatorView`.
    Circular,
}
