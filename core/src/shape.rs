//! Provides geometric shapes for UI elements
//!
//! This module contains various shape implementations that can be used to create
//! different UI elements. Each shape implements the `Shape` trait and can be
//! rendered in the UI.
//!
//! Available shapes include:
//! - `Rectangle`: A basic rectangular shape with no rounded corners
//! - `RoundedRectangle`: A rectangular shape with customizable rounded corners
//! - `Circle`: A perfect circular shape
//!
//! All shapes can be used directly with the rendering system.

use crate::raw_view;
use waterui_reactive::Computed;

/// Rectangle shape with no rounded corners
///
/// This shape can be used to create rectangular UI elements.
#[derive(Debug)]
#[must_use]
#[derive(uniffi::Record)]
pub struct Rectangle;

/// Rectangle shape with rounded corners
///
/// This shape can be used to create rectangular UI elements with rounded corners.
/// The corner radius is specified by the `radius` field.
#[derive(Debug)]
#[must_use]
pub struct RoundedRectangle {
    /// The radius of the rounded corners
    pub radius: Computed<f64>,
}

/// Circle shape
///
/// This shape can be used to create circular UI elements.
#[derive(Debug)]
#[must_use]
pub struct Circle;

/// Trait representing a shape that can be drawn
///
/// This trait is implemented by all shape types in this module.
pub trait Shape {}

/// Implements the Shape trait and raw_view for multiple types
///
/// This macro helps reduce boilerplate by implementing common functionality
/// across multiple shape types.
macro_rules! impl_shape {
    ($($ty:ty),*) => {
        $(
            raw_view!($ty);
            impl Shape for $ty {}
        )*
    };
}

impl_shape!(Rectangle, RoundedRectangle, Circle);
