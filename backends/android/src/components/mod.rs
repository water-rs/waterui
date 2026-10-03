//! One module per ported component. Each owns its platform object, its
//! reactive updates and its layout face, and registers through
//! `install(&mut Dispatcher)` — the only symbol [`crate::registry`] reaches.

#[cfg(feature = "button")]
pub(crate) mod button;
#[cfg(feature = "container")]
pub(crate) mod container;
pub(crate) mod empty;
#[cfg(feature = "text")]
pub(crate) mod text;
