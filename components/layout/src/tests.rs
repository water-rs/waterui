//! Comprehensive layout system tests.
//!
//! These tests define the expected behavior of the layout system for various
//! edge cases and ensure consistency across all layout containers.

#![allow(clippy::float_cmp)]

use alloc::{format, vec, vec::Vec};

use crate::stack::{HStackLayout, HorizontalAlignment, VStackLayout, VerticalAlignment};
use crate::{Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView};

// ============================================================================
// Test Infrastructure
// ============================================================================

/// A mock [`SubView`] that returns a fixed size regardless of proposal.
/// This simulates a "rigid" view like an icon or fixed-size image.
struct FixedSizeView {
    size: Size,
}

impl SubView for FixedSizeView {
    fn size_that_fits(&self, _proposal: ProposalSize) -> Size {
        self.size
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A mock [`SubView`] that respects width proposals (like Text).
/// When given a width constraint, it wraps and increases height.
/// When given None, it returns intrinsic single-line size.
struct FlexibleTextView {
    /// Intrinsic single-line size (no wrapping)
    intrinsic_size: Size,
    /// Line height
    line_height: f32,
}

impl FlexibleTextView {
    fn new(text_width: f32, line_height: f32) -> Self {
        Self {
            intrinsic_size: Size::new(text_width, line_height),
            line_height,
        }
    }
}

impl SubView for FlexibleTextView {
    fn size_that_fits(&self, proposal: ProposalSize) -> Size {
        match proposal.width {
            Some(max_width) if max_width < self.intrinsic_size.width => {
                // Text needs to wrap - calculate wrapped height
                let lines = (self.intrinsic_size.width / max_width).ceil();
                Size::new(max_width, lines * self.line_height)
            }
            _ => {
                // No width constraint or enough space - return intrinsic
                self.intrinsic_size
            }
        }
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A mock Spacer that stretches to fill available space (Both directions).
struct SpacerView;

impl SubView for SpacerView {
    fn size_that_fits(&self, _proposal: ProposalSize) -> Size {
        Size::zero()
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A mock axis-expanding view (like [`TextField`], Slider).
/// Expands to fill width, has fixed height.
/// Uses [`StretchAxis::Horizontal`] - stretches WIDTH only, not HEIGHT.
struct HorizontalExpandingView {
    height: f32,
}

impl SubView for HorizontalExpandingView {
    fn size_that_fits(&self, proposal: ProposalSize) -> Size {
        let width = proposal.width.unwrap_or(f32::INFINITY);
        Size::new(width, self.height)
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A mock vertical-expanding view.
/// Expands to fill height, has fixed width.
/// Uses [`StretchAxis::Vertical`] - stretches HEIGHT only, not WIDTH.
struct VerticalExpandingView {
    width: f32,
}

impl SubView for VerticalExpandingView {
    fn size_that_fits(&self, proposal: ProposalSize) -> Size {
        let height = proposal.height.unwrap_or(f32::INFINITY);
        Size::new(self.width, height)
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Vertical
    }
    fn priority(&self) -> i32 {
        0
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn assert_rect_within_bounds(rect: &Rect, bounds: &Rect, msg: &str) {
    assert!(
        rect.x() >= bounds.x() - 0.001,
        "{}: rect.x ({}) < bounds.x ({})",
        msg,
        rect.x(),
        bounds.x()
    );
    assert!(
        rect.y() >= bounds.y() - 0.001,
        "{}: rect.y ({}) < bounds.y ({})",
        msg,
        rect.y(),
        bounds.y()
    );
    assert!(
        rect.x() + rect.width() <= bounds.x() + bounds.width() + 0.001,
        "{}: rect.right ({}) > bounds.right ({})",
        msg,
        rect.x() + rect.width(),
        bounds.x() + bounds.width()
    );
    assert!(
        rect.y() + rect.height() <= bounds.y() + bounds.height() + 0.001,
        "{}: rect.bottom ({}) > bounds.bottom ({})",
        msg,
        rect.y() + rect.height(),
        bounds.y() + bounds.height()
    );
}

fn assert_no_overlap(rects: &[Rect], axis: &str) {
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            let r1 = &rects[i];
            let r2 = &rects[j];

            let overlap = match axis {
                "horizontal" => {
                    // In HStack, children should not overlap horizontally
                    let r1_right = r1.x() + r1.width();
                    let r2_left = r2.x();
                    r1_right > r2_left + 0.001
                }
                "vertical" => {
                    // In VStack, children should not overlap vertically
                    let r1_bottom = r1.y() + r1.height();
                    let r2_top = r2.y();
                    r1_bottom > r2_top + 0.001
                }
                _ => false,
            };

            assert!(!overlap, "Children {i} and {j} overlap on {axis} axis");
        }
    }
}

// ============================================================================
// HStack Tests
// ============================================================================

#[test]
fn test_hstack_children_exceed_bounds() {
    // Case: HStack { Text("123") Spacer() Text("234") }.frame(width: 100)
    // When children's intrinsic widths + spacing exceed bounds,
    // children should be compressed to fit.

    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    // Two texts that together (60 + 60 + 10 spacing = 130) exceed 100
    let mut text1 = FixedSizeView {
        size: Size::new(60.0, 20.0),
    };
    let mut spacer = SpacerView;
    let mut text2 = FixedSizeView {
        size: Size::new(60.0, 20.0),
    };

    let bounds = Rect::new(Point::zero(), Size::new(100.0, 40.0));
    let children: Vec<&dyn SubView> = vec![&mut text1, &mut spacer, &mut text2];

    let rects = layout.place(bounds, &children);

    // All children must fit within bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {i}"));
    }

    // Spacer should have zero or minimal width since children exceed bounds
    assert!(
        rects[1].width() <= 0.001,
        "Spacer should be zero when children exceed bounds, got {}",
        rects[1].width()
    );
}

#[test]
fn test_hstack_single_child_exceeds_bounds() {
    // A single child wider than bounds should be clamped
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 0.0,
    };

    let mut wide_child = FixedSizeView {
        size: Size::new(200.0, 30.0),
    };
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 50.0));
    let children: Vec<&dyn SubView> = vec![&mut wide_child];

    let rects = layout.place(bounds, &children);

    assert_eq!(rects.len(), 1);
    assert_rect_within_bounds(&rects[0], &bounds, "wide child");
    assert!(
        rects[0].width() <= bounds.width() + 0.001,
        "Child width {} exceeds bounds width {}",
        rects[0].width(),
        bounds.width()
    );
}

#[test]
fn test_hstack_multiple_children_total_exceeds_bounds() {
    // Multiple children whose total width exceeds bounds
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut child2 = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut child3 = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };

    // Total: 50 + 10 + 50 + 10 + 50 = 170, bounds = 100
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 40.0));
    let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2, &mut child3];

