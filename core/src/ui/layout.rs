//! Layout primitives and geometry types for the `WaterUI` layout system.
//!
//! # Logical Pixels (Points)
//!
//! All layout values in `WaterUI` use **logical pixels** (also called "points" or "dp").
//! This is the same unit system used by design tools like Figma, Sketch, and Adobe XD,
//! allowing seamless translation from design to implementation.
//!
//! - **1 logical pixel** = 1 point in design tools
//! - Native backends handle conversion to physical pixels based on screen density
//! - iOS: `UIKit` uses points natively (1pt = 1-3 physical pixels depending on device)
//! - Android: Backend converts dp to physical pixels using `displayMetrics.density`
//! - macOS: `AppKit` uses points (1pt = 1-2 physical pixels on Retina displays)
//!
//! This means `spacing: 8.0` or `width: 100.0` will appear the same physical size
//! across all platforms and screen densities.
//!
//! # Example
//!
//! ```ignore
//! // In Figma: Button with 16pt horizontal padding, 8pt vertical padding
//! // In WaterUI: Same values work directly
//! vstack((
//!     text("Hello").padding(16.0),  // 16 logical pixels = 16pt in Figma
//!     Divider,                       // 1pt thick line
//! )).spacing(8.0)                    // 8 logical pixels between items
//! ```

use core::any::Any;
use core::fmt;
use fmt::Debug;

use alloc::{rc::Rc, vec::Vec};
use nami::watcher::BoxWatcherGuard;

// ============================================================================
// StretchAxis - Specifies which axis a view stretches on
// ============================================================================

/// Specifies which axis (or axes) a view wants to stretch to fill available space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StretchAxis {
    /// No stretching - view uses its intrinsic size
    #[default]
    None,
    /// Stretch horizontally only (expand width, use intrinsic height)
    Horizontal,
    /// Stretch vertically only (expand height, use intrinsic width)
    Vertical,
    /// Stretch in both directions (expand width and height)
    Both,
    /// Stretch along the parent container's main axis.
    /// In `VStack`: expands vertically. In `HStack`: expands horizontally.
    /// Used by Spacer.
    MainAxis,
    /// Stretch along the parent container's cross axis.
    /// In `VStack`: expands horizontally. In `HStack`: expands vertically.
    CrossAxis,
}

