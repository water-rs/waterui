//! Layout integration with `waterui-layout`.
//!
//! This module provides the bridge between `WaterUI`'s layout system and GTK widgets:
//! - [`GtkSubView`] implements [`SubView`](waterui_core::layout::SubView) using GTK's measurement API
//! - [`place_children`] applies layout results to GTK widgets

pub mod placer;
pub mod subview;

pub use placer::{create_layout_container, place_children, update_positions};
pub use subview::{GtkSubView, stretch_axis_for_widget};