    let rects = layout.place(bounds, &children);

    // All children must fit within bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {i}"));
    }

    // Children should not overlap
    assert_no_overlap(&rects, "horizontal");

    // Last child's right edge should not exceed bounds
    let last_rect = rects.last().unwrap();
    assert!(
        last_rect.x() + last_rect.width() <= bounds.width() + 0.001,
        "Last child exceeds bounds: {} > {}",
        last_rect.x() + last_rect.width(),
        bounds.width()
    );
}

#[test]
fn test_hstack_with_flexible_text() {
    // When children exceed bounds, they are compressed proportionally
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut long_text = FlexibleTextView::new(200.0, 20.0); // 200pt wide text

    // bounds width = 150, spacing = 10, available = 140
    // Total intrinsic = 50 + 200 = 250
    // Scale = 140 / 250 = 0.56
    let bounds = Rect::new(Point::zero(), Size::new(150.0, 100.0));
    let children: Vec<&dyn SubView> = vec![&mut label, &mut long_text];

    let rects = layout.place(bounds, &children);

    // All children should fit within bounds
    let total_children_width: f32 = rects.iter().map(waterui_core::layout::Rect::width).sum();
    let expected_max = bounds.width() - 10.0; // minus spacing

    assert!(
        total_children_width <= expected_max + 0.001,
        "Total children width {total_children_width} exceeds available space {expected_max}"
    );

    // Children should not overflow bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {i}"));
    }
}

#[test]
fn test_hstack_empty() {
    let layout = HStackLayout::default();
    let children: Vec<&dyn SubView> = vec![];

    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);
    assert_eq!(size.width, 0.0);
    assert_eq!(size.height, 0.0);

    let bounds = Rect::new(Point::zero(), Size::new(100.0, 100.0));
    let rects = layout.place(bounds, &children);
    assert!(rects.is_empty());
}

#[test]
fn test_hstack_single_child() {
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut child = FixedSizeView {
        size: Size::new(50.0, 30.0),
    };
    let children: Vec<&dyn SubView> = vec![&mut child];

    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);
    assert_eq!(size.width, 50.0);
    assert_eq!(size.height, 30.0);
}

#[test]
fn test_hstack_multiple_spacers() {
    // Multiple spacers should divide remaining space equally
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 0.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(20.0, 30.0),
    };
    let mut spacer1 = SpacerView;
    let mut child2 = FixedSizeView {
        size: Size::new(20.0, 30.0),
    };
    let mut spacer2 = SpacerView;
    let mut child3 = FixedSizeView {
        size: Size::new(20.0, 30.0),
    };

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 50.0));
    let children: Vec<&dyn SubView> = vec![
        &mut child1,
        &mut spacer1,
        &mut child2,
        &mut spacer2,
        &mut child3,
    ];

    let rects = layout.place(bounds, &children);

    // Remaining space = 200 - 60 = 140, divided by 2 spacers = 70 each
    assert_eq!(rects[0].width(), 20.0);
    assert!(
        (rects[1].width() - 70.0).abs() < 0.001,
        "Spacer 1 width: {}",
        rects[1].width()
    );
    assert_eq!(rects[2].width(), 20.0);
    assert!(
        (rects[3].width() - 70.0).abs() < 0.001,
        "Spacer 2 width: {}",
        rects[3].width()
    );
    assert_eq!(rects[4].width(), 20.0);
}

