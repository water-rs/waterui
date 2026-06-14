//! Horizontal stack layout.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{
    AnyView, IntoSignalF32, View, env::with, id::Identifiable, view::TupleViews, views::ForEach,
};

use crate::{
    Layout, LazyContainer, PlacedSubview, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    ViewDimensions,
    container::FixedContainer,
    measure_children,
    stack::{Axis, VerticalAlignment},
};

/// A view that arranges its children in a horizontal line.
///
/// Use an `HStack` to arrange views side-by-side. The stack sizes itself to fit
/// its contents, distributing available space among its children.
///
/// ```ignore
/// hstack((
///     text("Hello"),
///     text("World"),
/// ))
/// ```
///
/// You can customize the spacing between children and their vertical alignment:
///
/// ```ignore
/// HStack::new(VerticalAlignment::Top, 20.0, (
///     text("First"),
///     text("Second"),
/// ))
/// ```
///
/// Use [`spacer()`](crate::spacer()) to push content to the sides:
///
/// ```ignore
/// hstack((
///     text("Leading"),
///     spacer(),
///     text("Trailing"),
/// ))
/// ```
#[derive(Debug, Clone)]
pub struct HStack<C> {
    layout: HStackLayout,
    contents: C,
}

/// Layout engine shared by the public [`HStack`] view.
#[derive(Debug, Clone)]
pub struct HStackLayout {
    /// The vertical alignment of children within the stack.
    pub alignment: VerticalAlignment,
    /// The spacing between children in the stack.
    pub spacing: f32,
}

impl Default for HStackLayout {
    fn default() -> Self {
        Self {
            alignment: VerticalAlignment::Center,
            spacing: 10.0,
        }
    }
}

/// Cached measurement for a child during layout
struct ChildMeasurement {
    dimensions: ViewDimensions,
    stretch_axis: StretchAxis,
}

impl ChildMeasurement {
    const fn size(&self) -> Size {
        self.dimensions.size
    }

    fn vertical_guide(&self, alignment: VerticalAlignment) -> f32 {
        self.dimensions.vertical(alignment)
    }

    /// Returns true if this child stretches horizontally (for `HStack` width distribution).
    /// In `HStack` context:
    /// - `MainAxis` means horizontal (`HStack`'s main axis)
    /// - `CrossAxis` means vertical (`HStack`'s cross axis)
    const fn stretches_main_axis(&self) -> bool {
        matches!(
            self.stretch_axis,
            StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::MainAxis
        )
    }

    /// Returns true if this child stretches vertically (for `HStack` height expansion).
    /// In `HStack` context:
    /// - `CrossAxis` means vertical (`HStack`'s cross axis)
    const fn stretches_cross_axis(&self) -> bool {
        matches!(
            self.stretch_axis,
            StretchAxis::Vertical | StretchAxis::Both | StretchAxis::CrossAxis
        )
    }
}

fn hstack_intrinsic_cross_metrics(
    measurements: &[ChildMeasurement],
    alignment: VerticalAlignment,
) -> (f32, f32) {
    let mut max_above = 0.0_f32;
    let mut max_below = 0.0_f32;

    for measurement in measurements.iter().filter(|m| !m.stretches_cross_axis()) {
        let size = measurement.size();
        let guide = measurement
            .vertical_guide(alignment)
            .clamp(0.0, size.height);
        max_above = max_above.max(guide);
        max_below = max_below.max((size.height - guide).max(0.0));
    }

    (max_above, max_below)
}

fn hstack_container_guide_offset(
    bounds: Rect,
    alignment: VerticalAlignment,
    intrinsic_above: f32,
) -> f32 {
    if alignment == VerticalAlignment::Top {
        0.0
    } else if alignment == VerticalAlignment::Bottom {
        bounds.height()
    } else if alignment == VerticalAlignment::Center {
        bounds.height() * 0.5
    } else {
        intrinsic_above
    }
}

fn hstack_stretch_allocations(
    stretch_indices: &[usize],
    available_for_children: f32,
    fixed_width: f32,
) -> Vec<(usize, f32)> {
    if stretch_indices.is_empty() {
        return Vec::new();
    }

    let width = (available_for_children - fixed_width).max(0.0) / stretch_indices.len() as f32;

    stretch_indices.iter().map(|&idx| (idx, width)).collect()
}

