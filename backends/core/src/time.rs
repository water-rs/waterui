//! Monotonic clock abstraction valid across native and web targets.
//!
//! Re-exports [`Instant`] from `std::time` on native targets and from
//! `web_time` on `wasm32`, where `std::time::Instant` panics. All renderer
//! timing — frame clocks, animation ticks, gesture deadlines — must use this
//! alias instead of naming `std::time::Instant` directly so the same code
//! compiles and runs on the web.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;
#[cfg(target_arch = "wasm32")]
pub use web_time::Instant;