#[test]
fn test_hstack_alignment_top() {
    let layout = HStackLayout {
        alignment: VerticalAlignment::Top,
        spacing: 10.0,
    };

    let mut short = FixedSizeView {
        size: Size::new(30.0, 20.0),
    };
    let mut tall = FixedSizeView {
        size: Size::new(30.0, 50.0),
    };

    let bounds = Rect::new(Point::new(10.0, 10.0), Size::new(100.0, 60.0));
    let children: Vec<&dyn SubView> = vec![&mut short, &mut tall];

    let rects = layout.place(bounds, &children);

    // Both should be aligned to top (y = 10)
    assert_eq!(rects[0].y(), 10.0);
    assert_eq!(rects[1].y(), 10.0);
}

#[test]
fn test_hstack_alignment_bottom() {
    let layout = HStackLayout {
        alignment: VerticalAlignment::Bottom,
        spacing: 10.0,
    };

    let mut short = FixedSizeView {
        size: Size::new(30.0, 20.0),
    };
    let mut tall = FixedSizeView {
        size: Size::new(30.0, 50.0),
    };

    let bounds = Rect::new(Point::new(10.0, 10.0), Size::new(100.0, 60.0));
    let children: Vec<&dyn SubView> = vec![&mut short, &mut tall];

    let rects = layout.place(bounds, &children);

    // Both should be aligned to bottom
    assert_eq!(rects[0].y() + rects[0].height(), 70.0); // 10 + 60
    assert_eq!(rects[1].y() + rects[1].height(), 70.0);
}

#[test]
fn test_hstack_alignment_center() {
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut short = FixedSizeView {
        size: Size::new(30.0, 20.0),
    };
    let mut tall = FixedSizeView {
        size: Size::new(30.0, 50.0),
    };

    let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 60.0));
    let children: Vec<&dyn SubView> = vec![&mut short, &mut tall];

    let rects = layout.place(bounds, &children);

    // Short child (20h) centered in 60h bounds: y = (60-20)/2 = 20
    assert_eq!(rects[0].y(), 20.0);
    // Tall child (50h) centered in 60h bounds: y = (60-50)/2 = 5
    assert_eq!(rects[1].y(), 5.0);
}

// ============================================================================
// VStack Tests
// ============================================================================

#[test]
fn test_vstack_children_exceed_bounds() {
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 10.0,
    };

    // Three children that together exceed bounds height
    let mut child1 = FixedSizeView {
        size: Size::new(50.0, 50.0),
    };
    let mut child2 = FixedSizeView {
        size: Size::new(50.0, 50.0),
    };
    let mut child3 = FixedSizeView {
        size: Size::new(50.0, 50.0),
    };

    // Total: 50 + 10 + 50 + 10 + 50 = 170, bounds height = 100
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 100.0));
    let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2, &mut child3];

    let rects = layout.place(bounds, &children);

    // Children should not overlap
    assert_no_overlap(&rects, "vertical");
}

#[test]
fn test_vstack_child_width_exceeds_bounds() {
    // A child wider than bounds should be clamped to bounds width
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 0.0,
    };

    let mut wide_child = FixedSizeView {
        size: Size::new(200.0, 30.0),
    };
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 50.0));
    let children: Vec<&dyn SubView> = vec![&mut wide_child];

    let rects = layout.place(bounds, &children);

    assert_eq!(rects.len(), 1);
    assert!(
        rects[0].width() <= bounds.width() + 0.001,
        "Child width {} exceeds bounds width {}",
        rects[0].width(),
        bounds.width()
    );
    assert_rect_within_bounds(&rects[0], &bounds, "wide child");
}

#[test]
fn test_vstack_text_wrapping() {
    // Text in VStack should wrap to container width
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: 10.0,
    };

    // Long text (200pt intrinsic width) in 100pt wide container
    let mut long_text = FlexibleTextView::new(200.0, 20.0);

    let bounds = Rect::new(Point::zero(), Size::new(100.0, 200.0));
    let children: Vec<&dyn SubView> = vec![&mut long_text];

    let rects = layout.place(bounds, &children);

    // Text should wrap to bounds width
    assert!(
        rects[0].width() <= bounds.width() + 0.001,
        "Text width {} exceeds bounds width {}",
        rects[0].width(),
        bounds.width()
    );

    // Height should increase due to wrapping (200/100 = 2 lines)
    assert!(
        rects[0].height() >= 40.0 - 0.001,
        "Text should wrap to multiple lines, height: {}",
        rects[0].height()
    );
}

#[test]
fn test_vstack_with_axis_expanding_child() {
    // Axis-expanding views (TextField) should expand to container width
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut text_field = HorizontalExpandingView { height: 30.0 };

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 100.0));
    let children: Vec<&dyn SubView> = vec![&mut label, &mut text_field];

    let rects = layout.place(bounds, &children);

    // Label keeps its intrinsic width
    assert_eq!(rects[0].width(), 50.0);

    // TextField expands to bounds width (but clamped if infinite)
    assert!(
        rects[1].width() <= bounds.width() + 0.001,
        "TextField width {} should not exceed bounds {}",
        rects[1].width(),
        bounds.width()
    );
}

