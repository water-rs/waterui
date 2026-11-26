//! Interaction, gesture, lifecycle/hover event, cursor, drag-and-drop, focus, and keyboard press semantics for `WaterUI`.

#![no_std]

extern crate alloc;

pub mod cursor;
pub mod drag_drop;
pub mod event;
pub mod focus;
pub mod gesture;
#[cfg(feature = "haptics")]
pub mod haptic;
pub mod interaction;
pub mod key;
