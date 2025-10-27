//! `WaterUI` Controls Components
//! This crate provides a set of common UI controls for building user interfaces with `WaterUI`.
//!

#![no_std]
extern crate alloc;

pub mod slider;

pub use slider::Slider;
pub mod text_field;
pub use text_field::{TextField, field};
pub mod toggle;
pub use toggle::{Toggle, toggle};

pub mod stepper;
pub use stepper::{Stepper, stepper};

pub mod button;
pub use button::{Button, button};