impl StretchAxis {
    /// Returns true if this stretches horizontally.
    #[must_use]
    pub const fn stretches_horizontal(&self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    /// Returns true if this stretches vertically.
    #[must_use]
    pub const fn stretches_vertical(&self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    /// Returns true if this stretches in any direction.
    #[must_use]
    pub const fn stretches_any(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// How strongly a view holds on to space when its container runs short.
///
/// A stack takes space from its lowest-priority children first, and only starts
/// on the next band up once every child below sits at the minimum it reports.
/// Ties share the shortfall. Defaults to `0`, so raising one child's priority is
/// enough to protect it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LayoutPriority(i32);

impl LayoutPriority {
    /// Wraps a raw priority.
    #[must_use]
    pub const fn new(priority: i32) -> Self {
        Self(priority)
    }

    /// The raw priority.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}

impl crate::components::metadata::MetadataKey for LayoutPriority {}

// ============================================================================
// Alignment Guides
// ============================================================================

/// Stable identifier for a built-in alignment guide in the layout runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AlignmentKeyId {
    low: u64,
    high: u64,
}

impl AlignmentKeyId {
    /// Creates a stable alignment identifier from its low/high 64-bit halves.
    #[must_use]
    pub const fn new(low: u64, high: u64) -> Self {
        Self { low, high }
    }

    /// Returns the low 64 bits of the identifier.
    #[must_use]
    pub const fn low(self) -> u64 {
        self.low
    }

    /// Returns the high 64 bits of the identifier.
    #[must_use]
    pub const fn high(self) -> u64 {
        self.high
    }

    /// Creates an identifier from a stable string name using FNV-1a 128-bit hashing.
    #[must_use]
    pub const fn from_name(name: &str) -> Self {
        let hash = fnv1a_128(name.as_bytes());
        let bytes = hash.to_le_bytes();
        Self {
            low: u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
            high: u64::from_le_bytes([
                bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                bytes[15],
            ]),
        }
    }
}

const fn fnv1a_128(bytes: &[u8]) -> u128 {
    const FNV_OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const FNV_PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u128;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

#[derive(Clone, Copy)]
/// Horizontal alignment guide handle.
pub struct HorizontalAlignment {
    stable_id: AlignmentKeyId,
    default_value: fn(&ViewDimensions) -> f32,
}

impl HorizontalAlignment {
    /// Leading alignment guide.
    #[allow(non_upper_case_globals)]
    pub const Leading: Self = Self {
        stable_id: AlignmentKeyId::from_name("waterui.layout.horizontal.leading"),
        default_value: leading_alignment_default,
    };

    /// Center alignment guide.
    #[allow(non_upper_case_globals)]
    pub const Center: Self = Self {
        stable_id: AlignmentKeyId::from_name("waterui.layout.horizontal.center"),
        default_value: center_horizontal_alignment_default,
    };

    /// Trailing alignment guide.
    #[allow(non_upper_case_globals)]
    pub const Trailing: Self = Self {
        stable_id: AlignmentKeyId::from_name("waterui.layout.horizontal.trailing"),
        default_value: trailing_alignment_default,
    };

    #[must_use]
    /// Returns the stable identifier for this built-in horizontal alignment.
    pub const fn stable_id(self) -> AlignmentKeyId {
        self.stable_id
    }

    #[must_use]
    pub(crate) fn default_value(self, dimensions: &ViewDimensions) -> f32 {
        (self.default_value)(dimensions)
    }
}

impl Default for HorizontalAlignment {
    fn default() -> Self {
        Self::Center
    }
}

impl PartialEq for HorizontalAlignment {
    fn eq(&self, other: &Self) -> bool {
        self.stable_id == other.stable_id
    }
}

impl Eq for HorizontalAlignment {}

impl Debug for HorizontalAlignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HorizontalAlignment")
            .field("stable_id", &self.stable_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
/// Vertical alignment guide handle.
pub struct VerticalAlignment {
    stable_id: AlignmentKeyId,
    default_value: fn(&ViewDimensions) -> f32,
}

impl VerticalAlignment {
    /// Top alignment guide.
    #[allow(non_upper_case_globals)]
    pub const Top: Self = Self {
        stable_id: AlignmentKeyId::from_name("waterui.layout.vertical.top"),
        default_value: top_alignment_default,
    };

    /// Center alignment guide.
    #[allow(non_upper_case_globals)]
    pub const Center: Self = Self {
        stable_id: AlignmentKeyId::from_name("waterui.layout.vertical.center"),
        default_value: center_vertical_alignment_default,
    };

    /// Bottom alignment guide.
    #[allow(non_upper_case_globals)]
    pub const Bottom: Self = Self {
        stable_id: AlignmentKeyId::from_name("waterui.layout.vertical.bottom"),
        default_value: bottom_alignment_default,
    };

    /// First baseline alignment guide.
    #[allow(non_upper_case_globals)]
    pub const FirstBaseline: Self = Self {
        stable_id: AlignmentKeyId::from_name("waterui.layout.vertical.first_baseline"),
        default_value: first_baseline_alignment_default,
    };

    /// Last baseline alignment guide.
    #[allow(non_upper_case_globals)]
    pub const LastBaseline: Self = Self {
        stable_id: AlignmentKeyId::from_name("waterui.layout.vertical.last_baseline"),
        default_value: last_baseline_alignment_default,
    };

    #[must_use]
    /// Returns the stable identifier for this built-in vertical alignment.
    pub const fn stable_id(self) -> AlignmentKeyId {
        self.stable_id
    }

    #[must_use]
    pub(crate) fn default_value(self, dimensions: &ViewDimensions) -> f32 {
        (self.default_value)(dimensions)
    }
}

impl Default for VerticalAlignment {
    fn default() -> Self {
        Self::Center
    }
}

impl PartialEq for VerticalAlignment {
    fn eq(&self, other: &Self) -> bool {
        self.stable_id == other.stable_id
    }
}

impl Eq for VerticalAlignment {}

impl Debug for VerticalAlignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerticalAlignment")
            .field("stable_id", &self.stable_id)
            .finish_non_exhaustive()
    }
}

/// Combined two-dimensional alignment used by layout containers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alignment {
    horizontal: HorizontalAlignment,
    vertical: VerticalAlignment,
}

impl Alignment {
    /// Top-center alignment.
    #[allow(non_upper_case_globals)]
    pub const Top: Self = Self::new(HorizontalAlignment::Center, VerticalAlignment::Top);

    /// Top-leading alignment.
    #[allow(non_upper_case_globals)]
    pub const TopLeading: Self = Self::new(HorizontalAlignment::Leading, VerticalAlignment::Top);

    /// Top-trailing alignment.
    #[allow(non_upper_case_globals)]
    pub const TopTrailing: Self = Self::new(HorizontalAlignment::Trailing, VerticalAlignment::Top);

    /// Center alignment.
    #[allow(non_upper_case_globals)]
    pub const Center: Self = Self::new(HorizontalAlignment::Center, VerticalAlignment::Center);

    /// Leading-center alignment.
    #[allow(non_upper_case_globals)]
    pub const Leading: Self = Self::new(HorizontalAlignment::Leading, VerticalAlignment::Center);

    /// Trailing-center alignment.
    #[allow(non_upper_case_globals)]
    pub const Trailing: Self = Self::new(HorizontalAlignment::Trailing, VerticalAlignment::Center);

    /// Bottom-center alignment.
    #[allow(non_upper_case_globals)]
    pub const Bottom: Self = Self::new(HorizontalAlignment::Center, VerticalAlignment::Bottom);

    /// Bottom-leading alignment.
    #[allow(non_upper_case_globals)]
    pub const BottomLeading: Self =
        Self::new(HorizontalAlignment::Leading, VerticalAlignment::Bottom);

    /// Bottom-trailing alignment.
    #[allow(non_upper_case_globals)]
    pub const BottomTrailing: Self =
        Self::new(HorizontalAlignment::Trailing, VerticalAlignment::Bottom);

    /// Creates a combined alignment from horizontal and vertical guides.
    #[must_use]
    pub const fn new(horizontal: HorizontalAlignment, vertical: VerticalAlignment) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }

    /// Returns the horizontal component.
    #[must_use]
    pub const fn horizontal(&self) -> HorizontalAlignment {
        self.horizontal
    }

    /// Returns the vertical component.
    #[must_use]
    pub const fn vertical(&self) -> VerticalAlignment {
        self.vertical
    }
}

impl Default for Alignment {
    fn default() -> Self {
        Self::Center
    }
}

/// Measured dimensions together with explicit alignment guides.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ViewDimensions {
    /// The measured size.
    pub size: Size,
    explicit_horizontal_guides: Vec<(HorizontalAlignment, f32)>,
    explicit_vertical_guides: Vec<(VerticalAlignment, f32)>,
}

impl ViewDimensions {
    /// Creates dimensions for the provided size.
    #[must_use]
    pub const fn new(size: Size) -> Self {
        Self {
            size,
            explicit_horizontal_guides: Vec::new(),
            explicit_vertical_guides: Vec::new(),
        }
    }

