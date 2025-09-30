use alloc::{vec, vec::Vec};
use waterui_core::view::TupleViews;

use crate::{Layout, Point, ProposalSize, Rect, Size, container, stack::HorizontalAlignment};

#[derive(Debug, Clone)]
pub struct VStackLayout {
    pub alignment: HorizontalAlignment,
    pub spacing: f64,
}

impl Layout for VStackLayout {
    fn propose(
        &mut self,
        parent: ProposalSize,
        children: &[crate::ChildMetadata],
    ) -> Vec<ProposalSize> {
        // For VStack, we propose the full width of the parent and an unconstrained height to each child
        let child_width = parent.width;
        let child_height = f64::INFINITY;
        let child_proposal = ProposalSize::new(child_width, child_height);

        vec![child_proposal; children.len()]
    }
    #[allow(clippy::cast_precision_loss)]
    fn size(&mut self, parent: ProposalSize, children: &[crate::ChildMetadata]) -> Size {
        // Firstly, let's sum up children's ideal heights (None = 0)
        let mut ideal_height: f64 = children
            .iter()
            .map(|c| c.proposal_height().unwrap_or_default())
            .sum();

        // Add spacing between children
        ideal_height += self.spacing * (children.len().saturating_sub(1) as f64);

        // VStack cannot exceed parent's proposed height
        let final_height = ideal_height.min(parent.height.unwrap_or(f64::INFINITY));

        // Width is determined by parent's proposed width
        Size::new(parent.width.unwrap_or(f64::INFINITY), final_height)
    }

    fn place(
        &mut self,
        bound: Rect,
        proposal: ProposalSize,
        children: &[crate::ChildMetadata],
    ) -> Vec<Rect> {
        // Let's place children vertically within the given bound
        // Tip: We return the size we required in `size` method,
        // however, if the parent gives us less space, we need to adjust accordingly.
        if children.is_empty() {
            return Vec::new();
        }

        let spacing_count = children.len().saturating_sub(1);
        let mut spacing = self.spacing.max(0.0);

        if spacing_count > 0 && spacing.is_finite() && !bound.height().is_infinite() {
            let max_spacing = bound.height().max(0.0) / spacing_count as f64;
            spacing = spacing.min(max_spacing);
        }

        let total_spacing = spacing * spacing_count as f64;

        let mut heights: Vec<f64> = children
            .iter()
            .map(|child| child.proposal_height().unwrap_or_default().max(0.0))
            .collect();

        let total_ideal_height: f64 = heights.iter().sum();

        let mut available_height = (bound.height() - total_spacing).max(0.0);
        if !available_height.is_finite() {
            available_height = total_ideal_height;
        }

        if available_height < total_ideal_height {
            let stretch_indices: Vec<usize> = children
                .iter()
                .enumerate()
                .filter_map(|(idx, child)| child.stretch().then_some(idx))
                .collect();

            let stretch_total: f64 = stretch_indices.iter().map(|&idx| heights[idx]).sum();
            if stretch_total > 0.0 {
                let deficit = total_ideal_height - available_height;
                for &idx in &stretch_indices {
                    let ratio = heights[idx] / stretch_total;
                    let reduce = (deficit * ratio).min(heights[idx]);
                    heights[idx] -= reduce;
                }
            }

            let current_total: f64 = heights.iter().sum();
            if current_total > available_height && current_total > 0.0 {
                let scale = (available_height / current_total).max(0.0);
                for height in &mut heights {
                    *height *= scale;
                }
            } else if available_height <= 0.0 {
                for height in heights.iter_mut() {
                    *height = 0.0;
                }
            }
        } else if available_height > total_ideal_height {
            let stretch_indices: Vec<usize> = children
                .iter()
                .enumerate()
                .filter_map(|(idx, child)| child.stretch().then_some(idx))
                .collect();

            if !stretch_indices.is_empty() {
                let extra = available_height - total_ideal_height;
                if extra.is_finite() {
                    let share = extra / stretch_indices.len() as f64;
                    for &idx in &stretch_indices {
                        heights[idx] += share;
                    }
                }
            }
        }

        let mut rects = Vec::with_capacity(children.len());
        let mut cursor_y = bound.y();
        let available_width = bound.width();
        let fallback_width = proposal.width.unwrap_or_else(|| {
            if available_width.is_finite() {
                available_width
            } else {
                0.0
            }
        });

        for (idx, child) in children.iter().enumerate() {
            let mut child_width = child.proposal_width().unwrap_or(fallback_width);
            if child_width.is_nan() {
                child_width = fallback_width;
            }

            let mut x = bound.x();
            if available_width.is_finite() {
                child_width = child_width.min(available_width).max(0.0);
                x = match self.alignment {
                    HorizontalAlignment::Leading => bound.x(),
                    HorizontalAlignment::Center => {
                        bound.x() + (available_width - child_width) / 2.0
                    }
                    HorizontalAlignment::Trailing => bound.max_x() - child_width,
                };
            } else if !child_width.is_finite() {
                child_width = 0.0;
            }

            rects.push(Rect::new(
                Point::new(x, cursor_y),
                Size::new(child_width, heights[idx]),
            ));

            cursor_y += heights[idx];
            if idx + 1 < children.len() {
                cursor_y += spacing;
            }
        }

        rects
    }
}

container!(VStack, VStackLayout);

impl VStack {
    pub fn new(alignment: HorizontalAlignment, spacing: f64, contents: impl TupleViews) -> Self {
        Self {
            layout: VStackLayout { alignment, spacing },
            contents: contents.into_views(),
        }
    }
}

pub fn vstack(contents: impl TupleViews) -> VStack {
    VStack::new(HorizontalAlignment::Center, 10.0, contents)
}
