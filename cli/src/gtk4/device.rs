//! GTK4 device - re-exports Local device.
//!
//! GTK4 apps run directly on the host machine, which is handled by the
//! shared `Local` device implementation in `crate::device`.

pub use crate::device::Local;