    /// Returns the resolved horizontal guide value.
    #[must_use]
    pub fn horizontal(&self, alignment: HorizontalAlignment) -> f32 {
        self.explicit_horizontal(alignment)
            .unwrap_or_else(|| alignment.default_value(self))
    }

    /// Returns the resolved vertical guide value.
    #[must_use]
    pub fn vertical(&self, alignment: VerticalAlignment) -> f32 {
        self.explicit_vertical(alignment)
            .unwrap_or_else(|| alignment.default_value(self))
    }

    /// Returns the explicit horizontal guide value, if any.
    #[must_use]
    pub fn explicit_horizontal(&self, alignment: HorizontalAlignment) -> Option<f32> {
        self.explicit_horizontal_guides
            .iter()
            .rev()
            .find_map(|(guide, value)| (*guide == alignment).then_some(*value))
    }

    /// Returns the explicit vertical guide value, if any.
    #[must_use]
    pub fn explicit_vertical(&self, alignment: VerticalAlignment) -> Option<f32> {
        self.explicit_vertical_guides
            .iter()
            .rev()
            .find_map(|(guide, value)| (*guide == alignment).then_some(*value))
    }

    /// Returns an iterator over explicit horizontal guides.
    pub fn explicit_horizontal_guides(
        &self,
    ) -> impl Iterator<Item = (HorizontalAlignment, f32)> + '_ {
        self.explicit_horizontal_guides.iter().copied()
    }

    /// Returns an iterator over explicit vertical guides.
    pub fn explicit_vertical_guides(&self) -> impl Iterator<Item = (VerticalAlignment, f32)> + '_ {
        self.explicit_vertical_guides.iter().copied()
    }

    /// Stores an explicit horizontal guide value.
    pub fn set_horizontal(&mut self, alignment: HorizontalAlignment, value: f32) {
        self.explicit_horizontal_guides.push((alignment, value));
    }

    /// Stores an explicit vertical guide value.
    pub fn set_vertical(&mut self, alignment: VerticalAlignment, value: f32) {
        self.explicit_vertical_guides.push((alignment, value));
    }

    /// Builder-style horizontal guide setter.
    #[must_use]
    pub fn with_horizontal(mut self, alignment: HorizontalAlignment, value: f32) -> Self {
        self.set_horizontal(alignment, value);
        self
    }

    /// Builder-style vertical guide setter.
    #[must_use]
    pub fn with_vertical(mut self, alignment: VerticalAlignment, value: f32) -> Self {
        self.set_vertical(alignment, value);
        self
    }
}

/// A child view together with its placed frame inside a container.
#[derive(Clone, Copy)]
pub struct PlacedSubview<'a> {
    /// The child proxy.
    pub view: &'a dyn SubView,
    /// The child frame in container-local coordinates.
    pub frame: Rect,
}

impl Debug for PlacedSubview<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlacedSubview")
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}

impl<'a> PlacedSubview<'a> {
    /// Creates a placed subview.
    #[must_use]
    pub const fn new(view: &'a dyn SubView, frame: Rect) -> Self {
        Self { view, frame }
    }

    /// Returns the child's dimensions for its placed size proposal.
    #[must_use]
    pub fn dimensions(&self) -> ViewDimensions {
        self.view.measure(ProposalSize::new(
            Some(self.frame.width()),
            Some(self.frame.height()),
        ))
    }

    /// Returns the resolved horizontal guide in container coordinates.
    #[must_use]
    pub fn horizontal(&self, alignment: HorizontalAlignment) -> f32 {
        self.frame.x() + self.dimensions().horizontal(alignment)
    }

    /// Returns the resolved vertical guide in container coordinates.
    #[must_use]
    pub fn vertical(&self, alignment: VerticalAlignment) -> f32 {
        self.frame.y() + self.dimensions().vertical(alignment)
    }

    /// Returns the explicit horizontal guide in container coordinates, if any.
    #[must_use]
    pub fn explicit_horizontal(&self, alignment: HorizontalAlignment) -> Option<f32> {
        self.dimensions()
            .explicit_horizontal(alignment)
            .map(|value| self.frame.x() + value)
    }

    /// Returns the explicit vertical guide in container coordinates, if any.
    #[must_use]
    pub fn explicit_vertical(&self, alignment: VerticalAlignment) -> Option<f32> {
        self.dimensions()
            .explicit_vertical(alignment)
            .map(|value| self.frame.y() + value)
    }
}

const fn leading_alignment_default(dimensions: &ViewDimensions) -> f32 {
    let _ = dimensions;
    0.0
}

const fn center_horizontal_alignment_default(dimensions: &ViewDimensions) -> f32 {
    dimensions.size.width * 0.5
}

const fn trailing_alignment_default(dimensions: &ViewDimensions) -> f32 {
    dimensions.size.width
}

const fn top_alignment_default(dimensions: &ViewDimensions) -> f32 {
    let _ = dimensions;
    0.0
}

const fn center_vertical_alignment_default(dimensions: &ViewDimensions) -> f32 {
    dimensions.size.height * 0.5
}

const fn bottom_alignment_default(dimensions: &ViewDimensions) -> f32 {
    dimensions.size.height
}

const fn first_baseline_alignment_default(dimensions: &ViewDimensions) -> f32 {
    dimensions.size.height
}

const fn last_baseline_alignment_default(dimensions: &ViewDimensions) -> f32 {
    dimensions.size.height
}

// ============================================================================
// SubView Trait - Child View Proxy
// ============================================================================