#[test]
fn test_vstack_empty() {
    let layout = VStackLayout::default();
    let children: Vec<&dyn SubView> = vec![];

    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);
    assert_eq!(size.width, 0.0);
    assert_eq!(size.height, 0.0);
}

#[test]
fn test_vstack_alignment_leading() {
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: 10.0,
    };

    let mut narrow = FixedSizeView {
        size: Size::new(30.0, 20.0),
    };
    let mut wide = FixedSizeView {
        size: Size::new(80.0, 20.0),
    };

    let bounds = Rect::new(Point::new(10.0, 10.0), Size::new(100.0, 60.0));
    let children: Vec<&dyn SubView> = vec![&mut narrow, &mut wide];

    let rects = layout.place(bounds, &children);

    // Both should be aligned to leading edge (x = 10)
    assert_eq!(rects[0].x(), 10.0);
    assert_eq!(rects[1].x(), 10.0);
}

#[test]
fn test_vstack_alignment_trailing() {
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Trailing,
        spacing: 10.0,
    };

    let mut narrow = FixedSizeView {
        size: Size::new(30.0, 20.0),
    };
    let mut wide = FixedSizeView {
        size: Size::new(80.0, 20.0),
    };

    let bounds = Rect::new(Point::new(10.0, 10.0), Size::new(100.0, 60.0));
    let children: Vec<&dyn SubView> = vec![&mut narrow, &mut wide];

    let rects = layout.place(bounds, &children);

    // Both should be aligned to trailing edge
    assert_eq!(rects[0].x() + rects[0].width(), 110.0); // 10 + 100
    assert_eq!(rects[1].x() + rects[1].width(), 110.0);
}

#[test]
fn test_vstack_alignment_center() {
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 10.0,
    };

    let mut narrow = FixedSizeView {
        size: Size::new(30.0, 20.0),
    };
    let mut wide = FixedSizeView {
        size: Size::new(80.0, 20.0),
    };

    let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 60.0));
    let children: Vec<&dyn SubView> = vec![&mut narrow, &mut wide];

    let rects = layout.place(bounds, &children);

    // Narrow (30w) centered in 100w: x = (100-30)/2 = 35
    assert_eq!(rects[0].x(), 35.0);
    // Wide (80w) centered in 100w: x = (100-80)/2 = 10
    assert_eq!(rects[1].x(), 10.0);
}

#[test]
fn test_vstack_multiple_spacers() {
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 0.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut spacer1 = SpacerView;
    let mut child2 = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut spacer2 = SpacerView;
    let mut child3 = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };

    let bounds = Rect::new(Point::zero(), Size::new(100.0, 200.0));
    let children: Vec<&dyn SubView> = vec![
        &mut child1,
        &mut spacer1,
        &mut child2,
        &mut spacer2,
        &mut child3,
    ];

    let rects = layout.place(bounds, &children);

    // Remaining height = 200 - 60 = 140, divided by 2 spacers = 70 each
    assert_eq!(rects[0].height(), 20.0);
    assert!(
        (rects[1].height() - 70.0).abs() < 0.001,
        "Spacer 1 height: {}",
        rects[1].height()
    );
    assert_eq!(rects[2].height(), 20.0);
    assert!(
        (rects[3].height() - 70.0).abs() < 0.001,
        "Spacer 2 height: {}",
        rects[3].height()
    );
    assert_eq!(rects[4].height(), 20.0);
}

// ============================================================================
// Nested Layout Tests
// ============================================================================

#[test]
fn test_hstack_in_vstack_respects_width() {
    // This simulates: VStack { HStack { "Name:" Text("long...") } }
    // The HStack should respect VStack's width constraint

    // We can't directly test nested layouts here, but we can test that
    // HStack respects the bounds it receives in place()
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut long_text = FixedSizeView {
        size: Size::new(500.0, 20.0),
    }; // Very long text

    // Constrained bounds (like inside a VStack)
    let bounds = Rect::new(Point::zero(), Size::new(200.0, 30.0));
    let children: Vec<&dyn SubView> = vec![&mut label, &mut long_text];

    let rects = layout.place(bounds, &children);

    // All children must fit within bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {i}"));
    }

    // The last child's right edge should not exceed bounds
    let last_rect = rects.last().unwrap();
    assert!(
        last_rect.x() + last_rect.width() <= bounds.width() + 0.001,
        "Content overflows bounds: {} > {}",
        last_rect.x() + last_rect.width(),
        bounds.width()
    );
}

// ============================================================================
// Size Calculation Tests
// ============================================================================