#[allow(clippy::cast_precision_loss)]
#[allow(clippy::too_many_lines)]
impl Layout for HStackLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
        } else {
            0.0
        };

        // First pass: measure children. If min size query (width=0), use 0 width proposal to get min sizes.
        let is_min_size_query = proposal.width == Some(0.0);
        let intrinsic_proposal = if is_min_size_query {
            ProposalSize::new(Some(0.0), proposal.height)
        } else {
            ProposalSize::new(None, proposal.height)
        };
        let mut measurements: Vec<ChildMeasurement> =
            measure_children(children, |child| ChildMeasurement {
                dimensions: child.measure(intrinsic_proposal),
                stretch_axis: child.stretch_axis(),
            });

        // HStack checks for main-axis (horizontal) stretching
        let has_main_axis_stretch = measurements
            .iter()
            .any(ChildMeasurement::stretches_main_axis);
        let main_axis_stretch_indices: Vec<usize> = measurements
            .iter()
            .enumerate()
            .filter(|(_, m)| m.stretches_main_axis())
            .map(|(idx, _)| idx)
            .collect();
        let main_axis_stretch_count = main_axis_stretch_indices.len();

        let intrinsic_width_all: f32 =
            measurements.iter().map(|m| m.size().width).sum::<f32>() + total_spacing;

        if is_min_size_query {
            let (max_above, max_below) =
                hstack_intrinsic_cross_metrics(&measurements, self.alignment);
            return Size::new(intrinsic_width_all, max_above + max_below);
        }

        // Intrinsic width used when the parent doesn't constrain width.
        // In unconstrained context, even "stretching" children should be measured at their
        // intrinsic widths (otherwise content-bearing views could collapse to 0 width).
        let intrinsic_width = intrinsic_width_all;

        // Determine final width
        let final_width = proposal.width.map_or(intrinsic_width, |proposed| {
            if has_main_axis_stretch {
                proposed
            } else {
                intrinsic_width.min(proposed)
            }
        });

        // If width is constrained, we need to distribute space properly
        // Key insight: small children (labels) keep intrinsic width, large children (text) compress
        let available_for_children = (final_width - total_spacing).max(0.0);

        let fixed_indices: Vec<usize> = if main_axis_stretch_count > 0 && proposal.width.is_some() {
            measurements
                .iter()
                .enumerate()
                .filter(|(_, m)| !m.stretches_main_axis())
                .map(|(idx, _)| idx)
                .collect()
        } else {
            (0..measurements.len()).collect()
        };

        let fixed_width: f32 = fixed_indices
            .iter()
            .map(|&idx| measurements[idx].size().width)
            .sum();

        if proposal.width.is_some()
            && !fixed_indices.is_empty()
            && fixed_width > available_for_children
        {
            // Need to compress - find the largest child and give it remaining space
            // Small children keep their intrinsic width
            let overflow = fixed_width - available_for_children;

            // Find indices of non-main-axis-stretching children sorted by width (largest first)
            let mut compress_indices = fixed_indices;
            compress_indices.sort_by(|&a, &b| {
                measurements[b]
                    .size()
                    .width
                    .total_cmp(&measurements[a].size().width)
            });

            // Compress largest children first until we fit
            let mut remaining_overflow = overflow;
            for &idx in &compress_indices {
                if remaining_overflow <= 0.0 {
                    break;
                }

                let current_width = measurements[idx].size().width;
                // Don't compress below a minimum (e.g., 20px for very small labels)
                let min_width = 20.0_f32.min(current_width);
                let max_reduction = current_width - min_width;
                let reduction = remaining_overflow.min(max_reduction);

                if reduction > 0.0 {
                    let new_width = current_width - reduction;
                    let constrained_proposal = ProposalSize::new(Some(new_width), proposal.height);
                    measurements[idx].dimensions = children[idx].measure(constrained_proposal);
                    remaining_overflow -= reduction;
                }
            }
        }

        // If there are main-axis stretching children and width is constrained, measure them with
        // their allocated widths so height reflects wrapped content.
        if proposal.width.is_some() && main_axis_stretch_count > 0 {
            let fixed_width: f32 = measurements
                .iter()
                .enumerate()
                .filter(|(_, m)| !m.stretches_main_axis())
                .map(|(_, m)| m.size().width)
                .sum();

            let allocations = hstack_stretch_allocations(
                &main_axis_stretch_indices,
                available_for_children,
                fixed_width,
            );

            for (idx, stretch_width) in allocations {
                let constrained_proposal = ProposalSize::new(Some(stretch_width), proposal.height);
                measurements[idx].dimensions = children[idx].measure(constrained_proposal);
                measurements[idx].dimensions.size.width =
                    measurements[idx].size().width.max(stretch_width);
            }
        }

        // Height: max of all children (after re-measurement for proper wrapped height)
        // Important: Do NOT cap height to proposal - if text wraps, we need the full height
        // Note: cross-axis-stretching children don't contribute to intrinsic height
        let (max_above, max_below) = hstack_intrinsic_cross_metrics(&measurements, self.alignment);
        let max_height = max_above + max_below;

        Size::new(final_width, max_height)
    }

    #[allow(clippy::too_many_lines)]
    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
        } else {
            0.0
        };

        let available_width = bounds.width() - total_spacing;

        // First pass: measure all children with None to get intrinsic sizes
        let intrinsic_proposal = ProposalSize::new(None, Some(bounds.height()));
        let mut measurements: Vec<ChildMeasurement> =
            measure_children(children, |child| ChildMeasurement {
                dimensions: child.measure(intrinsic_proposal),
                stretch_axis: child.stretch_axis(),
            });

        // Calculate totals - HStack cares about main-axis (horizontal) stretching
        let main_axis_stretch_indices: Vec<usize> = measurements
            .iter()
            .enumerate()
            .filter(|(_, m)| m.stretches_main_axis())
            .map(|(idx, _)| idx)
            .collect();
        let main_axis_stretch_count = main_axis_stretch_indices.len();

        let fixed_indices: Vec<usize> = if main_axis_stretch_count > 0 {
            measurements
                .iter()
                .enumerate()
                .filter(|(_, m)| !m.stretches_main_axis())
                .map(|(idx, _)| idx)
                .collect()
        } else {
            (0..measurements.len()).collect()
        };

        let fixed_width: f32 = fixed_indices
            .iter()
            .map(|&idx| measurements[idx].size().width)
            .sum();

        // Check if we need to compress children (when fixed children don't fit)
        let needs_compression = !fixed_indices.is_empty() && fixed_width > available_width;

        if needs_compression {
            // Compress largest children first, keeping small labels at intrinsic width
            let overflow = fixed_width - available_width;

            // Find indices of non-main-axis-stretching children sorted by width (largest first)
            let mut compress_indices = fixed_indices;
            compress_indices.sort_by(|&a, &b| {
                measurements[b]
                    .size()
                    .width
                    .total_cmp(&measurements[a].size().width)
            });

            // Compress largest children first until we fit
            let mut remaining_overflow = overflow;
            for &idx in &compress_indices {
                if remaining_overflow <= 0.0 {
                    break;
                }

                let current_width = measurements[idx].size().width;
                // Don't compress below a minimum (keep small labels readable)
                let min_width = 20.0_f32.min(current_width);
                let max_reduction = current_width - min_width;
                let reduction = remaining_overflow.min(max_reduction);

                if reduction > 0.0 {
                    let new_width = current_width - reduction;
                    let constrained_proposal =
                        ProposalSize::new(Some(new_width), Some(bounds.height()));
                    measurements[idx].dimensions = children[idx].measure(constrained_proposal);
                    measurements[idx].dimensions.size.width =
                        measurements[idx].size().width.min(new_width);
                    remaining_overflow -= reduction;
                }
            }
        }

        // Calculate stretch child width from remaining space
        let actual_fixed_width: f32 = measurements
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.stretches_main_axis())
            .map(|(_, m)| m.size().width)
            .sum();

        let stretch_allocations = hstack_stretch_allocations(
            &main_axis_stretch_indices,
            available_width,
            actual_fixed_width,
        );

        // Measure stretching children with their allocated width so cross-axis sizing is accurate.
        if main_axis_stretch_count > 0 {
            for &(idx, stretch_width) in &stretch_allocations {
                let constrained_proposal =
                    ProposalSize::new(Some(stretch_width), Some(bounds.height()));
                measurements[idx].dimensions = children[idx].measure(constrained_proposal);
                measurements[idx].dimensions.size.width =
                    measurements[idx].size().width.max(stretch_width);
            }
        }

        let (intrinsic_above, _intrinsic_below) =
            hstack_intrinsic_cross_metrics(&measurements, self.alignment);
        let guide_line =
            bounds.y() + hstack_container_guide_offset(bounds, self.alignment, intrinsic_above);

        // Place children
        let mut rects = Vec::with_capacity(children.len());
        let mut current_x = bounds.x();

        for (i, measurement) in measurements.iter().enumerate() {
            if i > 0 {
                current_x += self.spacing;
            }

            // Handle cross-axis (vertical) stretching and infinite height
            let child_height = if measurement.stretches_cross_axis() {
                // CrossAxis in HStack means expand vertically to full bounds height
                bounds.height()
            } else if measurement.size().height.is_infinite() {
                bounds.height()
            } else {
                measurement.size().height.min(bounds.height())
            };

            let child_width = measurement.size().width;

            let mut adjusted_dimensions = measurement.dimensions.clone();
            adjusted_dimensions.size = Size::new(child_width, child_height);

            let y =
                if measurement.stretches_cross_axis() || self.alignment == VerticalAlignment::Top {
                    bounds.y()
                } else if self.alignment == VerticalAlignment::Bottom {
                    bounds.y() + bounds.height() - child_height
                } else {
                    let guide = adjusted_dimensions
                        .vertical(self.alignment)
                        .clamp(0.0, child_height);
                    guide_line - guide
                };

            let rect = Rect::new(
                Point::new(current_x, y),
                Size::new(child_width, child_height),
            );
            rects.push(rect);

            current_x += child_width;
        }

        rects
    }

    fn explicit_vertical(
        &self,
        alignment: VerticalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if alignment == VerticalAlignment::LastBaseline {
            return children
                .iter()
                .filter_map(|child| child.explicit_vertical(alignment))
                .max_by(f32::total_cmp);
        }

        if alignment == VerticalAlignment::FirstBaseline || alignment == self.alignment {
            return children
                .iter()
                .filter_map(|child| child.explicit_vertical(alignment))
                .min_by(f32::total_cmp);
        }

        None
    }
}

