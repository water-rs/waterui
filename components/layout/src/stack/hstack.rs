//! Horizontal stack layout.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{AnyView, View, id::Identifable, view::TupleViews, views::ForEach};

use crate::{
    Container, Layout, Point, ProposalSize, Rect, Size,
    container::FixedContainer, stack::VerticalAlignment, SubView,
};

/// A horizontal stack that arranges its children in a horizontal line.
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
    size: Size,
    is_stretch: bool,
}

#[allow(clippy::cast_precision_loss)]
impl Layout for HStackLayout {
    fn size_that_fits(
        &self,
        proposal: ProposalSize,
        children: &mut [&mut dyn SubView],
    ) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
        } else {
            0.0
        };

        // First pass: measure children with unspecified width to get intrinsic sizes
        let intrinsic_proposal = ProposalSize::new(None, proposal.height);
        let mut measurements: Vec<ChildMeasurement> = children
            .iter_mut()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(intrinsic_proposal),
                is_stretch: child.is_stretch(),
            })
            .collect();

        let has_stretch = measurements.iter().any(|m| m.is_stretch);
        let stretch_count = measurements.iter().filter(|m| m.is_stretch).count();

        // Calculate intrinsic width (sum of non-stretch children + spacing)
        let non_stretch_width: f32 = measurements
            .iter()
            .filter(|m| !m.is_stretch)
            .map(|m| m.size.width)
            .sum();

        let intrinsic_width = non_stretch_width + total_spacing;

        // Determine final width
        let final_width = if has_stretch {
            proposal.width.unwrap_or(intrinsic_width)
        } else {
            match proposal.width {
                Some(proposed) => intrinsic_width.min(proposed),
                None => intrinsic_width,
            }
        };

        // If width is constrained, we need to distribute space properly
        // Key insight: small children (labels) keep intrinsic width, large children (text) compress
        let available_for_children = final_width - total_spacing;

        if proposal.width.is_some() && non_stretch_width > available_for_children {
            // Need to compress - find the largest child and give it remaining space
            // Small children keep their intrinsic width
            let overflow = non_stretch_width - available_for_children;

            // Find indices of non-stretch children sorted by width (largest first)
            let mut non_stretch_indices: Vec<usize> = measurements
                .iter()
                .enumerate()
                .filter(|(_, m)| !m.is_stretch)
                .map(|(i, _)| i)
                .collect();
            non_stretch_indices.sort_by(|&a, &b| {
                measurements[b].size.width.partial_cmp(&measurements[a].size.width).unwrap()
            });

            // Compress largest children first until we fit
            let mut remaining_overflow = overflow;
            for &idx in &non_stretch_indices {
                if remaining_overflow <= 0.0 {
                    break;
                }

                let current_width = measurements[idx].size.width;
                // Don't compress below a minimum (e.g., 20px for very small labels)
                let min_width = 20.0_f32.min(current_width);
                let max_reduction = current_width - min_width;
                let reduction = remaining_overflow.min(max_reduction);

                if reduction > 0.0 {
                    let new_width = current_width - reduction;
                    let constrained_proposal = ProposalSize::new(
                        Some(new_width),
                        proposal.height,
                    );
                    measurements[idx].size = children[idx].size_that_fits(constrained_proposal);
                    remaining_overflow -= reduction;
                }
            }
        } else if proposal.width.is_some() && stretch_count > 0 {
            // With spacers, non-stretch children keep intrinsic width
            // Spacers get the remaining space (but we don't measure them here)
        }

        // Height: max of all children (after re-measurement for proper wrapped height)
        // Important: Do NOT cap height to proposal - if text wraps, we need the full height
        let max_height = measurements
            .iter()
            .filter(|m| !m.is_stretch)
            .map(|m| m.size.height)
            .max_by(f32::total_cmp)
            .unwrap_or(0.0);

        Size::new(final_width, max_height)
    }

    fn place(
        &self,
        bounds: Rect,
        children: &mut [&mut dyn SubView],
    ) -> Vec<Rect> {
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
        let mut measurements: Vec<ChildMeasurement> = children
            .iter_mut()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(intrinsic_proposal),
                is_stretch: child.is_stretch(),
            })
            .collect();

        // Calculate totals
        let stretch_count = measurements.iter().filter(|m| m.is_stretch).count();
        let non_stretch_count = measurements.iter().filter(|m| !m.is_stretch).count();

        let total_intrinsic_width: f32 = measurements
            .iter()
            .filter(|m| !m.is_stretch)
            .map(|m| m.size.width)
            .sum();

        // Calculate how much space is available for non-stretch children
        let width_for_non_stretch = if stretch_count > 0 {
            // If there are spacers, non-stretch children get their intrinsic width
            // but capped to available space
            available_width.min(total_intrinsic_width)
        } else {
            // No spacers - all width goes to non-stretch children
            available_width
        };

        // Check if we need to compress children
        let needs_compression = total_intrinsic_width > width_for_non_stretch && non_stretch_count > 0;

        if needs_compression {
            // Compress largest children first, keeping small labels at intrinsic width
            let overflow = total_intrinsic_width - width_for_non_stretch;

            // Find indices of non-stretch children sorted by width (largest first)
            let mut non_stretch_indices: Vec<usize> = measurements
                .iter()
                .enumerate()
                .filter(|(_, m)| !m.is_stretch)
                .map(|(i, _)| i)
                .collect();
            non_stretch_indices.sort_by(|&a, &b| {
                measurements[b].size.width.partial_cmp(&measurements[a].size.width).unwrap()
            });

            // Compress largest children first until we fit
            let mut remaining_overflow = overflow;
            for &idx in &non_stretch_indices {
                if remaining_overflow <= 0.0 {
                    break;
                }

                let current_width = measurements[idx].size.width;
                // Don't compress below a minimum (keep small labels readable)
                let min_width = 20.0_f32.min(current_width);
                let max_reduction = current_width - min_width;
                let reduction = remaining_overflow.min(max_reduction);

                if reduction > 0.0 {
                    let new_width = current_width - reduction;
                    let constrained_proposal = ProposalSize::new(
                        Some(new_width),
                        Some(bounds.height()),
                    );
                    measurements[idx].size = children[idx].size_that_fits(constrained_proposal);
                    measurements[idx].size.width = measurements[idx].size.width.min(new_width);
                    remaining_overflow -= reduction;
                }
            }
        }

        // Calculate stretch child width from remaining space
        let actual_non_stretch_width: f32 = measurements
            .iter()
            .filter(|m| !m.is_stretch)
            .map(|m| m.size.width)
            .sum();

        let remaining_width = (available_width - actual_non_stretch_width).max(0.0);
        let stretch_width = if stretch_count > 0 {
            remaining_width / stretch_count as f32
        } else {
            0.0
        };

        // Place children
        let mut rects = Vec::with_capacity(children.len());
        let mut current_x = bounds.x();

        for (i, measurement) in measurements.iter().enumerate() {
            if i > 0 {
                current_x += self.spacing;
            }

            // Handle infinite height (axis-expanding views) and clamp to bounds
            let child_height = if measurement.size.height.is_infinite() {
                bounds.height()
            } else {
                measurement.size.height.min(bounds.height())
            };

            let child_width = if measurement.is_stretch {
                stretch_width
            } else {
                measurement.size.width
            };

            let y = match self.alignment {
                VerticalAlignment::Top => bounds.y(),
                VerticalAlignment::Center => bounds.y() + (bounds.height() - child_height) / 2.0,
                VerticalAlignment::Bottom => bounds.y() + bounds.height() - child_height,
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
    #[must_use]
    pub const fn spacing(mut self, spacing: f32) -> Self {
        self.layout.spacing = spacing;
        self
    }
}

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
    C: Collection,
    C::Item: Identifable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Container::new(self.layout, self.contents)
    }
}