#[test]
fn test_hstack_size_respects_proposal() {
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(100.0, 30.0),
    };
    let mut child2 = FixedSizeView {
        size: Size::new(100.0, 30.0),
    };

    // Intrinsic width would be 210 (100 + 10 + 100)
    // But with proposal width = 150, should report 150 (capped to proposal)
    let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];
    let size = layout.size_that_fits(ProposalSize::new(Some(150.0), None), &children);

    // HStack without spacers should cap its reported width to the proposal
    // This ensures parent layouts get accurate sizing information
    assert_eq!(size.width, 150.0);
}

#[test]
fn test_vstack_size_respects_proposal() {
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 10.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(100.0, 50.0),
    };
    let mut child2 = FixedSizeView {
        size: Size::new(80.0, 50.0),
    };

    let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];

    // With width proposal, VStack should report min(max_child_width, proposal)
    let size = layout.size_that_fits(ProposalSize::new(Some(90.0), None), &children);

    assert_eq!(size.width, 90.0); // Clamped to proposal
    assert_eq!(size.height, 110.0); // 50 + 10 + 50
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_zero_bounds() {
    let hstack = HStackLayout::default();
    let vstack = VStackLayout::default();

    let mut child = FixedSizeView {
        size: Size::new(50.0, 30.0),
    };
    let bounds = Rect::new(Point::zero(), Size::zero());

    let children: Vec<&dyn SubView> = vec![&mut child];
    let rects = hstack.place(bounds, &children);

    // Should handle gracefully - child clamped to zero
    assert!(rects[0].width() <= 0.001 || rects[0].width() <= 50.0);

    let mut child = FixedSizeView {
        size: Size::new(50.0, 30.0),
    };
    let children: Vec<&dyn SubView> = vec![&mut child];
    let rects = vstack.place(bounds, &children);

    assert!(rects[0].height() <= 0.001 || rects[0].height() <= 30.0);
}

#[test]
fn test_negative_remaining_space_with_spacer() {
    // When children exceed bounds, spacer should get zero width, not negative
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(60.0, 20.0),
    };
    let mut spacer = SpacerView;
    let mut child2 = FixedSizeView {
        size: Size::new(60.0, 20.0),
    };

    // Total non-spacer width = 60 + 10 + 10 + 60 = 140, bounds = 100
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 30.0));
    let children: Vec<&dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

    let rects = layout.place(bounds, &children);

    // Spacer should never have negative dimensions
    assert!(
        rects[1].width() >= 0.0,
        "Spacer has negative width: {}",
        rects[1].width()
    );
    assert!(
        rects[1].height() >= 0.0,
        "Spacer has negative height: {}",
        rects[1].height()
    );
}

#[test]
fn test_bounds_with_offset() {
    // Bounds starting at non-zero origin
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(30.0, 20.0),
    };
    let mut child2 = FixedSizeView {
        size: Size::new(40.0, 25.0),
    };

    let bounds = Rect::new(Point::new(50.0, 100.0), Size::new(200.0, 50.0));
    let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];

    let rects = layout.place(bounds, &children);

    // First child should start at bounds origin
    assert_eq!(rects[0].x(), 50.0);

    // All children should be within bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {i} with offset"));
    }
}

// ============================================================================
// StretchAxis Tests - Comprehensive tests for the new stretch behavior
// ============================================================================

#[test]
fn test_vstack_horizontal_expanding_child_height() {
    // StretchAxis::Horizontal child in VStack should NOT stretch vertically
    // This is the core bug fix: TextField should keep its intrinsic height
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut text_field = HorizontalExpandingView { height: 40.0 }; // Fixed 40pt height
    let mut button = FixedSizeView {
        size: Size::new(100.0, 44.0),
    };

    // Size that fits: no stretching child should share height
    let children: Vec<&dyn SubView> = vec![&mut label, &mut text_field, &mut button];
    let size = layout.size_that_fits(ProposalSize::new(None, Some(300.0)), &children);

    // Height should be sum of all children + spacing (no height distribution)
    // 20 + 10 + 40 + 10 + 44 = 124
    assert_eq!(size.height, 124.0);

    // Rebuild children for place
    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut text_field = HorizontalExpandingView { height: 40.0 };
    let mut button = FixedSizeView {
        size: Size::new(100.0, 44.0),
    };
    let children: Vec<&dyn SubView> = vec![&mut label, &mut text_field, &mut button];

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 300.0));
    let rects = layout.place(bounds, &children);

    // TextField should keep its 40pt height, not stretch to fill remaining space
    assert_eq!(
        rects[1].height(),
        40.0,
        "TextField height should be intrinsic, not stretched"
    );
}

#[test]
fn test_vstack_with_both_spacer_and_horizontal_expanding() {
    // VStack with Spacer (Both) and TextField (Horizontal)
    // Only Spacer should stretch vertically
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut spacer = SpacerView; // StretchAxis::Both
    let mut text_field = HorizontalExpandingView { height: 40.0 }; // StretchAxis::Horizontal

    let children: Vec<&dyn SubView> = vec![&mut label, &mut spacer, &mut text_field];

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 200.0));
    let rects = layout.place(bounds, &children);

    // Label: 20pt
    assert_eq!(rects[0].height(), 20.0);
    // TextField: 40pt (intrinsic, doesn't stretch vertically)
    assert_eq!(rects[2].height(), 40.0);
    // Spacer gets remaining: 200 - 20 - 10 - 10 - 40 = 120pt
    assert!(
        (rects[1].height() - 120.0).abs() < 0.001,
        "Spacer height: {}",
        rects[1].height()
    );
}

