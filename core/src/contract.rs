//! Reporting of broken contracts: the panic-in-debug, log-in-release policy.

/// Reports a broken contract: a debug build panics with the message, a
/// release build logs it through `tracing::error!` and the caller continues.
///
/// Takes the arguments of [`format!`]. The build mode is that of the
/// *calling* crate (`cfg!(debug_assertions)` expands at the call site), and
/// callers need no `tracing` dependency.
///
/// # When to use it
///
/// Use it for a state that user code, a platform or external input can
/// produce — an unowned `.transition(...)` declaration, a failed pixel
/// capture, a malformed host message. A GUI framework must not crash a
/// shipped app, so in a release build the violation is logged and the caller
/// continues to a defined outcome (a dropped claim, a completed instance),
/// while a debug build panics so the violation is found in development.
///
/// Internal invariants — states that cannot occur by construction — keep
/// `unreachable!` and `expect`.
///
/// # Example
///
/// ```
/// use waterui_core::contract_violation;
///
/// /// Opacity is a fraction; an out-of-range value is a caller bug, and the
/// /// defined outcome in a release build is the nearest valid fraction.
/// fn opacity(value: f32) -> f32 {
///     if !(0.0..=1.0).contains(&value) {
///         contract_violation!("opacity {value} is outside 0.0..=1.0");
///         return value.clamp(0.0, 1.0);
///     }
///     value
/// }
///
/// assert_eq!(opacity(0.5), 0.5);
/// ```
#[macro_export]
macro_rules! contract_violation {
    ($($arg:tt)*) => {
        $crate::__contract_violation!(::core::cfg!(debug_assertions), $($arg)*)
    };
}

/// The body of [`contract_violation!`] with the build mode as an argument.
/// Both arms expand at the call site, so the panic location and the log
/// event's target, file and line are the caller's.
#[doc(hidden)]
#[macro_export]
macro_rules! __contract_violation {
    ($debug:expr, $($arg:tt)*) => {
        if $debug {
            ::core::panic!($($arg)*);
        } else {
            $crate::__private::tracing::error!($($arg)*);
        }
    };
}

#[doc(hidden)]
pub mod __private {
    pub use tracing;
}

#[cfg(test)]
mod tests {
    #[test]
    #[should_panic(expected = "contract broken: 7")]
    fn contract_violation_panics_in_debug_build() {
        contract_violation!("contract broken: {}", 7);
    }

    #[test]
    fn contract_violation_logs_in_release_build() {
        crate::__contract_violation!(false, "contract broken: {}", 7);
    }
}