/// A proxy for querying child view sizes during layout.
///
/// This trait allows layout containers to negotiate with children by asking
/// "if I propose this size, how big would you be?" multiple times with
/// different proposals.
///
/// # Pure Functions
///
/// All methods are pure (take `&self`) with no side effects.
///
/// # Caching is the `SubView`'s responsibility, never the [`Layout`]'s
///
/// The [`Layout`] trait deliberately has **no** caching: containers probe their
/// children freely with many proposals. Any measurement caching must therefore be
/// owned by the `SubView` implementation itself (the leaf), not the container.
/// Expensive measures — text shaping above all — **must** cache.
///
/// # Measurement is single-threaded by contract
///
/// Measuring runs on whichever thread drives layout, so a `SubView` is neither
/// `Send` nor `Sync` and its cache may be a plain
/// [`RefCell`](core::cell::RefCell).
///
/// Measuring siblings on a worker pool was tried and removed. A container's
/// children number in the single digits and a cache-hit measure costs tens of
/// nanoseconds, while a fork-join costs tens of microseconds — and it was paid by
/// every container, including the majority with nothing to offload. Parallelism
/// belongs where the work is genuinely large and batched (a renderer pre-shaping
/// every visible text run for a frame), not inside the per-container measurement
/// loop.
pub trait SubView {
    /// Measure the child for a given proposal.
    ///
    /// This method may be called multiple times with different proposals
    /// to probe the child's flexibility:
    ///
    /// - `ProposalSize::new(None, None)` - ideal/intrinsic size
    /// - `ProposalSize::new(Some(0.0), None)` - minimum width
    /// - `ProposalSize::new(Some(f32::INFINITY), None)` - maximum width
    /// - `ProposalSize::new(Some(200.0), None)` - constrained width
    ///
    /// # The three-point contract
    ///
    /// Containers work out how far a child may shrink, and how far it wants to
    /// grow, from those first three answers, so on each axis they must satisfy
    /// `min <= ideal <= max`. Every extent is finite and non-negative, except a
    /// maximum, where `f32::INFINITY` means unbounded — that is how a view says
    /// it will take whatever it is offered.
    ///
    /// Breaking this is not a local error: a minimum above the ideal makes a
    /// stack compress a child past a size it cannot take, and an ideal above the
    /// maximum makes it stretch one past a size it cannot take. Both surface as
    /// a misplaced layout somewhere else entirely.
    #[must_use]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions;

    /// Which axis (or axes) this view stretches to fill available space.
    ///
    /// - `StretchAxis::None`: Content-sized, uses intrinsic size
    /// - `StretchAxis::Horizontal`: Expands width only (e.g., `TextField`, Slider)
    /// - `StretchAxis::Vertical`: Expands height only
    /// - `StretchAxis::Both`: Greedy, fills all space (e.g., Spacer, Color)
    ///
    /// Layout containers use this to distribute remaining space appropriately:
    /// - `VStack` checks `stretches_vertical()` for height distribution
    /// - `HStack` checks `stretches_horizontal()` for width distribution
    fn stretch_axis(&self) -> StretchAxis;

    /// Layout priority for space distribution.
    ///
    /// Higher priority views are measured first and get space preference.
    fn priority(&self) -> i32;
}

/// A [`SubView`] that remembers what each proposal measured.
///
/// Containers probe the same child repeatedly — `size_that_fits` measures at the
/// ideal proposal, `place` measures again at the resolved bounds, alignment-guide
/// resolution measures once more per guide — and a container that is itself a child
/// re-runs all of that for its own parent's probes, so the repeats multiply with
/// tree depth. Wrapping each child for the duration of one pass collapses them to
/// one measure per distinct proposal.
///
/// The cache is a fixed-capacity inline array, scanned linearly: a pass probes any
/// one child a handful of times, and keeping it inline means wrapping a child costs
/// no allocation — which matters on the embedded backend, whose budget is heap
/// traffic per frame rather than CPU. A child probed at more than
/// [`MEMOIZED_PROPOSALS`] distinct proposals simply measures again for the extras.
/// Entries are keyed on the proposal's bits, so `-0.0` and a NaN proposal compare
/// consistently instead of by float equality.
pub struct MemoizedSubView<'a> {
    inner: &'a dyn SubView,
    cache: core::cell::RefCell<[Option<(ProposalSize, ViewDimensions)>; MEMOIZED_PROPOSALS]>,
}

/// How many distinct proposals one child's memo retains.
///
/// A container probes a child at its ideal size, at the resolved bounds, and —
/// once the distribution algorithm asks — at its minimum and maximum, so four
/// covers a pass without spilling.
pub const MEMOIZED_PROPOSALS: usize = 4;

impl<'a> MemoizedSubView<'a> {
    /// Wraps `inner` with a cache that lives as long as this value.
    #[must_use]
    pub fn new(inner: &'a dyn SubView) -> Self {
        Self {
            inner,
            cache: core::cell::RefCell::new([const { None }; MEMOIZED_PROPOSALS]),
        }
    }
}

impl Debug for MemoizedSubView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoizedSubView").finish_non_exhaustive()
    }
}

const fn proposal_axis_bits(axis: Option<f32>) -> Option<u32> {
    match axis {
        Some(value) => Some(value.to_bits()),
        None => None,
    }
}

fn same_proposal(left: ProposalSize, right: ProposalSize) -> bool {
    proposal_axis_bits(left.width) == proposal_axis_bits(right.width)
        && proposal_axis_bits(left.height) == proposal_axis_bits(right.height)
}

impl SubView for MemoizedSubView<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        if let Some((_, dimensions)) = self
            .cache
            .borrow()
            .iter()
            .flatten()
            .find(|(cached, _)| same_proposal(*cached, proposal))
        {
            return dimensions.clone();
        }
        let dimensions = self.inner.measure(proposal);
        if let Some(slot) = self
            .cache
            .borrow_mut()
            .iter_mut()
            .find(|slot| slot.is_none())
        {
            *slot = Some((proposal, dimensions.clone()));
        }
        dimensions
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.inner.stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.inner.priority()
    }
}