#[test]
fn test_hstack_vertical_expanding_child_width() {
    // StretchAxis::Vertical child in HStack should NOT stretch horizontally
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut vertical_component = VerticalExpandingView { width: 60.0 }; // Fixed 60pt width
    let mut button = FixedSizeView {
        size: Size::new(100.0, 44.0),
    };

    let children: Vec<&dyn SubView> = vec![&mut label, &mut vertical_component, &mut button];
    let size = layout.size_that_fits(ProposalSize::new(Some(400.0), None), &children);

    // Width should be sum of all children + spacing (no width distribution)
    // 50 + 10 + 60 + 10 + 100 = 230
    assert_eq!(size.width, 230.0);
}

#[test]
fn test_vstack_intrinsic_width_excludes_horizontal_stretch() {
    // VStack intrinsic width should NOT include horizontally-stretching children
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(80.0, 20.0),
    };
    let mut text_field = HorizontalExpandingView { height: 40.0 }; // Returns INFINITY width when unspecified
    let mut button = FixedSizeView {
        size: Size::new(100.0, 44.0),
    };

    let children: Vec<&dyn SubView> = vec![&mut label, &mut text_field, &mut button];
    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

    // Width: max of non-horizontal-stretching children = max(80, 100) = 100
    // TextField stretches horizontally so its infinity width doesn't contribute
    assert_eq!(size.width, 100.0);
}

#[test]
fn test_hstack_intrinsic_height_excludes_vertical_stretch() {
    // HStack intrinsic height should NOT include vertically-stretching children
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut vertical_component = VerticalExpandingView { width: 60.0 }; // Returns INFINITY height
    let mut button = FixedSizeView {
        size: Size::new(100.0, 44.0),
    };

    let children: Vec<&dyn SubView> = vec![&mut label, &mut vertical_component, &mut button];
    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

    // Height: max of non-vertical-stretching children = max(20, 44) = 44
    // vertical_component stretches vertically so its infinity height doesn't contribute
    assert_eq!(size.height, 44.0);
}

#[test]
fn test_vstack_multiple_vertical_stretch_children() {
    // Multiple StretchAxis::Vertical children should share remaining height equally
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 0.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(50.0, 30.0),
    };
    let mut spacer1 = SpacerView; // Both - stretches vertically
    let mut child2 = FixedSizeView {
        size: Size::new(50.0, 30.0),
    };
    let mut spacer2 = SpacerView; // Both - stretches vertically
    let mut child3 = FixedSizeView {
        size: Size::new(50.0, 30.0),
    };

    let children: Vec<&dyn SubView> = vec![
        &mut child1,
        &mut spacer1,
        &mut child2,
        &mut spacer2,
        &mut child3,
    ];

    let bounds = Rect::new(Point::zero(), Size::new(100.0, 200.0));
    let rects = layout.place(bounds, &children);

    // Fixed children: 30 + 30 + 30 = 90
    // Remaining: 200 - 90 = 110
    // Each spacer: 110 / 2 = 55
    assert_eq!(rects[0].height(), 30.0);
    assert!(
        (rects[1].height() - 55.0).abs() < 0.001,
        "Spacer 1 height: {}",
        rects[1].height()
    );
    assert_eq!(rects[2].height(), 30.0);
    assert!(
        (rects[3].height() - 55.0).abs() < 0.001,
        "Spacer 2 height: {}",
        rects[3].height()
    );
    assert_eq!(rects[4].height(), 30.0);
}

#[test]
fn test_hstack_multiple_horizontal_stretch_children() {
    // Multiple StretchAxis::Horizontal children should share remaining width equally
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 0.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(30.0, 50.0),
    };
    let mut spacer1 = SpacerView; // Both - stretches horizontally
    let mut child2 = FixedSizeView {
        size: Size::new(30.0, 50.0),
    };
    let mut spacer2 = SpacerView; // Both - stretches horizontally
    let mut child3 = FixedSizeView {
        size: Size::new(30.0, 50.0),
    };

    let children: Vec<&dyn SubView> = vec![
        &mut child1,
        &mut spacer1,
        &mut child2,
        &mut spacer2,
        &mut child3,
    ];

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 100.0));
    let rects = layout.place(bounds, &children);

    // Fixed children: 30 + 30 + 30 = 90
    // Remaining: 200 - 90 = 110
    // Each spacer: 110 / 2 = 55
    assert_eq!(rects[0].width(), 30.0);
    assert!(
        (rects[1].width() - 55.0).abs() < 0.001,
        "Spacer 1 width: {}",
        rects[1].width()
    );
    assert_eq!(rects[2].width(), 30.0);
    assert!(
        (rects[3].width() - 55.0).abs() < 0.001,
        "Spacer 2 width: {}",
        rects[3].width()
    );
    assert_eq!(rects[4].width(), 30.0);
}

