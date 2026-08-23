//! Safe area handling for layout containers.
//!
//! `WaterUI` uses metadata to signal to native renderers which views should extend
//! into unsafe screen regions (areas obscured by notches, home indicators, status bars, etc.).
//!
//! # Architecture
//!
//! Placing native views against the device insets is the **native backend's**
//! job, and [`IgnoreSafeArea`] is the metadata hint that opts a view out of it.
//! Layers that `WaterUI` lays out itself — the window's snackbar and overlay
//! hosts, which a backend sees as one Rust-laid-out container — cannot be inset
//! from the outside, so a backend publishes the window's insets through
//! [`SafeAreaInsets`] and those layers pad themselves.
//!
//! # Native Backend Responsibilities
//!
//! The native renderer must:
//! 1. **Default behavior**: Apply platform safe area insets to all views
//! 2. **When encountering `IgnoreSafeArea` metadata**:
//!    - Ignore safe area constraints on the specified edges
//!    - Allow the view to extend edge-to-edge for those edges
//! 3. **Handle changes**: Re-layout when safe area changes (keyboard, rotation, etc.)

use nami::{Computed, SignalExt, signal::IntoComputed};
use waterui_core::{Environment, metadata::MetadataKey};

use super::padding::EdgeInsets;

/// Specifies which edges should ignore safe area insets.
///
/// Used with `IgnoreSafeArea` to control which edges of a view
/// should extend into the unsafe screen regions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct EdgeSet {
    /// Ignore safe area on the top edge.
    pub top: bool,
    /// Ignore safe area on the leading edge.
    pub leading: bool,
    /// Ignore safe area on the bottom edge.
    pub bottom: bool,
    /// Ignore safe area on the trailing edge.
    pub trailing: bool,
}

impl EdgeSet {
    /// All edges - ignore safe area on all sides.
    pub const ALL: Self = Self {
        top: true,
        leading: true,
        bottom: true,
        trailing: true,
    };

    /// No edges - respect safe area on all sides (default).
    pub const NONE: Self = Self {
        top: false,
        leading: false,
        bottom: false,
        trailing: false,
    };

    /// Horizontal edges only (leading and trailing).
    pub const HORIZONTAL: Self = Self {
        top: false,
        leading: true,
        bottom: false,
        trailing: true,
    };

    /// Vertical edges only (top and bottom).
    pub const VERTICAL: Self = Self {
        top: true,
        leading: false,
        bottom: true,
        trailing: false,
    };

    /// Top edge only.
    pub const TOP: Self = Self {
        top: true,
        leading: false,
        bottom: false,
        trailing: false,
    };

    /// Bottom edge only.
    pub const BOTTOM: Self = Self {
        top: false,
        leading: false,
        bottom: true,
        trailing: false,
    };

    /// Creates a custom edge set.
    #[must_use]
    #[allow(clippy::fn_params_excessive_bools)]
    pub const fn new(top: bool, leading: bool, bottom: bool, trailing: bool) -> Self {
        Self {
            top,
            leading,
            bottom,
            trailing,
        }
    }

    /// Returns true if any edge is set to ignore safe area.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.top || self.leading || self.bottom || self.trailing
    }

    /// Returns true if all edges are set to ignore safe area.
    #[must_use]
    pub const fn all(&self) -> bool {
        self.top && self.leading && self.bottom && self.trailing
    }
}

/// Marker metadata indicating this view should ignore safe area insets.
///
/// When a native renderer encounters this metadata, it should:
/// - In **propose phase**: Use full screen bounds (not safe bounds) for the specified edges
/// - In **place phase**: Position the view in full screen coordinates for the specified edges
///
/// This allows backgrounds, images, and other visual elements to extend
/// edge-to-edge while content remains in the safe area.
///
/// # Example
///
/// ```rust
/// use waterui::prelude::*;
///
/// // Extend background to fill entire screen
/// Color::blue().ignore_safe_area(EdgeSet::ALL);
///
/// // Only extend to top (under status bar)
/// let header_view = text!("Inbox");
/// header_view.ignore_safe_area(EdgeSet::TOP);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IgnoreSafeArea {
    /// Which edges should ignore the safe area.
    pub edges: EdgeSet,
}

impl MetadataKey for IgnoreSafeArea {}

impl IgnoreSafeArea {
    /// Creates a new `IgnoreSafeArea` with the specified edges.
    #[must_use]
    pub const fn new(edges: EdgeSet) -> Self {
        Self { edges }
    }

