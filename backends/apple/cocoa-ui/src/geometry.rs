//! Points, sizes, rectangles and insets, in points (not pixels).
//!
//! These are plain values shared by both platforms, and convert to and from
//! the Core Graphics types the frameworks use.

use objc2_core_foundation::{CGPoint, CGRect, CGSize};

/// A location in a two-dimensional coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// The horizontal coordinate.
    pub x: f64,
    /// The vertical coordinate.
    pub y: f64,
}

impl Point {
    /// The origin, `(0, 0)`.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// A point at `(x, y)`.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// A width and a height.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    /// The horizontal extent.
    pub width: f64,
    /// The vertical extent.
    pub height: f64,
}

impl Size {
    /// A size with no extent.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// A size of `width` by `height`.
    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// A rectangle: an origin and a size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// The corner the rectangle extends from.
    pub origin: Point,
    /// How far the rectangle extends from its origin.
    pub size: Size,
}

impl Rect {
    /// The empty rectangle at the origin.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    /// A rectangle at `(x, y)` of `width` by `height`.
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Point::new(x, y),
            size: Size::new(width, height),
        }
    }
}

/// A size suggestion handed to a view's measure handler: each axis is either
/// a bound it must fit inside or `None`, meaning unconstrained.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MeasureProposal {
    /// The horizontal bound, or `None` for unconstrained.
    pub width: Option<f64>,
    /// The vertical bound, or `None` for unconstrained.
    pub height: Option<f64>,
}

impl MeasureProposal {
    /// Every axis bounded by `size`.
    #[must_use]
    pub const fn fitted(size: Size) -> Self {
        Self {
            width: Some(size.width),
            height: Some(size.height),
        }
    }

    /// Only the width bounded.
    #[must_use]
    pub const fn width(width: f64) -> Self {
        Self {
            width: Some(width),
            height: None,
        }
    }

    /// No axis bounded.
    pub const UNBOUNDED: Self = Self {
        width: None,
        height: None,
    };
}

/// Distances inward from each edge of a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgeInsets {
    /// The inset from the top edge.
    pub top: f64,
    /// The inset from the left edge.
    pub left: f64,
    /// The inset from the bottom edge.
    pub bottom: f64,
    /// The inset from the right edge.
    pub right: f64,
}

impl EdgeInsets {
    /// No inset on any edge.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    /// Insets of `top`, `left`, `bottom` and `right`.
    #[must_use]
    pub const fn new(top: f64, left: f64, bottom: f64, right: f64) -> Self {
        Self {
            top,
            left,
            bottom,
            right,
        }
    }
}

/// Which of a view's four edges a flag applies to.
///
/// The kit stores `Edges` as a bitmask when a sibling backend reads it
/// through the `cocoaUiIgnoredSafeAreaEdges` selector: bit 0 top, bit 1
/// leading, bit 2 bottom, bit 3 trailing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct Edges {
    /// The top edge.
    pub top: bool,
    /// The leading edge — left in a left-to-right layout.
    pub leading: bool,
    /// The bottom edge.
    pub bottom: bool,
    /// The trailing edge — right in a left-to-right layout.
    pub trailing: bool,
}

impl Edges {
    /// No edges set.
    pub const NONE: Self = Self::new(false, false, false, false);

    /// Every edge set.
    pub const ALL: Self = Self::new(true, true, true, true);

    /// `Edges` of `top`, `leading`, `bottom` and `trailing`.
    #[must_use]
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "each edge is an independent on/off, the memberwise form `EdgeSet` itself spells"
    )]
    pub const fn new(top: bool, leading: bool, bottom: bool, trailing: bool) -> Self {
        Self {
            top,
            leading,
            bottom,
            trailing,
        }
    }

    /// The bitmask `cocoaUiIgnoredSafeAreaEdges` reports: bit 0 top, bit 1
    /// leading, bit 2 bottom, bit 3 trailing.
    #[must_use]
    pub const fn mask(self) -> u8 {
        (self.top as u8)
            | ((self.leading as u8) << 1)
            | ((self.bottom as u8) << 2)
            | ((self.trailing as u8) << 3)
    }
}