impl<C> HStack<(C,)> {
    /// Creates a horizontal stack with the provided alignment, spacing, and
    /// children.
    pub const fn new(alignment: VerticalAlignment, spacing: f32, contents: C) -> Self {
        Self {
            layout: HStackLayout { alignment, spacing },
            contents: (contents,),
        }
    }
}

impl<C> HStack<C> {
    /// Sets the vertical alignment for children in the stack.
    #[must_use]
    pub const fn alignment(mut self, alignment: VerticalAlignment) -> Self {
        self.layout.alignment = alignment;
        self
    }

    /// Sets the spacing between children in the stack.
    ///
    /// Accepts any numeric literal or signal of f32; snapshotted at modifier
    /// time per `WaterUI`'s Vue-like reactivity model.
    #[must_use]
    pub fn spacing(mut self, spacing: impl IntoSignalF32 + 'static) -> Self {
        use waterui_core::Signal;
        self.layout.spacing = spacing.into_signal_f32().get();
        self
    }
}

crate::stack::impl_stack_for_each!(HStack, HStackLayout);

impl<V> FromIterator<V> for HStack<(Vec<AnyView>,)>
where
    V: View,
{
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let contents = iter.into_iter().map(AnyView::new).collect();
        Self::new(VerticalAlignment::default(), 10.0, contents)
    }
}