    /// Ignore safe area on all edges.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            edges: EdgeSet::ALL,
        }
    }

    /// Ignore safe area on vertical edges (top and bottom).
    #[must_use]
    pub const fn vertical() -> Self {
        Self {
            edges: EdgeSet::VERTICAL,
        }
    }

    /// Ignore safe area on horizontal edges (leading and trailing).
    #[must_use]
    pub const fn horizontal() -> Self {
        Self {
            edges: EdgeSet::HORIZONTAL,
        }
    }
}

/// The window's safe area, published by the platform backend.
///
/// A backend that knows its device insets — the notch, the status bar, the home
/// indicator — installs them here once per window and republishes on rotation.
/// Rust-side layers that the backend cannot inset for itself, such as the
/// window's snackbar and overlay hosts, read the value out of the environment
/// and pad themselves. Backends with no such concept install nothing and
/// [`SafeAreaInsets::resolve`] answers zero, which is the correct answer for a
/// desktop window.
///
/// This does not replace [`IgnoreSafeArea`]: that stays the hint a backend reads
/// to let a view span the full screen, and native chrome containers keep
/// insetting their own content. `SafeAreaInsets` exists for the layers `WaterUI`
/// lays out itself.
#[derive(Debug, Clone)]
pub struct SafeAreaInsets(Computed<EdgeInsets>);

impl SafeAreaInsets {
    /// Wraps a reactive inset signal.
    #[must_use]
    pub fn new(insets: impl IntoComputed<EdgeInsets>) -> Self {
        Self(insets.into_computed())
    }

    /// The reactive insets carried by this scope.
    #[must_use]
    pub const fn signal(&self) -> &Computed<EdgeInsets> {
        &self.0
    }

    /// The insets in force at this position, or zero when no backend published
    /// any.
    #[must_use]
    pub fn resolve(env: &Environment) -> Computed<EdgeInsets> {
        env.get::<Self>().map_or_else(
            || EdgeInsets::default().into_computed(),
            |scope| scope.0.clone(),
        )
    }

    /// The insets in force at this position plus a constant `margin`.
    ///
    /// This is what an overlay layer wants: clear of the hardware, and then
    /// clear of the window edge by the theme's own spacing.
    #[must_use]
    pub fn resolve_with_margin(env: &Environment, margin: EdgeInsets) -> Computed<EdgeInsets> {
        Self::resolve(env)
            .map(move |insets| insets + margin.clone())
            .into_computed()
    }

    /// Publishes `insets` to the subtree rendered from `env`.
    pub fn install(env: &mut Environment, insets: impl IntoComputed<EdgeInsets>) {
        env.insert(Self::new(insets));
    }
}

#[cfg(test)]
mod tests {
    use super::{EdgeInsets, SafeAreaInsets};
    use nami::{Signal, binding};
    use waterui_core::Environment;

    #[test]
    fn resolves_to_zero_when_no_backend_published_insets() {
        let env = Environment::new();
        assert_eq!(SafeAreaInsets::resolve(&env).get(), EdgeInsets::default());
    }

    #[test]
    fn resolved_insets_track_the_published_signal() {
        let published = binding(EdgeInsets::new(59.0, 34.0, 0.0, 0.0));
        let mut env = Environment::new();
        SafeAreaInsets::install(&mut env, published.clone());

        let resolved = SafeAreaInsets::resolve(&env);
        assert_eq!(resolved.get().top(), 59.0);

        // Rotation: the same signal reports the new insets without the
        // consuming subtree being rebuilt.
        published.set(EdgeInsets::new(0.0, 21.0, 59.0, 59.0));
        assert_eq!(resolved.get().top(), 0.0);
        assert_eq!(resolved.get().leading(), 59.0);
    }

    #[test]
    fn margin_stacks_on_top_of_the_published_insets() {
        let mut env = Environment::new();
        SafeAreaInsets::install(&mut env, EdgeInsets::new(59.0, 34.0, 0.0, 0.0));

        let padded = SafeAreaInsets::resolve_with_margin(&env, EdgeInsets::all(16.0));
        assert_eq!(padded.get(), EdgeInsets::new(75.0, 50.0, 16.0, 16.0));
    }

    #[test]
    fn margin_alone_applies_without_a_backend() {
        let env = Environment::new();
        let padded = SafeAreaInsets::resolve_with_margin(&env, EdgeInsets::all(16.0));
        assert_eq!(padded.get(), EdgeInsets::all(16.0));
    }
}