impl From<CGPoint> for Point {
    fn from(point: CGPoint) -> Self {
        Self::new(point.x, point.y)
    }
}

impl From<Point> for CGPoint {
    fn from(point: Point) -> Self {
        Self::new(point.x, point.y)
    }
}

impl From<objc2_foundation::NSEdgeInsets> for EdgeInsets {
    fn from(insets: objc2_foundation::NSEdgeInsets) -> Self {
        Self::new(insets.top, insets.left, insets.bottom, insets.right)
    }
}

impl From<CGSize> for Size {
    fn from(size: CGSize) -> Self {
        Self::new(size.width, size.height)
    }
}

impl From<Size> for CGSize {
    fn from(size: Size) -> Self {
        Self::new(size.width, size.height)
    }
}

impl Rect {
    /// Whether every component is finite and the size is non-negative — a
    /// rectangle a layout engine's answer can safely become a frame.
    #[must_use]
    pub fn is_valid_for_layout(self) -> bool {
        self.origin.x.is_finite()
            && self.origin.y.is_finite()
            && self.size.width.is_finite()
            && self.size.height.is_finite()
            && self.size.width >= 0.0
            && self.size.height >= 0.0
    }

    /// `self` snapped to a `scale`-times pixel grid.
    ///
    /// The origin snaps to its nearest pixel. A size that is already a whole
    /// number of pixels — within `1e-3`, absorbing the noise of
    /// `points * scale` on a value produced by dividing a whole pixel count
    /// by the same scale — keeps that count exactly; any other size falls
    /// out of the far edge's own snap, so siblings that shared an edge in a
    /// fractional layout answer keep sharing a pixel.
    #[must_use]
    pub fn pixel_snapped(self, scale: f64) -> Self {
        fn snapped_edge(origin: f64, extent: f64, scale: f64) -> (f64, f64) {
            let snap = |points: f64| (points * scale).round() / scale;
            let snapped_origin = snap(origin);
            let raw = extent * scale;
            let nearest = raw.round();
            if (raw - nearest).abs() < 1e-3 {
                return (snapped_origin, nearest / scale);
            }
            (snapped_origin, snap(origin + extent) - snapped_origin)
        }
        let (x, width) = snapped_edge(self.origin.x, self.size.width, scale);
        let (y, height) = snapped_edge(self.origin.y, self.size.height, scale);
        Self::new(x, y, width, height)
    }

    /// `self` grown to `chrome` on every edge where it touches `within`,
    /// within a half point.
    ///
    /// A content rect inset by a safe area: a child laid out inside `within`
    /// that manages its own obscured edges is extended through `within` to
    /// `chrome` on the edges it touches.
    #[must_use]
    pub fn extended_through(self, within: Self, chrome: Self) -> Self {
        let mut result = self;
        let (min_x, max_x) = (self.origin.x, self.origin.x + self.size.width);
        let (min_y, max_y) = (self.origin.y, self.origin.y + self.size.height);
        let (safe_min_x, safe_max_x) = (within.origin.x, within.origin.x + within.size.width);
        let (safe_min_y, safe_max_y) = (within.origin.y, within.origin.y + within.size.height);
        let (chrome_min_x, chrome_max_x) = (chrome.origin.x, chrome.origin.x + chrome.size.width);
        let (chrome_min_y, chrome_max_y) = (chrome.origin.y, chrome.origin.y + chrome.size.height);
        if (min_x - safe_min_x).abs() < 0.5 {
            result.origin.x = chrome_min_x;
            result.size.width += min_x - chrome_min_x;
        }
        if (max_x - safe_max_x).abs() < 0.5 {
            result.size.width += chrome_max_x - max_x;
        }
        if (min_y - safe_min_y).abs() < 0.5 {
            result.origin.y = chrome_min_y;
            result.size.height += min_y - chrome_min_y;
        }
        if (max_y - safe_max_y).abs() < 0.5 {
            result.size.height += chrome_max_y - max_y;
        }
        result
    }
}