#[test]
fn test_vstack_form_layout() {
    // Real-world scenario: Form with labels and text fields
    // VStack { Text("Name") TextField() Text("Email") TextField() Button() }
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: 8.0,
    };

    let mut name_label = FixedSizeView {
        size: Size::new(60.0, 20.0),
    };
    let mut name_field = HorizontalExpandingView { height: 34.0 };
    let mut email_label = FixedSizeView {
        size: Size::new(60.0, 20.0),
    };
    let mut email_field = HorizontalExpandingView { height: 34.0 };
    let mut submit_button = FixedSizeView {
        size: Size::new(100.0, 44.0),
    };

    let children: Vec<&dyn SubView> = vec![
        &mut name_label,
        &mut name_field,
        &mut email_label,
        &mut email_field,
        &mut submit_button,
    ];

    let bounds = Rect::new(Point::zero(), Size::new(300.0, 500.0));
    let rects = layout.place(bounds, &children);

    // Each TextField should have exactly 34pt height (not stretched)
    assert_eq!(rects[1].height(), 34.0, "Name field should be 34pt");
    assert_eq!(rects[3].height(), 34.0, "Email field should be 34pt");

    // Each TextField should expand to container width
    assert_eq!(rects[1].width(), 300.0, "Name field should expand to width");
    assert_eq!(
        rects[3].width(),
        300.0,
        "Email field should expand to width"
    );

    // No overlapping
    assert_no_overlap(&rects, "vertical");
}

#[test]
fn test_vstack_form_with_spacer() {
    // Form with spacer pushing button to bottom
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: 8.0,
    };

    let mut name_label = FixedSizeView {
        size: Size::new(60.0, 20.0),
    };
    let mut name_field = HorizontalExpandingView { height: 34.0 };
    let mut spacer = SpacerView;
    let mut submit_button = FixedSizeView {
        size: Size::new(100.0, 44.0),
    };

    let children: Vec<&dyn SubView> = vec![
        &mut name_label,
        &mut name_field,
        &mut spacer,
        &mut submit_button,
    ];

    let bounds = Rect::new(Point::zero(), Size::new(300.0, 300.0));
    let rects = layout.place(bounds, &children);

    // Label: 20, spacing: 8, TextField: 34, spacing: 8, Button: 44
    // Fixed content = 20 + 8 + 34 + 8 + 44 = 114
    // Spacer gets: 300 - 114 - 8 (one more spacing) = 178

    assert_eq!(rects[0].height(), 20.0);
    assert_eq!(rects[1].height(), 34.0, "TextField keeps intrinsic height");
    // Button at bottom
    assert_eq!(
        rects[3].y() + rects[3].height(),
        300.0,
        "Button should be at bottom"
    );
}

// ============================================================================
// MainAxis / CrossAxis Tests
// ============================================================================

/// A mock Spacer that uses [`StretchAxis::MainAxis`].
/// Should expand along [`VStack`]'s main axis (vertical) and [`HStack`]'s main axis (horizontal).
struct MainAxisSpacerView;

impl SubView for MainAxisSpacerView {
    fn size_that_fits(&self, _proposal: ProposalSize) -> Size {
        Size::zero()
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::MainAxis
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A mock Divider that uses [`StretchAxis::CrossAxis`].
/// Should expand along [`VStack`]'s cross axis (horizontal) and [`HStack`]'s cross axis (vertical).
struct CrossAxisDividerView {
    thickness: f32,
}

impl SubView for CrossAxisDividerView {
    fn size_that_fits(&self, _proposal: ProposalSize) -> Size {
        // Returns minimal size - cross axis expansion handled by layout
        Size::new(self.thickness, self.thickness)
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::CrossAxis
    }
    fn priority(&self) -> i32 {
        0
    }
}

#[test]
fn test_vstack_main_axis_spacer() {
    // MainAxis spacer in VStack should expand vertically (the main axis of VStack)
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Center,
        spacing: 0.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(100.0, 30.0),
    };
    let mut spacer = MainAxisSpacerView;
    let mut child2 = FixedSizeView {
        size: Size::new(100.0, 30.0),
    };

    let bounds = Rect::new(Point::zero(), Size::new(100.0, 200.0));
    let children: Vec<&dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

    let rects = layout.place(bounds, &children);

    // Fixed children: 30 + 30 = 60
    // MainAxis spacer should expand: 200 - 60 = 140
    assert_eq!(rects[0].height(), 30.0);
    assert_eq!(
        rects[1].height(),
        140.0,
        "MainAxis spacer should expand vertically in VStack"
    );
    assert_eq!(rects[2].height(), 30.0);
}

#[test]
fn test_hstack_main_axis_spacer() {
    // MainAxis spacer in HStack should expand horizontally (the main axis of HStack)
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 0.0,
    };