/// Runs `pass` with every child wrapped in a [`MemoizedSubView`], so repeated
/// probes within that one layout pass measure each child at most once per
/// distinct proposal.
///
/// Call this at the boundary where a layout pass begins — driving a [`Layout`]
/// directly rather than through [`measure_layout`] means owning this yourself.
pub fn with_memoized_children<R>(
    children: &[&dyn SubView],
    pass: impl FnOnce(&[&dyn SubView]) -> R,
) -> R {
    let memoized: Vec<MemoizedSubView<'_>> =
        children.iter().copied().map(MemoizedSubView::new).collect();
    let refs: Vec<&dyn SubView> = memoized.iter().map(|child| child as &dyn SubView).collect();
    pass(&refs)
}

// ============================================================================
// Layout Trait - Container Layout
// ============================================================================

/// Callback used by reactive layouts to invalidate their native container.
#[doc(hidden)]
pub type LayoutInvalidationCallback = Rc<dyn Fn() + 'static>;

/// A layout algorithm for arranging child views.
///
/// Layouts receive a size proposal from their parent, query their children
/// to determine sizes, and then place children within the final bounds.
///
/// # Two-Phase Layout
///
/// 1. **Sizing** ([`size_that_fits`](Self::size_that_fits)): Determine how big
///    this container should be given a proposal
/// 2. **Placement** ([`place`](Self::place)): Position children within the
///    final bounds
///
/// # Note on Safe Area
///
/// Safe area handling is intentionally **not** part of the Layout trait.
/// Safe area is a platform-specific concept handled by backends. Views can
/// use the `IgnoresSafeArea` metadata to opt out of safe area insets.
pub trait Layout: Debug + Any {
    /// Calculate the size this layout wants given a proposal.
    ///
    /// The layout can query children multiple times with different proposals
    /// to determine optimal sizing.
    ///
    /// # Arguments
    ///
    /// * `proposal` - The size proposed by the parent
    /// * `children` - References to child proxies for size queries
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size;

    /// Place children within the given bounds.
    ///
    /// Called after sizing is complete. Returns a rect for each child
    /// specifying its position and size within `bounds`.
    ///
    /// # Arguments
    ///
    /// * `bounds` - The rectangle this layout should fill
    /// * `children` - References to child proxies (may query sizes again)
    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect>;

    /// Returns an explicit horizontal guide for this container, if any.
    fn explicit_horizontal(
        &self,
        _alignment: HorizontalAlignment,
        _bounds: Rect,
        _children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        None
    }

    /// Returns an explicit vertical guide for this container, if any.
    fn explicit_vertical(
        &self,
        _alignment: VerticalAlignment,
        _bounds: Rect,
        _children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        None
    }

    /// Returns the horizontal alignments this container may expose explicitly.
    fn explicit_horizontal_alignments(&self) -> Vec<HorizontalAlignment> {
        Vec::new()
    }

    /// Returns the vertical alignments this container may expose explicitly.
    fn explicit_vertical_alignments(&self) -> Vec<VerticalAlignment> {
        Vec::new()
    }

    /// Which axis this container stretches to fill available space, given what
    /// its children said about themselves.
    ///
    /// Stacks are content-sized, so they answer `None` and let a child that wants
    /// to fill say so. A layout that is transparent to its content — a background,
    /// an overlay, an alignment guide — answers with that content's axis, and a
    /// frame answers from its own constraints.
    ///
    /// `children` carries each child's own [`StretchAxis`] in order. Passing it is
    /// what lets a transparent layout answer from live state: without it such a
    /// layout has to copy its content's axis when it is built, and that copy goes
    /// stale the moment the content's own answer changes — the container then
    /// claims space its content no longer wants, or refuses space it now does.
    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        let _ = children;
        StretchAxis::None
    }

    /// Watches layout inputs whose changes require a new native layout pass.
    ///
    /// This is backend infrastructure. Layout implementations return guards for
    /// their precise reactive fields; native containers retain those guards for
    /// the layout object's lifetime.
    #[doc(hidden)]
    fn watch_invalidation(&self, _invalidate: LayoutInvalidationCallback) -> Vec<BoxWatcherGuard> {
        Vec::new()
    }
}

/// Measures a layout and resolves its explicit guides for the given children.
///
/// Children are memoized for the duration of the call (see
/// [`with_memoized_children`]): sizing, placement and guide resolution all probe
/// the same children, so without it each child is measured several times over.
#[must_use]
pub fn measure_layout(
    layout: &dyn Layout,
    proposal: ProposalSize,
    children: &[&dyn SubView],
) -> ViewDimensions {
    with_memoized_children(children, |children| {
        measure_layout_memoized(layout, proposal, children)
    })
}

