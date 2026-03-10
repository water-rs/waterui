use std::time::Duration;

use crate::selector::Selector;

/// XCTest-like expectation descriptor.
#[derive(Debug, Clone)]
pub struct Expectation {
    pub(crate) kind: ExpectationKind,
    pub(crate) inverted: bool,
}

impl Expectation {
    #[must_use]
    pub const fn inverted(mut self) -> Self {
        self.inverted = true;
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ExpectationKind {
    Exists(Selector),
    NotExists(Selector),
    ValueEquals { selector: Selector, value: String },
}

/// Wait behavior configuration.
#[derive(Debug, Clone, Copy)]
pub struct WaitOptions {
    pub timeout: Duration,
    pub enforce_order: bool,
}

impl WaitOptions {
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            enforce_order: false,
        }
    }

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
    Completed,
    TimedOut,
    IncorrectOrder,
    InvertedFulfillment,
    Interrupted,
}
