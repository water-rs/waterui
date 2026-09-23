use std::time::Duration;

use crate::selector::Selector;

/// XCTest-like expectation descriptor.
#[derive(Debug, Clone)]
pub struct Expectation {
    pub(crate) kind: ExpectationKind,
    pub(crate) inverted: bool,
}

impl Expectation {
    /// Marks this expectation as inverted.
    #[must_use]
    pub const fn inverted(mut self) -> Self {
        self.inverted = true;
        self
    }
}

#[derive(Debug, Clone)]
pub enum ExpectationKind {
    /// Wait for selector existence.
    Exists(Selector),
    /// Wait for selector absence.
    NotExists(Selector),
    /// Wait for selector value equality.
    ValueEquals { selector: Selector, value: String },
}

/// Wait behavior configuration.
#[derive(Debug, Clone, Copy)]
pub struct WaitOptions {
    /// Maximum time to wait before returning a timeout result.
    pub timeout: Duration,
    /// Whether non-inverted expectations must be fulfilled in list order.
    pub enforce_order: bool,
}

impl WaitOptions {
    /// Creates wait options with the provided timeout.
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            enforce_order: false,
        }
    }

    /// Sets whether expectations must be fulfilled in order.
    #[must_use]
    pub const fn enforce_order(mut self, enforce_order: bool) -> Self {
        self.enforce_order = enforce_order;
        self
    }
}

impl Default for WaitOptions {
    fn default() -> Self {
        Self::new(Duration::from_secs(2))
    }
}

/// XCTest-like wait result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitResult {
    /// All expectations were fulfilled.
    Completed,
    /// The wait timed out.
    TimedOut,
    /// A later expectation was fulfilled before an earlier ordered expectation.
    IncorrectOrder,
    /// An inverted expectation became true.
    InvertedFulfillment,
    /// The wait was interrupted.
    Interrupted,
}