fn measure_layout_memoized(
    layout: &dyn Layout,
    proposal: ProposalSize,
    children: &[&dyn SubView],
) -> ViewDimensions {
    let size = layout.size_that_fits(proposal, children);
    let bounds = Rect::from_size(size);
    let child_rects = layout.place(bounds, children);
    let placed_subviews: Vec<PlacedSubview<'_>> = children
        .iter()
        .zip(child_rects.iter().copied())
        .map(|(view, frame)| PlacedSubview::new(*view, frame))
        .collect();

    let mut dimensions = ViewDimensions::new(size);
    let mut horizontal_keys = layout.explicit_horizontal_alignments();
    let mut vertical_keys = layout.explicit_vertical_alignments();

    for child in &placed_subviews {
        let child_dimensions = child.dimensions();
        for (alignment, _) in child_dimensions.explicit_horizontal_guides() {
            if !horizontal_keys.contains(&alignment) {
                horizontal_keys.push(alignment);
            }
        }
        for (alignment, _) in child_dimensions.explicit_vertical_guides() {
            if !vertical_keys.contains(&alignment) {
                vertical_keys.push(alignment);
            }
        }
    }

    for alignment in horizontal_keys {
        if let Some(value) = layout.explicit_horizontal(alignment, bounds, &placed_subviews) {
            dimensions.set_horizontal(alignment, value);
        }
    }
    for alignment in vertical_keys {
        if let Some(value) = layout.explicit_vertical(alignment, bounds, &placed_subviews) {
            dimensions.set_vertical(alignment, value);
        }
    }

    dimensions
}

// ============================================================================
// Geometry Types
// ============================================================================

/// Axis-aligned rectangle relative to its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    origin: Point,
    size: Size,
}

impl Rect {
    /// Creates a new [`Rect`] with the provided `origin` and `size`.
    #[must_use]
    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    /// Creates a rectangle from origin (0, 0) with the given size.
    #[must_use]
    pub const fn from_size(size: Size) -> Self {
        Self {
            origin: Point::zero(),
            size,
        }
    }

    /// Returns the rectangle's origin (top-left corner).
    #[must_use]
    pub const fn origin(&self) -> Point {
        self.origin
    }

    /// Returns the rectangle's size.
    #[must_use]
    pub const fn size(&self) -> &Size {
        &self.size
    }

    /// Returns the rectangle's x-coordinate (left edge).
    #[must_use]
    pub const fn x(&self) -> f32 {
        self.origin.x
    }

    /// Returns the rectangle's y-coordinate (top edge).
    #[must_use]
    pub const fn y(&self) -> f32 {
        self.origin.y
    }

    /// Returns the rectangle's width.
    #[must_use]
    pub const fn width(&self) -> f32 {
        self.size.width
    }

    /// Returns the rectangle's height.
    #[must_use]
    pub const fn height(&self) -> f32 {
        self.size.height
    }

    /// Returns the minimum x-coordinate (left edge).
    #[must_use]
    pub const fn min_x(&self) -> f32 {
        self.origin.x
    }

    /// Returns the minimum y-coordinate (top edge).
    #[must_use]
    pub const fn min_y(&self) -> f32 {
        self.origin.y
    }

    /// Returns the maximum x-coordinate (right edge).
    #[must_use]
    pub const fn max_x(&self) -> f32 {
        self.origin.x + self.size.width
    }

    /// Returns the maximum y-coordinate (bottom edge).
    #[must_use]
    pub const fn max_y(&self) -> f32 {
        self.origin.y + self.size.height
    }

    /// Returns the midpoint x-coordinate.
    #[must_use]
    pub const fn mid_x(&self) -> f32 {
        self.origin.x + self.size.width / 2.0
    }

    /// Returns the midpoint y-coordinate.
    #[must_use]
    pub const fn mid_y(&self) -> f32 {
        self.origin.y + self.size.height / 2.0
    }

    /// Returns the center point of the rectangle.
    #[must_use]
    pub const fn center(&self) -> Point {
        Point::new(self.mid_x(), self.mid_y())
    }

    /// Inset the rectangle by the given amounts on each edge.
    #[must_use]
    pub fn inset(&self, top: f32, bottom: f32, leading: f32, trailing: f32) -> Self {
        Self::new(
            Point::new(self.origin.x + leading, self.origin.y + top),
            Size::new(
                (self.size.width - leading - trailing).max(0.0),
                (self.size.height - top - bottom).max(0.0),
            ),
        )
    }
}

// ============================================================================
// Size
// ============================================================================

/// Two-dimensional size expressed in points.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default)]
pub struct Size {
    /// The width in points.
    pub width: f32,
    /// The height in points.
    pub height: f32,
}

impl Size {
    /// Constructs a [`Size`] with the given `width` and `height`.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Creates a [`Size`] with zero width and height.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
        }
    }

    /// Returns true if both dimensions are zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.width == 0.0 && self.height == 0.0
    }
}

// ============================================================================
// Point
// ============================================================================

/// Absolute coordinate relative to a parent layout's origin.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Point {
    /// The x-coordinate in points.
    pub x: f32,
    /// The y-coordinate in points.
    pub y: f32,
}

impl Point {
    /// Constructs a [`Point`] at the given `x` and `y`.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Creates a [`Point`] at the origin (0, 0).
    #[must_use]
    pub const fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

impl From<(f32, f32)> for Point {
    fn from((x, y): (f32, f32)) -> Self {
        Self { x, y }
    }
}

impl From<[f32; 2]> for Point {
    fn from([x, y]: [f32; 2]) -> Self {
        Self { x, y }
    }
}

impl From<(f32, f32)> for Size {
    fn from((width, height): (f32, f32)) -> Self {
        Self { width, height }
    }
}

impl From<[f32; 2]> for Size {
    fn from([width, height]: [f32; 2]) -> Self {
        Self { width, height }
    }
}

// ============================================================================
// Vec2
// ============================================================================

/// Two-dimensional displacement vector (e.g. velocity, gravity, wind, offset).
///
/// Distinct from [`Point`], which represents an absolute position. A `Vec2`
/// encodes a delta and is the natural input for physics-style modifiers such
/// as `gravity(...)` and `wind(...)` on a particle system.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec2 {
    /// Displacement along the x axis.
    pub dx: f32,
    /// Displacement along the y axis.
    pub dy: f32,
}

