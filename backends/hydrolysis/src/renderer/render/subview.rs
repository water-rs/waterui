use super::*;
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub(crate) struct HydroSubview<'a> {
    view: &'a AnyView,
    state: &'a RefCell<&'a mut HydroState>,
    env: Environment,
    stretch_axis: StretchAxis,
    measure_cache: RefCell<Vec<(ProposalSize, ViewDimensions)>>,
}

impl<'a> HydroSubview<'a> {
    pub(crate) fn from_view(
        view: &'a AnyView,
        state: &'a RefCell<&'a mut HydroState>,
        env: &'a Environment,
    ) -> Self {
        Self {
            view,
            state,
            env: env.clone(),
            stretch_axis: effective_stretch_axis(view),
            measure_cache: RefCell::new(Vec::new()),
        }
    }

    pub(crate) const fn view(&self) -> &'a AnyView {
        self.view
    }
}

impl SubView for HydroSubview<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        if let Some((_, dimensions)) = self
            .measure_cache
            .borrow()
            .iter()
            .find(|(cached_proposal, _)| *cached_proposal == proposal)
        {
            return dimensions.clone();
        }

        let mut state = self.state.borrow_mut();
        let mut dimensions =
            measure_view_dimensions_with_proposal(self.view, proposal, &mut state, &self.env);

        if self.stretch_axis.stretches_horizontal() {
            if let Some(width) = proposal.width {
                dimensions.size.width = width;
            }
        } else if let Some(width) = proposal.width {
            dimensions.size.width = dimensions.size.width.min(width);
        }

        if self.stretch_axis.stretches_vertical() {
            if let Some(height) = proposal.height {
                dimensions.size.height = height;
            }
        } else if let Some(height) = proposal.height {
            dimensions.size.height = dimensions.size.height.min(height);
        }

        self.measure_cache
            .borrow_mut()
            .push((proposal, dimensions.clone()));
        dimensions
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis
    }

    fn priority(&self) -> i32 {
        0
    }
}

impl core::fmt::Debug for HydrolysisRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisRenderer")
            .field("dispatcher", &self.dispatcher)
            .finish_non_exhaustive()
    }
}
