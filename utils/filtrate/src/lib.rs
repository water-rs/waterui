//! GPU texture filters using wgpu.
//!
//! `filtrate` provides GPU-accelerated image filters that work with any wgpu texture.
//! It's designed to be standalone and usable outside of `WaterUI` - for images, video frames,
//! render targets, or any GPU texture.
//!
//! # Features
//!
//! - **Blur**: Gaussian blur with configurable radius
//! - **Brightness**: Adjust image brightness
//! - **Saturation**: Control color saturation
//! - **Contrast**: Adjust image contrast
//! - **Grayscale**: Convert to grayscale
//! - **Hue Rotation**: Rotate colors around the color wheel
//! - **Invert**: Invert all colors
//! - **Sepia**: Apply sepia tone effect
//! - **Vignette**: Add vignette effect
//! - **Sharpen**: Sharpen image details
//!
//! # Example
//!
//! ```ignore
//! use filtrate::{Blur, Brightness, FilterExt};
//! use waterui_graphics::FilterAdapter;
//!
//! // Chain filters together
//! let filter = Blur(5.0).then(Brightness(0.1));
//!
//! // Use FilterAdapter to apply to GPU textures
//! let adapter = FilterAdapter::new(filter);
//! ```

// Re-export core traits for convenience
pub use filtrate_core::{Chain, Filter, FilterExt, FragmentList, ParamArray};

// Re-export nami for Signal trait access
pub use filtrate_core::nami;

// Re-export all filter implementations
pub use filtrate_core::filters::{
    Blur, Brightness, Contrast, Grayscale, HueRotation, Invert, Saturation, Sepia, Sharpen,
    Vignette,
};