impl<C: TupleViews + 'static> View for HStack<(C,)> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, self.contents.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSubView {
        size: Size,
        is_stretch: bool,
    }

    impl SubView for MockSubView {
        fn size_that_fits(&mut self, _proposal: ProposalSize) -> Size {
            self.size
        }
        fn is_stretch(&self) -> bool {
            self.is_stretch
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
            is_stretch: false,
        };
        let mut child2 = MockSubView {
            size: Size::new(60.0, 40.0),
            is_stretch: false,
        };

        let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut child2];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &mut children);

        assert_eq!(size.width, 120.0); // 50 + 10 + 60
        assert_eq!(size.height, 40.0); // max height
    }

    #[test]
    fn test_hstack_with_spacer() {
        let layout = HStackLayout {
            alignment: VerticalAlignment::Center,
            spacing: 0.0,
        };

        let mut child1 = MockSubView {
            size: Size::new(30.0, 40.0),
            is_stretch: false,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            is_stretch: true,
        };
        let mut child2 = MockSubView {
            size: Size::new(30.0, 40.0),
            is_stretch: false,
        };

        let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

        // With specified width, spacer should expand
        let size = layout.size_that_fits(
            ProposalSize::new(Some(200.0), None),
            &mut children,
        );

        assert_eq!(size.width, 200.0);

        // Place should distribute remaining space to spacer
        let bounds = Rect::new(Point::zero(), Size::new(200.0, 40.0));

        let mut child1 = MockSubView {
            size: Size::new(30.0, 40.0),
            is_stretch: false,
        };
        let mut spacer = MockSubView {
            size: Size::zero(),
            is_stretch: true,
        };
        let mut child2 = MockSubView {
            size: Size::new(30.0, 40.0),
            is_stretch: false,
        };
        let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut spacer, &mut child2];

        let rects = layout.place(bounds, &mut children);

        assert_eq!(rects[0].width(), 30.0);
        assert_eq!(rects[1].width(), 140.0); // 200 - 30 - 30
        assert_eq!(rects[2].width(), 30.0);
        assert_eq!(rects[2].x(), 170.0); // 30 + 140
    }
}
