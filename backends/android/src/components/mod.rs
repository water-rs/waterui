//! One module per ported component. Each owns its platform object, its
//! reactive updates and its layout face, and registers through
//! `install(&mut Dispatcher)` — the only symbol [`crate::registry`] reaches.

#[cfg(feature = "button")]
pub mod button;
#[cfg(feature = "container")]
pub mod container;
pub mod dynamic;
pub mod empty;
#[cfg(feature = "text")]
pub mod text;
pub mod with_env;