impl Vec2 {
    /// Constructs a [`Vec2`] with the given components.
    #[must_use]
    pub const fn new(dx: f32, dy: f32) -> Self {
        Self { dx, dy }
    }

    /// The zero vector.
    pub const ZERO: Self = Self { dx: 0.0, dy: 0.0 };
}

impl From<(f32, f32)> for Vec2 {
    fn from((dx, dy): (f32, f32)) -> Self {
        Self { dx, dy }
    }
}

impl From<[f32; 2]> for Vec2 {
    fn from([dx, dy]: [f32; 2]) -> Self {
        Self { dx, dy }
    }
}

// ============================================================================
// UnitPoint
// ============================================================================

/// Normalized coordinates (0.0–1.0) for positioning and gradient endpoints.
///
/// Used to specify both anchor points on views and target positions in parents.
/// Values outside `0.0..=1.0` are valid and will position outside bounds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UnitPoint {
    /// X coordinate (0.0 = left edge, 1.0 = right edge).
    pub x: f32,
    /// Y coordinate (0.0 = top edge, 1.0 = bottom edge).
    pub y: f32,
}

impl UnitPoint {
    /// Top-left corner (0.0, 0.0).
    pub const TOP_LEADING: Self = Self { x: 0.0, y: 0.0 };
    /// Top center (0.5, 0.0).
    pub const TOP: Self = Self { x: 0.5, y: 0.0 };
    /// Top-right corner (1.0, 0.0).
    pub const TOP_TRAILING: Self = Self { x: 1.0, y: 0.0 };
    /// Left center (0.0, 0.5).
    pub const LEADING: Self = Self { x: 0.0, y: 0.5 };
    /// Center (0.5, 0.5).
    pub const CENTER: Self = Self { x: 0.5, y: 0.5 };
    /// Right center (1.0, 0.5).
    pub const TRAILING: Self = Self { x: 1.0, y: 0.5 };
    /// Bottom-left corner (0.0, 1.0).
    pub const BOTTOM_LEADING: Self = Self { x: 0.0, y: 1.0 };
    /// Bottom center (0.5, 1.0).
    pub const BOTTOM: Self = Self { x: 0.5, y: 1.0 };
    /// Bottom-right corner (1.0, 1.0).
    pub const BOTTOM_TRAILING: Self = Self { x: 1.0, y: 1.0 };

    /// Creates a custom unit point.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl From<(f32, f32)> for UnitPoint {
    fn from((x, y): (f32, f32)) -> Self {
        Self { x, y }
    }
}

impl From<[f32; 2]> for UnitPoint {
    fn from([x, y]: [f32; 2]) -> Self {
        Self { x, y }
    }
}

impl From<Alignment> for UnitPoint {
    fn from(alignment: Alignment) -> Self {
        let horizontal = alignment.horizontal();
        let vertical = alignment.vertical();
        if horizontal == HorizontalAlignment::Leading && vertical == VerticalAlignment::Top {
            Self::TOP_LEADING
        } else if horizontal == HorizontalAlignment::Trailing && vertical == VerticalAlignment::Top
        {
            Self::TOP_TRAILING
        } else if horizontal == HorizontalAlignment::Leading
            && vertical == VerticalAlignment::Bottom
        {
            Self::BOTTOM_LEADING
        } else if horizontal == HorizontalAlignment::Trailing
            && vertical == VerticalAlignment::Bottom
        {
            Self::BOTTOM_TRAILING
        } else if horizontal == HorizontalAlignment::Leading {
            Self::LEADING
        } else if horizontal == HorizontalAlignment::Trailing {
            Self::TRAILING
        } else if vertical == VerticalAlignment::Top {
            Self::TOP
        } else if vertical == VerticalAlignment::Bottom {
            Self::BOTTOM
        } else {
            Self::CENTER
        }
    }
}

// ============================================================================
// Affine2
// ============================================================================

/// 2D affine transform stored as a row-major 2x3 matrix.
///
/// The transform maps a point `(x, y)` to:
///
/// ```text
/// x' = a * x + c * y + e
/// y' = b * x + d * y + f
/// ```
///
/// Layout matches the canonical 6-coefficient ordering used by `kurbo` and the
/// HTML Canvas 2D context, so a transform built here can be losslessly handed to
/// a 2D rendering backend.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Affine2 {
    /// Scale on the x axis.
    pub a: f32,
    /// Shear on the y axis (i.e. y component of the transformed x basis).
    pub b: f32,
    /// Shear on the x axis (i.e. x component of the transformed y basis).
    pub c: f32,
    /// Scale on the y axis.
    pub d: f32,
    /// Translation on the x axis.
    pub e: f32,
    /// Translation on the y axis.
    pub f: f32,
}

impl Affine2 {
    /// Identity transform: leaves any point unchanged.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Constructs an affine transform from raw coefficients.
    #[must_use]
    pub const fn new(
        scale_x: f32,
        shear_y: f32,
        shear_x: f32,
        scale_y: f32,
        translate_x: f32,
        translate_y: f32,
    ) -> Self {
        Self {
            a: scale_x,
            b: shear_y,
            c: shear_x,
            d: scale_y,
            e: translate_x,
            f: translate_y,
        }
    }

