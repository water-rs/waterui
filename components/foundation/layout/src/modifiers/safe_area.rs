//! Safe area handling for layout containers.
//!
//! `WaterUI` uses metadata to signal to native renderers which views should extend
//! into unsafe screen regions (areas obscured by notches, home indicators, status
//! bars, or the software keyboard).
//!
//! # Architecture
//!
//! Placing views against the device insets is the **native backend's** job,
//! and [`IgnoreSafeArea`] is the metadata hint that opts a view out of it. The
//! safe area has two regions on each edge — *container* (system bars, display
//! cutouts, the home indicator, window chrome) and *keyboard* (the software
//! keyboard and other input-method surfaces) — and [`IgnoreSafeArea`] names the
//! regions and edges a view ignores; a stack lays its children out inside what
//! remains.
//!
//! A background whose content is a fill (a solid color, a gradient or a
//! material) extends past its frame to the window edge on every edge it
//! touches, without moving the content it backs; an `IgnoreSafeArea` on the
//! fill replaces that default with the regions and edges it names. A scroll
//! surface extends under the regions of the edges it touches, insets its
//! content by them, and scrolls the minimum distance that brings a focused
//! text field clear of the keyboard region; a chrome container extends its
//! bars under the regions they touch.
//! `docs/layout-spec.md` §7.1 is the normative statement.
//!
//! Nothing `WaterUI` lays out itself — the window's snackbar and overlay
//! hosts included — pads itself against the hardware, because the container
//! it lives in has already placed it clear of it.
//!
//! # Native Backend Responsibilities
//!
//! The native renderer must:
//! 1. **Default behavior**: Lay content out inside the platform safe area,
//!    extending the scroll surfaces and chrome containers that touch its edges
//! 2. **When encountering `IgnoreSafeArea` metadata**:
//!    - Ignore safe area constraints on the specified regions and edges
//!    - Allow the view to extend edge-to-edge for those regions and edges
//! 3. **Handle changes**: Re-layout when safe area changes (keyboard, rotation, etc.)

use waterui_core::metadata::MetadataKey;

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

/// Specifies which safe-area regions a view ignores.
///
/// The safe area has two regions on each edge: *container* — the system bars,
/// display cutouts, the home indicator and window chrome — and *keyboard* —
/// the software keyboard and other input-method surfaces. A region set always
/// names at least one region; [`ALL`](Self::ALL), [`CONTAINER`](Self::CONTAINER)
/// and [`KEYBOARD`](Self::KEYBOARD) are its only values, and
/// [`on`](Self::on) pairs one with the edges it is ignored on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeAreaRegions {
    container: bool,
    keyboard: bool,
}

impl SafeAreaRegions {
    /// Both regions — the container and the keyboard.
    pub const ALL: Self = Self {
        container: true,
        keyboard: true,
    };

    /// The container region only.
    pub const CONTAINER: Self = Self {
        container: true,
        keyboard: false,
    };

    /// The keyboard region only.
    pub const KEYBOARD: Self = Self {
        container: false,
        keyboard: true,
    };

    /// Whether the set names the container region: system bars, display
    /// cutouts, the home indicator, window chrome.
    #[must_use]
    pub const fn container(self) -> bool {
        self.container
    }

    /// Whether the set names the keyboard region: the software keyboard and
    /// other input-method surfaces.
    #[must_use]
    pub const fn keyboard(self) -> bool {
        self.keyboard
    }

    /// Names the edges this region set is ignored on — the canonical way to
    /// build an [`IgnoreSafeArea`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Lay content out under the keyboard but still above the container inset.
    /// text!("editor").ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM));
    /// ```
    #[must_use]
    pub const fn on(self, edges: EdgeSet) -> IgnoreSafeArea {
        IgnoreSafeArea {
            regions: self,
            edges,
        }
    }
}

/// Marker metadata indicating this view should ignore safe area insets.
///
/// `regions` names which safe-area regions are ignored — the container, the
/// keyboard, or both — and `edges` the edges they are ignored on. When a
/// native renderer encounters this metadata, it lays the view out clear of
/// every region it does not ignore:
/// - In **propose phase**: the unsafe bands of the ignored regions count as
///   layout space for the named edges.
/// - In **place phase**: the view reaches into the ignored regions' bands on
///   the named edges.
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
///
/// // Lay an editor under the keyboard but still above the container inset
/// let editor = text!("editor");
/// editor.ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IgnoreSafeArea {
    /// Which safe-area regions are ignored.
    pub regions: SafeAreaRegions,
    /// Which edges ignore the named regions.
    pub edges: EdgeSet,
}

impl MetadataKey for IgnoreSafeArea {}

impl From<EdgeSet> for IgnoreSafeArea {
    /// An `EdgeSet` alone ignores every region on its edges — the same
    /// covering `.ignore_safe_area(EdgeSet::ALL)` always gave.
    fn from(edges: EdgeSet) -> Self {
        Self {
            regions: SafeAreaRegions::ALL,
            edges,
        }
    }
}

impl IgnoreSafeArea {
    /// Creates a new `IgnoreSafeArea` ignoring every region on the specified
    /// edges.
    #[must_use]
    pub const fn new(edges: EdgeSet) -> Self {
        Self {
            regions: SafeAreaRegions::ALL,
            edges,
        }
    }

    /// Ignore every region on all edges.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            regions: SafeAreaRegions::ALL,
            edges: EdgeSet::ALL,
        }
    }

    /// Ignore every region on vertical edges (top and bottom).
    #[must_use]
    pub const fn vertical() -> Self {
        Self {
            regions: SafeAreaRegions::ALL,
            edges: EdgeSet::VERTICAL,
        }
    }

    /// Ignore every region on horizontal edges (leading and trailing).
    #[must_use]
    pub const fn horizontal() -> Self {
        Self {
            regions: SafeAreaRegions::ALL,
            edges: EdgeSet::HORIZONTAL,
        }
    }
}
