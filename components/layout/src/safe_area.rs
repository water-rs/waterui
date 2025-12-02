//! Safe area handling for layout containers.
//!
//! WaterUI uses metadata to signal to native renderers which views should extend
//! into unsafe screen regions (areas obscured by notches, home indicators, status bars, etc.).
//!
//! # Architecture
//!
//! Safe area is entirely handled by the **native backend**. Rust code only provides
//! metadata hints via `IgnoreSafeArea`.
//!
//! # Native Backend Responsibilities
//!
//! The native renderer must:
//! 1. **Default behavior**: Apply platform safe area insets to all views
//! 2. **When encountering `IgnoreSafeArea` metadata**:
//!    - Ignore safe area constraints on the specified edges
//!    - Allow the view to extend edge-to-edge for those edges
//! 3. **Handle changes**: Re-layout when safe area changes (keyboard, rotation, etc.)

/// Specifies which edges should ignore safe area insets.
///
/// Used with `IgnoreSafeArea` to control which edges of a view
/// should extend into the unsafe screen regions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
/// ```ignore
/// // Extend background to fill entire screen
/// Color::blue()
///     .ignore_safe_area(EdgeSet::ALL)
///
/// // Only extend to top (under status bar)
/// header_view
///     .ignore_safe_area(EdgeSet::TOP)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IgnoreSafeArea {
    /// Which edges should ignore the safe area.
    pub edges: EdgeSet,
}

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
