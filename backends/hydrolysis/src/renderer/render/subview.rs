use super::*;
use std::cell::RefCell;
use waterui_core::MainThreadBound;

/// Layout proxy that measures a child view through the hydrolysis renderer.
///
/// Measurement borrows the renderer's `HydroState` and a `!Send` `Environment` and
/// recurses into arbitrary (possibly reactive) child bodies, so the subview is
/// confined to the main thread — it reports `require_main_thread() == true`, and the
/// `MainThreadBound` assertions are the fail-fast safety net if the layout executor
/// ever schedules it on a worker.
#[derive(Debug)]
pub(crate) struct HydroSubview<'a> {
    view: MainThreadBound<&'a AnyView>,
    state: MainThreadBound<&'a RefCell<&'a mut HydroState>>,
    env: MainThreadBound<Environment>,
    stretch_axis: StretchAxis,
    measure_cache: MainThreadBound<RefCell<Vec<(ProposalSize, ViewDimensions)>>>,
}

impl<'a> HydroSubview<'a> {
    pub(crate) fn from_view(
        view: &'a AnyView,
        state: &'a RefCell<&'a mut HydroState>,
        env: &'a Environment,
    ) -> Self {
        Self {
            view: MainThreadBound::new(view),
            state: MainThreadBound::new(state),
            env: MainThreadBound::new(env.clone()),
            stretch_axis: effective_stretch_axis(view),
            measure_cache: MainThreadBound::new(RefCell::new(Vec::new())),
        }
    }

    pub(crate) fn view(&self) -> &'a AnyView {
        *self.view
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
            measure_view_dimensions_with_proposal(*self.view, proposal, &mut state, &self.env);

        if self.stretch_axis.stretches_horizontal()
            && let Some(width) = proposal.width
        {
            dimensions.size.width = dimensions.size.width.max(width);
        }

        if self.stretch_axis.stretches_vertical() {
            if let Some(height) = proposal.height {
                dimensions.size.height = dimensions.size.height.max(height);
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

    fn require_main_thread(&self) -> bool {
        true
    }
}

impl core::fmt::Debug for HydrolysisRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisRenderer")
            .field("dispatcher", &self.dispatcher)
            .finish_non_exhaustive()
    }
}
