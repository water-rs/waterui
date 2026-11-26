//! Progress indicators for the `WaterUI` framework.
//!
//! This module provides components for showing progress indicators to users,
//! both as linear (bar) and circular progress indicators.
//!
//! # Examples
//!
//! ```
//! use waterui::prelude::*;
//!
//! // Create a basic linear progress indicator
//! let basic_progress = progress(0.75); // 75% complete
//!
//! // Create a circular progress indicator
//! let circular_progress = progress(0.5).circular();
//!
//! // Create a progress with custom label
//! let labeled_progress = progress(0.3).label("Downloading...");
//!
//! // Create an indeterminate loading indicator
//! let loading_indicator = Progress::infinity().circular();
//! ```

pub use waterui_controls::progress::{
    Progress, ProgressConfig, ProgressStyle, ProgressWithTotal, loading, progress,
};