/// Convenience constructor that centres children and uses the default spacing.
pub const fn hstack<C>(contents: C) -> HStack<(C,)> {
    HStack::new(VerticalAlignment::Center, 10.0, contents)
}

impl<C, F, V> View for HStack<ForEach<C, F, V>>
where
    C: Collection + Clone,
    C::Item: Identifiable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // Inject the horizontal axis into the container
        with(
            LazyContainer::new(self.layout, self.contents),
            Axis::Horizontal,
        )
    }
}

impl<C: TupleViews + 'static> View for HStack<(C,)> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // Inject the horizontal axis into the container
        with(
            FixedContainer::new(self.layout, self.contents.0),
            Axis::Horizontal,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSubView {
        size: Size,
        stretch_axis: StretchAxis,
    }

    impl SubView for MockSubView {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.size)
        }
        fn stretch_axis(&self) -> StretchAxis {
            self.stretch_axis
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    struct ResponsiveSubView {
        intrinsic: Size,
        wrapped_height: f32,
        wrap_at_or_below: f32,
        stretch_axis: StretchAxis,
    }

    impl SubView for ResponsiveSubView {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(match proposal.width {
                Some(width) if width <= self.wrap_at_or_below => {
                    Size::new(width, self.wrapped_height)
                }
                Some(width) => Size::new(width, self.intrinsic.height),
                None => self.intrinsic,
            })
        }

        fn stretch_axis(&self) -> StretchAxis {
            self.stretch_axis
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn test_hstack_size_two_children() {
        let layout = HStackLayout {
            alignment: VerticalAlignment::Center,
            spacing: 10.0,
        };

        let mut child1 = MockSubView {
            size: Size::new(50.0, 30.0),
            stretch_axis: StretchAxis::None,
        };
        let mut child2 = MockSubView {
            size: Size::new(60.0, 40.0),
            stretch_axis: StretchAxis::None,
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        assert!((size.width - 120.0).abs() < f32::EPSILON); // 50 + 10 + 60
        assert!((size.height - 40.0).abs() < f32::EPSILON); // max height
    }

    #[test]
    fn test_hstack_with_spacer() {
        let layout = HStackLayout {
            alignment: VerticalAlignment::Center,
            spacing: 0.0,
        };

        let mut child1 = MockSubView {
            size: Size::new(30.0, 40.0),
            stretch_axis: StretchAxis::None,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            stretch_axis: StretchAxis::Both, // Spacer stretches in both directions
        };
        let mut child2 = MockSubView {
            size: Size::new(30.0, 40.0),
            stretch_axis: StretchAxis::None,
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

        // With specified width, spacer should expand
        let size = layout.size_that_fits(ProposalSize::new(Some(200.0), None), &children);

        assert!((size.width - 200.0).abs() < f32::EPSILON);

        // Place should distribute remaining space to spacer
        let bounds = Rect::new(Point::zero(), Size::new(200.0, 40.0));

        let mut child1 = MockSubView {
            size: Size::new(30.0, 40.0),
            stretch_axis: StretchAxis::None,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            stretch_axis: StretchAxis::Both,
        };
        let mut child2 = MockSubView {
            size: Size::new(30.0, 40.0),
            stretch_axis: StretchAxis::None,
        };
        let children: Vec<&dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

        let rects = layout.place(bounds, &children);

        assert!((rects[0].width() - 30.0).abs() < f32::EPSILON);
        assert!((rects[1].width() - 140.0).abs() < f32::EPSILON); // 200 - 30 - 30
        assert!((rects[2].width() - 30.0).abs() < f32::EPSILON);
        assert!((rects[2].x() - 170.0).abs() < f32::EPSILON); // 30 + 140
    }

    #[test]
    fn test_hstack_with_vertical_stretch() {
        // Vertical stretch component in HStack: stretches HEIGHT but has fixed WIDTH
        // This is like a Slider/TextField rotated for use in HStack context
        let layout = HStackLayout {
            alignment: VerticalAlignment::Center,
            spacing: 10.0,
        };

        let mut label = MockSubView {
            size: Size::new(50.0, 20.0),
            stretch_axis: StretchAxis::None,
        };
        // A vertically-stretching component: fixed width, wants to fill height
        let mut vertical_stretch = MockSubView {
            size: Size::new(40.0, 100.0), // reports minimum width, tall height
            stretch_axis: StretchAxis::Vertical, // stretches height only
        };
        let mut button = MockSubView {
            size: Size::new(80.0, 44.0),
            stretch_axis: StretchAxis::None,
        };

        let children: Vec<&dyn SubView> = vec![&mut label, &mut vertical_stretch, &mut button];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        // Width: all children contribute (vertical_stretch doesn't stretch horizontally)
        // = 50 + 10 + 40 + 10 + 80 = 190
        assert!((size.width - 190.0).abs() < f32::EPSILON);
        // Height: max of non-vertically-stretching children = max(20, 44) = 44
        // Note: vertical_stretch stretches vertically so its height doesn't contribute
        assert!((size.height - 44.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_hstack_measures_stretch_child_with_allocated_width_for_height() {
        let layout = HStackLayout {
            alignment: VerticalAlignment::Center,
            spacing: 0.0,
        };

        let mut fixed = MockSubView {
            size: Size::new(4.0, 10.0),
            stretch_axis: StretchAxis::None,
        };

        // Simulate a content-bearing stretch child whose height increases when width is constrained.
        let mut stretch = ResponsiveSubView {
            intrinsic: Size::new(100.0, 20.0),
            wrapped_height: 40.0,
            wrap_at_or_below: 60.0,
            stretch_axis: StretchAxis::Horizontal,
        };

        let children: Vec<&dyn SubView> = vec![&mut fixed, &mut stretch];

        let size = layout.size_that_fits(ProposalSize::new(Some(40.0), None), &children);

        assert!((size.width - 40.0).abs() < f32::EPSILON);
        assert!((size.height - 40.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_hstack_place_uses_stretch_child_wrapped_height() {
        let layout = HStackLayout {
            alignment: VerticalAlignment::Center,
            spacing: 0.0,
        };

        let bounds = Rect::new(Point::zero(), Size::new(40.0, 40.0));

        let mut fixed = MockSubView {
            size: Size::new(4.0, 10.0),
            stretch_axis: StretchAxis::None,
        };

        let mut stretch = ResponsiveSubView {
            intrinsic: Size::new(100.0, 20.0),
            wrapped_height: 40.0,
            wrap_at_or_below: 60.0,
            stretch_axis: StretchAxis::Horizontal,
        };

        let children: Vec<&dyn SubView> = vec![&mut fixed, &mut stretch];

        let rects = layout.place(bounds, &children);

        assert!((rects[0].width() - 4.0).abs() < f32::EPSILON);
        assert!((rects[1].width() - 36.0).abs() < f32::EPSILON);
        assert!((rects[1].height() - 40.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_hstack_min_size_query_correctly_sums_children() {
        // When ProposalSize::ZERO is used (min size query), ALL children's widths
        // should be summed to ensure window minimum width calculation is correct.
        // Previous bug: if spacer (stretch) existed, it would return 0 width for min size query.
        let layout = HStackLayout {
            alignment: VerticalAlignment::Center,
            spacing: 10.0,
        };

        let mut label = MockSubView {
            size: Size::new(50.0, 20.0),
            stretch_axis: StretchAxis::None,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            stretch_axis: StretchAxis::Both,
        };
        let mut button = MockSubView {
            size: Size::new(80.0, 44.0),
            stretch_axis: StretchAxis::None,
        };

        let children: Vec<&dyn SubView> = vec![&mut label, &mut spacer, &mut button];

        // With ZERO proposal (min size query)
        let min_size = layout.size_that_fits(ProposalSize::ZERO, &children);

        // Width: 50 + 10 + 0 + 10 + 80 = 150
        assert!(
            (min_size.width - 150.0).abs() < f32::EPSILON,
            "Min size query should return sum of child min widths (150.0), got {}",
            min_size.width
        );

        // Verify regular behavior (Unspecified) still works
        let intrinsic_size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);
        // Should also be 150 because spacer has 0 intrinsic width
        assert!((intrinsic_size.width - 150.0).abs() < f32::EPSILON);
    }
}