    /// Pure translation by `(tx, ty)`.
    #[must_use]
    pub const fn translate(tx: f32, ty: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    /// Non-uniform scale around the origin.
    #[must_use]
    pub const fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Rotation around the origin by `radians`.
    #[must_use]
    pub fn rotate(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Self {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }
}

impl From<[f32; 6]> for Affine2 {
    fn from(coefficients: [f32; 6]) -> Self {
        Self {
            a: coefficients[0],
            b: coefficients[1],
            c: coefficients[2],
            d: coefficients[3],
            e: coefficients[4],
            f: coefficients[5],
        }
    }
}

impl From<Affine2> for [f32; 6] {
    fn from(t: Affine2) -> Self {
        [t.a, t.b, t.c, t.d, t.e, t.f]
    }
}

macro_rules! impl_layout_signal_constant {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl nami::Signal for $ty {
                type Output = Self;
                type Guard = ();

                fn get(&self) -> Self::Output {
                    *self
                }

                fn watch(
                    &self,
                    _watcher: impl Fn(nami::watcher::Context<Self::Output>) + 'static,
                ) {
                }
            }
        )+
    };
}

impl_layout_signal_constant!(
    Point,
    Size,
    Rect,
    Vec2,
    UnitPoint,
    Affine2,
    HorizontalAlignment,
    VerticalAlignment,
    Alignment
);

// ============================================================================
// ProposalSize
// ============================================================================

/// A size proposal from parent to child during layout negotiation.
///
/// Each dimension can be:
/// - `None` - "Tell me your ideal size" (unspecified)
/// - `Some(0.0)` - "Tell me your minimum size"
/// - `Some(f32::INFINITY)` - "Tell me your maximum size"
/// - `Some(value)` - "I suggest you use this size"
///
/// Children are free to return any size; the proposal is just a suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ProposalSize {
    /// Width proposal: `None` = unspecified, `Some(f32)` = suggested width
    pub width: Option<f32>,
    /// Height proposal: `None` = unspecified, `Some(f32)` = suggested height
    pub height: Option<f32>,
}

impl ProposalSize {
    /// Creates a [`ProposalSize`] from optional width and height.
    #[must_use]
    pub fn new(width: impl Into<Option<f32>>, height: impl Into<Option<f32>>) -> Self {
        Self {
            width: width.into(),
            height: height.into(),
        }
    }

    /// Unspecified proposal - asks for ideal/intrinsic size.
    pub const UNSPECIFIED: Self = Self {
        width: None,
        height: None,
    };

    /// Zero proposal - asks for minimum size.
    pub const ZERO: Self = Self {
        width: Some(0.0),
        height: Some(0.0),
    };

    /// Infinite proposal - asks for maximum size.
    pub const INFINITY: Self = Self {
        width: Some(f32::INFINITY),
        height: Some(f32::INFINITY),
    };

    /// Returns the width or a default value if unspecified.
    #[must_use]
    pub fn width_or(&self, default: f32) -> f32 {
        self.width.unwrap_or(default)
    }

    /// Returns the height or a default value if unspecified.
    #[must_use]
    pub fn height_or(&self, default: f32) -> f32 {
        self.height.unwrap_or(default)
    }

    /// Replace only the width, keeping the height.
    #[must_use]
    pub const fn with_width(self, width: Option<f32>) -> Self {
        Self {
            width,
            height: self.height,
        }
    }

    /// Replace only the height, keeping the width.
    #[must_use]
    pub const fn with_height(self, height: Option<f32>) -> Self {
        Self {
            width: self.width,
            height,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_geometry() {
        let rect = Rect::new(Point::new(10.0, 20.0), Size::new(100.0, 50.0));

        assert_eq!(rect.min_x(), 10.0);
        assert_eq!(rect.min_y(), 20.0);
        assert_eq!(rect.max_x(), 110.0);
        assert_eq!(rect.max_y(), 70.0);
        assert_eq!(rect.mid_x(), 60.0);
        assert_eq!(rect.mid_y(), 45.0);
        assert_eq!(rect.width(), 100.0);
        assert_eq!(rect.height(), 50.0);
    }

    #[test]
    fn test_rect_inset() {
        let rect = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let inset = rect.inset(10.0, 10.0, 20.0, 20.0);

        assert_eq!(inset.x(), 20.0);
        assert_eq!(inset.y(), 10.0);
        assert_eq!(inset.width(), 60.0);
        assert_eq!(inset.height(), 80.0);
    }

    #[test]
    fn test_proposal_size() {
        let proposal = ProposalSize::new(Some(100.0), None);

        assert_eq!(proposal.width_or(0.0), 100.0);
        assert_eq!(proposal.height_or(50.0), 50.0);

        let with_height = proposal.with_height(Some(200.0));
        assert_eq!(with_height.width, Some(100.0));
        assert_eq!(with_height.height, Some(200.0));
    }

    /// A child that records how many times it was actually measured.
    struct CountingSubView {
        measures: core::cell::Cell<usize>,
    }

    impl SubView for CountingSubView {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            self.measures.set(self.measures.get() + 1);
            ViewDimensions::new(Size::new(
                proposal.width_or(10.0),
                proposal.height_or(20.0),
            ))
        }

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn memoized_subview_measures_once_per_distinct_proposal() {
        let inner = CountingSubView {
            measures: core::cell::Cell::new(0),
        };
        let memo = MemoizedSubView::new(&inner);

        let ideal = ProposalSize::UNSPECIFIED;
        let constrained = ProposalSize::new(Some(80.0), None);

        for _ in 0..5 {
            assert_eq!(memo.measure(ideal).size, Size::new(10.0, 20.0));
            assert_eq!(memo.measure(constrained).size, Size::new(80.0, 20.0));
        }

        assert_eq!(
            inner.measures.get(),
            2,
            "ten probes over two distinct proposals must reach the child twice"
        );
    }

    #[test]
    fn memoized_subview_forwards_priority_and_stretch() {
        let inner = CountingSubView {
            measures: core::cell::Cell::new(0),
        };
        let memo = MemoizedSubView::new(&inner);

        assert_eq!(memo.priority(), inner.priority());
        assert_eq!(memo.stretch_axis(), inner.stretch_axis());
    }
}