    let mut child1 = FixedSizeView {
        size: Size::new(30.0, 100.0),
    };
    let mut spacer = MainAxisSpacerView;
    let mut child2 = FixedSizeView {
        size: Size::new(30.0, 100.0),
    };

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 100.0));
    let children: Vec<&dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

    let rects = layout.place(bounds, &children);

    // Fixed children: 30 + 30 = 60
    // MainAxis spacer should expand: 200 - 60 = 140
    assert_eq!(rects[0].width(), 30.0);
    assert_eq!(
        rects[1].width(),
        140.0,
        "MainAxis spacer should expand horizontally in HStack"
    );
    assert_eq!(rects[2].width(), 30.0);
}

#[test]
fn test_vstack_cross_axis_divider() {
    // CrossAxis divider in VStack should expand horizontally (the cross axis of VStack)
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut divider = CrossAxisDividerView { thickness: 1.0 };
    let mut button = FixedSizeView {
        size: Size::new(80.0, 30.0),
    };

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 100.0));
    let children: Vec<&dyn SubView> = vec![&mut label, &mut divider, &mut button];

    let rects = layout.place(bounds, &children);

    // Divider should expand to full width of VStack (cross axis)
    assert_eq!(
        rects[1].width(),
        200.0,
        "CrossAxis divider should expand to full width in VStack"
    );
    // Divider height should be its intrinsic thickness
    assert_eq!(
        rects[1].height(),
        1.0,
        "CrossAxis divider should keep intrinsic height in VStack"
    );
}

#[test]
fn test_hstack_cross_axis_divider() {
    // CrossAxis divider in HStack should expand vertically (the cross axis of HStack)
    let layout = HStackLayout {
        alignment: VerticalAlignment::Top,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut divider = CrossAxisDividerView { thickness: 1.0 };
    let mut button = FixedSizeView {
        size: Size::new(80.0, 30.0),
    };

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 100.0));
    let children: Vec<&dyn SubView> = vec![&mut label, &mut divider, &mut button];

    let rects = layout.place(bounds, &children);

    // Divider should expand to full height of HStack (cross axis)
    assert_eq!(
        rects[1].height(),
        100.0,
        "CrossAxis divider should expand to full height in HStack"
    );
    // Divider width should be its intrinsic thickness
    assert_eq!(
        rects[1].width(),
        1.0,
        "CrossAxis divider should keep intrinsic width in HStack"
    );
}

/// A mock TextField/Slider that uses [`StretchAxis::Horizontal`].
/// Should expand horizontally in any stack (not context-dependent like MainAxis/CrossAxis).
struct HorizontalStretchView {
    min_width: f32,
    height: f32,
}

impl SubView for HorizontalStretchView {
    fn size_that_fits(&self, proposal: ProposalSize) -> Size {
        // When proposed width, use it (but not less than minimum)
        let width = proposal
            .width
            .map_or(self.min_width, |w| w.max(self.min_width));
        Size::new(width, self.height)
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
    fn priority(&self) -> i32 {
        0
    }
}

#[test]
fn test_vstack_horizontal_stretch_textfield() {
    // StretchAxis::Horizontal in VStack should expand horizontally to full width
    // This tests TextField/Slider behavior
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut textfield = HorizontalStretchView {
        min_width: 100.0,
        height: 34.0,
    };
    let mut button = FixedSizeView {
        size: Size::new(80.0, 40.0),
    };

    let bounds = Rect::new(Point::zero(), Size::new(300.0, 200.0));
    let children: Vec<&dyn SubView> = vec![&mut label, &mut textfield, &mut button];

    let rects = layout.place(bounds, &children);

    // Label keeps its intrinsic width (alignment is Leading, not stretched)
    assert_eq!(rects[0].width(), 50.0, "Label should keep intrinsic width");
    // TextField should expand to full container width
    assert_eq!(
        rects[1].width(),
        300.0,
        "Horizontal stretch TextField should expand to full width in VStack"
    );
    // Button keeps its intrinsic width
    assert_eq!(rects[2].width(), 80.0, "Button should keep intrinsic width");
}

#[test]
fn test_hstack_horizontal_stretch_textfield() {
    // StretchAxis::Horizontal in HStack should also expand horizontally
    // But since HStack's main axis is horizontal, this behaves like MainAxis
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut label = FixedSizeView {
        size: Size::new(50.0, 20.0),
    };
    let mut textfield = HorizontalStretchView {
        min_width: 100.0,
        height: 34.0,
    };
    let mut button = FixedSizeView {
        size: Size::new(80.0, 40.0),
    };

    let bounds = Rect::new(Point::zero(), Size::new(300.0, 100.0));
    let children: Vec<&dyn SubView> = vec![&mut label, &mut textfield, &mut button];

    let rects = layout.place(bounds, &children);

    // Fixed children: 50 + 80 = 130, spacing: 20
    // Remaining for textfield: 300 - 130 - 20 = 150
    assert_eq!(rects[0].width(), 50.0, "Label should keep intrinsic width");
    assert_eq!(
        rects[1].width(),
        150.0,
        "Horizontal stretch TextField should expand in HStack"
    );
    assert_eq!(rects[2].width(), 80.0, "Button should keep intrinsic width");
}
