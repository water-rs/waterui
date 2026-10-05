//! One module per ported component. Each owns its platform object, its
//! reactive updates and its layout face, and registers through
//! `install(&mut Dispatcher)` — the only symbol [`crate::registry`] reaches.

#[cfg(feature = "button")]
pub mod button;
#[cfg(feature = "container")]
pub mod container;
pub mod dynamic;
pub mod empty;
// The wrapper is a `RustViewGroup` — it shares the container port's group
// plumbing, so it builds only where that feature does.
#[cfg(feature = "container")]
pub mod ignore_safe_area;
#[cfg(feature = "text")]
pub mod text;
pub mod with_env;
