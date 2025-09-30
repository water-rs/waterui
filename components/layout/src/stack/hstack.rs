use alloc::{vec, vec::Vec};
use waterui_core::view::TupleViews;

use crate::{Layout, Point, ProposalSize, Rect, Size, container, stack::VerticalAlignment};

#[derive(Debug, Clone)]
pub struct HStackLayout {
    pub alignment: VerticalAlignment,
    pub spacing: f64,
}

impl Layout for HStackLayout {
    fn propose(
        &mut self,
        parent: ProposalSize,
        children: &[crate::ChildMetadata],
    ) -> Vec<ProposalSize> {
        // For HStack, we propose the full height of the parent and an unconstrained width to each child
        let child_width = f64::INFINITY;
        let child_height = parent.height;
        let child_proposal = ProposalSize::new(child_width, child_height);

        vec![child_proposal; children.len()]
    }
    #[allow(clippy::cast_precision_loss)]
    fn size(&mut self, parent: ProposalSize, children: &[crate::ChildMetadata]) -> Size {
        // Firstly, let's sum up children's ideal widths (None = 0)
        let mut ideal_width: f64 = children
            .iter()
            .map(|c| c.proposal_width().unwrap_or_default())
            .sum();

        // Add spacing between children
        ideal_width += self.spacing * (children.len().saturating_sub(1) as f64);

        // HStack cannot exceed parent's proposed width
        let final_width = ideal_width.min(parent.width.unwrap_or(f64::INFINITY));

        // Height is determined by parent's proposed height
        Size::new(final_width, parent.height.unwrap_or(f64::INFINITY))
    }

    fn place(
        &mut self,
        bound: Rect,
        proposal: ProposalSize,
        children: &[crate::ChildMetadata],
    ) -> Vec<Rect> {
        // Let's place children horizontally within the given bound
        // Tip: We return the size we required in `size` method,
        // however, if the parent gives us less space, we need to adjust accordingly.
        if children.is_empty() {
            return Vec::new();
        }

        let spacing_count = children.len().saturating_sub(1);
        let mut spacing = self.spacing.max(0.0);

        if spacing_count > 0 && spacing.is_finite() && !bound.width().is_infinite() {
            let max_spacing = bound.width().max(0.0) / spacing_count as f64;
            spacing = spacing.min(max_spacing);
        }

        let total_spacing = spacing * spacing_count as f64;

        let mut widths: Vec<f64> = children
            .iter()
            .map(|child| child.proposal_width().unwrap_or_default().max(0.0))
            .collect();

        let total_ideal_width: f64 = widths.iter().sum();

        let mut available_width = (bound.width() - total_spacing).max(0.0);
        if !available_width.is_finite() {
            available_width = total_ideal_width;
        }

        if available_width < total_ideal_width {
            let stretch_indices: Vec<usize> = children
                .iter()
                .enumerate()
                .filter_map(|(idx, child)| child.stretch().then_some(idx))
                .collect();

            let stretch_total: f64 = stretch_indices.iter().map(|&idx| widths[idx]).sum();
            if stretch_total > 0.0 {
                let deficit = total_ideal_width - available_width;
                for &idx in &stretch_indices {
                    let ratio = widths[idx] / stretch_total;
                    let reduce = (deficit * ratio).min(widths[idx]);
                    widths[idx] -= reduce;
                }
            }

            let current_total: f64 = widths.iter().sum();
            if current_total > available_width && current_total > 0.0 {
                let scale = (available_width / current_total).max(0.0);
                for width in &mut widths {
                    *width *= scale;
                }
            } else if available_width <= 0.0 {
                for width in widths.iter_mut() {
                    *width = 0.0;
                }
            }
        } else if available_width > total_ideal_width {
            let stretch_indices: Vec<usize> = children
                .iter()
                .enumerate()
                .filter_map(|(idx, child)| child.stretch().then_some(idx))
                .collect();

            if !stretch_indices.is_empty() {
                let extra = available_width - total_ideal_width;
                if extra.is_finite() {
                    let share = extra / stretch_indices.len() as f64;
                    for &idx in &stretch_indices {
                        widths[idx] += share;
                    }
                }
            }
        }

        let mut rects = Vec::with_capacity(children.len());
        let mut cursor_x = bound.x();
        let available_height = bound.height();
        let fallback_height = proposal.height.unwrap_or_else(|| {
            if available_height.is_finite() {
                available_height
            } else {
                0.0
            }
        });

        for (idx, child) in children.iter().enumerate() {
            let mut child_height = child.proposal_height().unwrap_or(fallback_height);
            if child_height.is_nan() {
                child_height = fallback_height;
            }

            let mut y = bound.y();
            if available_height.is_finite() {
                child_height = child_height.min(available_height).max(0.0);
                y = match self.alignment {
                    VerticalAlignment::Top => bound.y(),
                    VerticalAlignment::Center => {
                        bound.y() + (available_height - child_height) / 2.0
                    }
                    VerticalAlignment::Bottom => bound.max_y() - child_height,
                };
            } else if !child_height.is_finite() {
                child_height = 0.0;
            }

            rects.push(Rect::new(
                Point::new(cursor_x, y),
                Size::new(widths[idx], child_height),
            ));

            cursor_x += widths[idx];
            if idx + 1 < children.len() {
                cursor_x += spacing;
            }
        }

        rects
    }
}

container!(HStack, HStackLayout);

impl HStack {
    pub fn new(alignment: VerticalAlignment, spacing: f64, contents: impl TupleViews) -> Self {
        Self {
            layout: HStackLayout { alignment, spacing },
            contents: contents.into_views(),
        }
    }
}

pub fn hstack(contents: impl TupleViews) -> HStack {
    HStack::new(VerticalAlignment::Center, 10.0, contents)
}