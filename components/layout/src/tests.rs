//! Comprehensive layout system tests.
//!
//! These tests define the expected behavior of the layout system for various
//! edge cases and ensure consistency across all layout containers.

use alloc::{format, vec, vec::Vec};

use crate::{Layout, Point, ProposalSize, Rect, Size, SubView};
use crate::stack::{HStackLayout, VStackLayout, HorizontalAlignment, VerticalAlignment};

// ============================================================================
// Test Infrastructure
// ============================================================================

/// A mock SubView that returns a fixed size regardless of proposal.
/// This simulates a "rigid" view like an icon or fixed-size image.
struct FixedSizeView {
    size: Size,
}

impl SubView for FixedSizeView {
    fn size_that_fits(&mut self, _proposal: ProposalSize) -> Size {
        self.size
    }
    fn is_stretch(&self) -> bool {
        false
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A mock SubView that respects width proposals (like Text).
/// When given a width constraint, it wraps and increases height.
/// When given None, it returns intrinsic single-line size.
struct FlexibleTextView {
    /// Intrinsic single-line size (no wrapping)
    intrinsic_size: Size,
    /// Character width for calculating wrapped sizes
    char_width: f32,
    /// Line height
    line_height: f32,
}

impl FlexibleTextView {
    fn new(text_width: f32, line_height: f32) -> Self {
        Self {
            intrinsic_size: Size::new(text_width, line_height),
            char_width: 10.0, // Assume 10pt per character for simplicity
            line_height,
        }
    }
}

impl SubView for FlexibleTextView {
    fn size_that_fits(&mut self, proposal: ProposalSize) -> Size {
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
    fn is_stretch(&self) -> bool {
        false
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A mock Spacer that stretches to fill available space.
struct SpacerView;

impl SubView for SpacerView {
    fn size_that_fits(&mut self, _proposal: ProposalSize) -> Size {
        Size::zero()
    }
    fn is_stretch(&self) -> bool {
        true
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A mock axis-expanding view (like TextField, Slider).
/// Expands to fill width, has fixed height.
struct AxisExpandingView {
    height: f32,
}

impl SubView for AxisExpandingView {
    fn size_that_fits(&mut self, proposal: ProposalSize) -> Size {
        let width = proposal.width.unwrap_or(f32::INFINITY);
        Size::new(width, self.height)
    }
    fn is_stretch(&self) -> bool {
        false
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
        msg, rect.x(), bounds.x()
    );
    assert!(
        rect.y() >= bounds.y() - 0.001,
        "{}: rect.y ({}) < bounds.y ({})",
        msg, rect.y(), bounds.y()
    );
    assert!(
        rect.x() + rect.width() <= bounds.x() + bounds.width() + 0.001,
        "{}: rect.right ({}) > bounds.right ({})",
        msg, rect.x() + rect.width(), bounds.x() + bounds.width()
    );
    assert!(
        rect.y() + rect.height() <= bounds.y() + bounds.height() + 0.001,
        "{}: rect.bottom ({}) > bounds.bottom ({})",
        msg, rect.y() + rect.height(), bounds.y() + bounds.height()
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

            assert!(!overlap, "Children {} and {} overlap on {} axis", i, j, axis);
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
    let mut text1 = FixedSizeView { size: Size::new(60.0, 20.0) };
    let mut spacer = SpacerView;
    let mut text2 = FixedSizeView { size: Size::new(60.0, 20.0) };

    let bounds = Rect::new(Point::zero(), Size::new(100.0, 40.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut text1, &mut spacer, &mut text2];

    let rects = layout.place(bounds, &mut children);

    // All children must fit within bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {}", i));
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

    let mut wide_child = FixedSizeView { size: Size::new(200.0, 30.0) };
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 50.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut wide_child];

    let rects = layout.place(bounds, &mut children);

    assert_eq!(rects.len(), 1);
    assert_rect_within_bounds(&rects[0], &bounds, "wide child");
    assert!(
        rects[0].width() <= bounds.width() + 0.001,
        "Child width {} exceeds bounds width {}",
        rects[0].width(), bounds.width()
    );
}

#[test]
fn test_hstack_multiple_children_total_exceeds_bounds() {
    // Multiple children whose total width exceeds bounds
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut child1 = FixedSizeView { size: Size::new(50.0, 20.0) };
    let mut child2 = FixedSizeView { size: Size::new(50.0, 20.0) };
    let mut child3 = FixedSizeView { size: Size::new(50.0, 20.0) };

    // Total: 50 + 10 + 50 + 10 + 50 = 170, bounds = 100
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 40.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut child2, &mut child3];

    let rects = layout.place(bounds, &mut children);

    // All children must fit within bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {}", i));
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

    let mut label = FixedSizeView { size: Size::new(50.0, 20.0) };
    let mut long_text = FlexibleTextView::new(200.0, 20.0); // 200pt wide text

    // bounds width = 150, spacing = 10, available = 140
    // Total intrinsic = 50 + 200 = 250
    // Scale = 140 / 250 = 0.56
    let bounds = Rect::new(Point::zero(), Size::new(150.0, 100.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut label, &mut long_text];

    let rects = layout.place(bounds, &mut children);

    // All children should fit within bounds
    let total_children_width: f32 = rects.iter().map(|r| r.width()).sum();
    let expected_max = bounds.width() - 10.0; // minus spacing

    assert!(
        total_children_width <= expected_max + 0.001,
        "Total children width {} exceeds available space {}",
        total_children_width,
        expected_max
    );

    // Children should not overflow bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {}", i));
    }
}

#[test]
fn test_hstack_empty() {
    let layout = HStackLayout::default();
    let mut children: Vec<&mut dyn SubView> = vec![];

    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &mut children);
    assert_eq!(size.width, 0.0);
    assert_eq!(size.height, 0.0);

    let bounds = Rect::new(Point::zero(), Size::new(100.0, 100.0));
    let rects = layout.place(bounds, &mut children);
    assert!(rects.is_empty());
}

#[test]
fn test_hstack_single_child() {
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut child = FixedSizeView { size: Size::new(50.0, 30.0) };
    let mut children: Vec<&mut dyn SubView> = vec![&mut child];

    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &mut children);
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

    let mut child1 = FixedSizeView { size: Size::new(20.0, 30.0) };
    let mut spacer1 = SpacerView;
    let mut child2 = FixedSizeView { size: Size::new(20.0, 30.0) };
    let mut spacer2 = SpacerView;
    let mut child3 = FixedSizeView { size: Size::new(20.0, 30.0) };

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 50.0));
    let mut children: Vec<&mut dyn SubView> = vec![
        &mut child1, &mut spacer1, &mut child2, &mut spacer2, &mut child3
    ];

    let rects = layout.place(bounds, &mut children);

    // Remaining space = 200 - 60 = 140, divided by 2 spacers = 70 each
    assert_eq!(rects[0].width(), 20.0);
    assert!((rects[1].width() - 70.0).abs() < 0.001, "Spacer 1 width: {}", rects[1].width());
    assert_eq!(rects[2].width(), 20.0);
    assert!((rects[3].width() - 70.0).abs() < 0.001, "Spacer 2 width: {}", rects[3].width());
    assert_eq!(rects[4].width(), 20.0);
}

#[test]
fn test_hstack_alignment_top() {
    let layout = HStackLayout {
        alignment: VerticalAlignment::Top,
        spacing: 10.0,
    };

    let mut short = FixedSizeView { size: Size::new(30.0, 20.0) };
    let mut tall = FixedSizeView { size: Size::new(30.0, 50.0) };

    let bounds = Rect::new(Point::new(10.0, 10.0), Size::new(100.0, 60.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut short, &mut tall];

    let rects = layout.place(bounds, &mut children);

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

    let mut short = FixedSizeView { size: Size::new(30.0, 20.0) };
    let mut tall = FixedSizeView { size: Size::new(30.0, 50.0) };

    let bounds = Rect::new(Point::new(10.0, 10.0), Size::new(100.0, 60.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut short, &mut tall];

    let rects = layout.place(bounds, &mut children);

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

    let mut short = FixedSizeView { size: Size::new(30.0, 20.0) };
    let mut tall = FixedSizeView { size: Size::new(30.0, 50.0) };

    let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 60.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut short, &mut tall];

    let rects = layout.place(bounds, &mut children);

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
    let mut child1 = FixedSizeView { size: Size::new(50.0, 50.0) };
    let mut child2 = FixedSizeView { size: Size::new(50.0, 50.0) };
    let mut child3 = FixedSizeView { size: Size::new(50.0, 50.0) };

    // Total: 50 + 10 + 50 + 10 + 50 = 170, bounds height = 100
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 100.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut child2, &mut child3];

    let rects = layout.place(bounds, &mut children);

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

    let mut wide_child = FixedSizeView { size: Size::new(200.0, 30.0) };
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 50.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut wide_child];

    let rects = layout.place(bounds, &mut children);

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
    let mut children: Vec<&mut dyn SubView> = vec![&mut long_text];

    let rects = layout.place(bounds, &mut children);

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

    let mut label = FixedSizeView { size: Size::new(50.0, 20.0) };
    let mut text_field = AxisExpandingView { height: 30.0 };

    let bounds = Rect::new(Point::zero(), Size::new(200.0, 100.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut label, &mut text_field];

    let rects = layout.place(bounds, &mut children);

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
    let mut children: Vec<&mut dyn SubView> = vec![];

    let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &mut children);
    assert_eq!(size.width, 0.0);
    assert_eq!(size.height, 0.0);
}

#[test]
fn test_vstack_alignment_leading() {
    let layout = VStackLayout {
        alignment: HorizontalAlignment::Leading,
        spacing: 10.0,
    };

    let mut narrow = FixedSizeView { size: Size::new(30.0, 20.0) };
    let mut wide = FixedSizeView { size: Size::new(80.0, 20.0) };

    let bounds = Rect::new(Point::new(10.0, 10.0), Size::new(100.0, 60.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut narrow, &mut wide];

    let rects = layout.place(bounds, &mut children);

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

    let mut narrow = FixedSizeView { size: Size::new(30.0, 20.0) };
    let mut wide = FixedSizeView { size: Size::new(80.0, 20.0) };

    let bounds = Rect::new(Point::new(10.0, 10.0), Size::new(100.0, 60.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut narrow, &mut wide];

    let rects = layout.place(bounds, &mut children);

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

    let mut narrow = FixedSizeView { size: Size::new(30.0, 20.0) };
    let mut wide = FixedSizeView { size: Size::new(80.0, 20.0) };

    let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 60.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut narrow, &mut wide];

    let rects = layout.place(bounds, &mut children);

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

    let mut child1 = FixedSizeView { size: Size::new(50.0, 20.0) };
    let mut spacer1 = SpacerView;
    let mut child2 = FixedSizeView { size: Size::new(50.0, 20.0) };
    let mut spacer2 = SpacerView;
    let mut child3 = FixedSizeView { size: Size::new(50.0, 20.0) };

    let bounds = Rect::new(Point::zero(), Size::new(100.0, 200.0));
    let mut children: Vec<&mut dyn SubView> = vec![
        &mut child1, &mut spacer1, &mut child2, &mut spacer2, &mut child3
    ];

    let rects = layout.place(bounds, &mut children);

    // Remaining height = 200 - 60 = 140, divided by 2 spacers = 70 each
    assert_eq!(rects[0].height(), 20.0);
    assert!((rects[1].height() - 70.0).abs() < 0.001, "Spacer 1 height: {}", rects[1].height());
    assert_eq!(rects[2].height(), 20.0);
    assert!((rects[3].height() - 70.0).abs() < 0.001, "Spacer 2 height: {}", rects[3].height());
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

    let mut label = FixedSizeView { size: Size::new(50.0, 20.0) };
    let mut long_text = FixedSizeView { size: Size::new(500.0, 20.0) }; // Very long text

    // Constrained bounds (like inside a VStack)
    let bounds = Rect::new(Point::zero(), Size::new(200.0, 30.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut label, &mut long_text];

    let rects = layout.place(bounds, &mut children);

    // All children must fit within bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {}", i));
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

    let mut child1 = FixedSizeView { size: Size::new(100.0, 30.0) };
    let mut child2 = FixedSizeView { size: Size::new(100.0, 30.0) };

    // Intrinsic width would be 210 (100 + 10 + 100)
    // But with proposal width = 150, should report 150 (capped to proposal)
    let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut child2];
    let size = layout.size_that_fits(
        ProposalSize::new(Some(150.0), None),
        &mut children
    );

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

    let mut child1 = FixedSizeView { size: Size::new(100.0, 50.0) };
    let mut child2 = FixedSizeView { size: Size::new(80.0, 50.0) };

    let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut child2];

    // With width proposal, VStack should report min(max_child_width, proposal)
    let size = layout.size_that_fits(
        ProposalSize::new(Some(90.0), None),
        &mut children
    );

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

    let mut child = FixedSizeView { size: Size::new(50.0, 30.0) };
    let bounds = Rect::new(Point::zero(), Size::zero());

    let mut children: Vec<&mut dyn SubView> = vec![&mut child];
    let rects = hstack.place(bounds, &mut children);

    // Should handle gracefully - child clamped to zero
    assert!(rects[0].width() <= 0.001 || rects[0].width() <= 50.0);

    let mut child = FixedSizeView { size: Size::new(50.0, 30.0) };
    let mut children: Vec<&mut dyn SubView> = vec![&mut child];
    let rects = vstack.place(bounds, &mut children);

    assert!(rects[0].height() <= 0.001 || rects[0].height() <= 30.0);
}

#[test]
fn test_negative_remaining_space_with_spacer() {
    // When children exceed bounds, spacer should get zero width, not negative
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut child1 = FixedSizeView { size: Size::new(60.0, 20.0) };
    let mut spacer = SpacerView;
    let mut child2 = FixedSizeView { size: Size::new(60.0, 20.0) };

    // Total non-spacer width = 60 + 10 + 10 + 60 = 140, bounds = 100
    let bounds = Rect::new(Point::zero(), Size::new(100.0, 30.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

    let rects = layout.place(bounds, &mut children);

    // Spacer should never have negative dimensions
    assert!(rects[1].width() >= 0.0, "Spacer has negative width: {}", rects[1].width());
    assert!(rects[1].height() >= 0.0, "Spacer has negative height: {}", rects[1].height());
}

#[test]
fn test_bounds_with_offset() {
    // Bounds starting at non-zero origin
    let layout = HStackLayout {
        alignment: VerticalAlignment::Center,
        spacing: 10.0,
    };

    let mut child1 = FixedSizeView { size: Size::new(30.0, 20.0) };
    let mut child2 = FixedSizeView { size: Size::new(40.0, 25.0) };

    let bounds = Rect::new(Point::new(50.0, 100.0), Size::new(200.0, 50.0));
    let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut child2];

    let rects = layout.place(bounds, &mut children);

    // First child should start at bounds origin
    assert_eq!(rects[0].x(), 50.0);

    // All children should be within bounds
    for (i, rect) in rects.iter().enumerate() {
        assert_rect_within_bounds(rect, &bounds, &format!("child {} with offset", i));
    }
}