impl From<CGRect> for Rect {
    fn from(rect: CGRect) -> Self {
        Self {
            origin: rect.origin.into(),
            size: rect.size.into(),
        }
    }
}

impl From<Rect> for CGRect {
    fn from(rect: Rect) -> Self {
        Self::new(rect.origin.into(), rect.size.into())
    }
}

/// The frame of an accessory anchored to a preview.
///
/// Centred on the preview's horizontal midpoint, above it with `gap` of air
/// when the space allows, below it when not, clamped inside `container`
/// with `edge_margin` on every side. Sizes wider or taller than the
/// container minus the margins are clipped first.
///
/// Written for a top-left coordinate space — "above" is
/// `preview.minY - gap - height`. For `AppKit`'s bottom-left screen
/// coordinates use [`anchored_screen_frame`].
#[must_use]
pub fn anchored_frame(
    preview: Rect,
    accessory: Size,
    container: Rect,
    gap: f64,
    edge_margin: f64,
) -> Rect {
    let double_margin = edge_margin * 2.0;
    let width = accessory
        .width
        .min((container.size.width - double_margin).max(0.0));
    let height = accessory
        .height
        .min((container.size.height - double_margin).max(0.0));
    let inner_min_x = container.origin.x + edge_margin;
    let inner_min_y = container.origin.y + edge_margin;
    let inner_max_x = container.origin.x + container.size.width - edge_margin;
    let inner_max_y = container.origin.y + container.size.height - edge_margin;
    let preview_mid_x = preview.origin.x + preview.size.width / 2.0;
    let preview_min_y = preview.origin.y;
    let preview_max_y = preview.origin.y + preview.size.height;

    let x = (preview_mid_x - width / 2.0)
        .max(inner_min_x)
        .min(inner_min_x.max(inner_max_x - width));

    let mut y = preview_min_y - gap - height;
    if y < inner_min_y {
        y = preview_max_y + gap;
    }
    if y + height > inner_max_y {
        y = inner_min_y.max(inner_max_y - height);
    }
    Rect::new(x, y, width, height)
}

/// [`anchored_frame`] in a bottom-left screen coordinate space.
///
/// Every rect is mirrored about `screen_bounds` (which the mirror leaves
/// unchanged), the top-left math runs, and the result is mirrored back.
/// "Above the preview" lands at `preview.maxY + gap`, where screen `y`
/// grows upward.
#[must_use]
pub fn anchored_screen_frame(
    preview: Rect,
    accessory: Size,
    screen_bounds: Rect,
    gap: f64,
    edge_margin: f64,
) -> Rect {
    let mirror = |rect: Rect| {
        Rect::new(
            rect.origin.x,
            screen_bounds.origin.y + screen_bounds.origin.y + screen_bounds.size.height
                - rect.origin.y
                - rect.size.height,
            rect.size.width,
            rect.size.height,
        )
    };
    mirror(anchored_frame(
        mirror(preview),
        accessory,
        screen_bounds,
        gap,
        edge_margin,
    ))
}

#[cfg(test)]
mod tests {
    use super::{Point, Rect, Size};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    #[test]
    fn rect_round_trips_through_core_graphics() {
        let rect = Rect::new(1.5, -2.0, 800.0, 600.25);
        let native = CGRect::from(rect);
        assert_eq!(native.origin, CGPoint::new(1.5, -2.0));
        assert_eq!(native.size, CGSize::new(800.0, 600.25));
        assert_eq!(Rect::from(native), rect);
    }

    #[test]
    fn point_and_size_keep_their_components() {
        assert_eq!(Point::from(CGPoint::new(3.0, 4.0)), Point::new(3.0, 4.0));
        assert_eq!(Size::from(CGSize::new(5.0, 6.0)), Size::new(5.0, 6.0));
        assert_eq!(Rect::ZERO, Rect::default());
    }
}

/// A two-component `(section, row)` coordinate a table view hands its
/// callbacks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IndexPath {
    /// The section component.
    pub section: usize,
    /// The row component.
    pub row: usize,
}
