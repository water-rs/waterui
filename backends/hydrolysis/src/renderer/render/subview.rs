use super::*;
use std::cell::RefCell;
use std::sync::Arc;
use waterui_core::MainThreadBound;

/// Layout proxy that measures a child view through the hydrolysis renderer.
///
/// Most children measure by recursing into arbitrary (possibly reactive) bodies,
/// which borrows the renderer's `HydroState` and a `!Send` `Environment`. Those
/// measurements are confined to the main thread: the relevant fields are wrapped
/// in [`MainThreadBound`] and the subview reports `require_main_thread() == true`.
///
/// Text leaves are different. Hydrolysis fully resolves a `Text`/`Str` node on
/// the main thread (reading its signals and theme) into a `Send`
/// [`ResolvedTextLayoutInput`]; the remaining work — shaping it into a layout and
/// reading line metrics — is pure and runs through the thread-safe
/// [`TextMeasureService`]. For those children `require_main_thread()` returns
/// `false`, so the layout executor measures them across worker threads.
pub(crate) struct HydroSubview<'a> {
    view: MainThreadBound<&'a AnyView>,
    state: MainThreadBound<&'a RefCell<&'a mut HydroState>>,
    env: MainThreadBound<Environment>,
    stretch_axis: StretchAxis,
    measure_cache: MainThreadBound<RefCell<Vec<(ProposalSize, ViewDimensions)>>>,
    /// Present when this child is a text leaf resolved on the main thread. When
    /// set, `measure` shapes through `service` on any thread and the subview is
    /// worker-safe.
    resolved_text: Option<ResolvedTextMeasure>,
}

/// A text leaf resolved on the main thread, ready to be shaped on any thread.
struct ResolvedTextMeasure {
    input: ResolvedTextLayoutInput,
    service: Arc<TextMeasureService>,
}

impl<'a> HydroSubview<'a> {
    pub(crate) fn from_view(
        view: &'a AnyView,
        state: &'a RefCell<&'a mut HydroState>,
        env: &'a Environment,
    ) -> Self {
        let resolved_text = try_resolve_text_leaf(view, env).map(|input| ResolvedTextMeasure {
            input,
            service: Arc::clone(&state.borrow().text),
        });
        Self {
            view: MainThreadBound::new(view),
            state: MainThreadBound::new(state),
            env: MainThreadBound::new(env.clone()),
            stretch_axis: effective_stretch_axis(view),
            measure_cache: MainThreadBound::new(RefCell::new(Vec::new())),
            resolved_text,
        }
    }

    pub(crate) fn view(&self) -> &'a AnyView {
        *self.view
    }

    /// Apply this child's stretch axis to a measured size against the proposal.
    /// Shared by the worker-safe text path and the main-thread recursion path so
    /// both produce identical dimensions.
    fn apply_stretch(
        &self,
        mut dimensions: ViewDimensions,
        proposal: ProposalSize,
    ) -> ViewDimensions {
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

        dimensions
    }
}

impl SubView for HydroSubview<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        // Worker-safe path: shaping a resolved text leaf touches no
        // `MainThreadBound` state, so it may run on any thread.
        if let Some(resolved) = &self.resolved_text {
            let layout = resolved.service.shape(&resolved.input, proposal.width);
            let dimensions = text_dimensions_from_layout(&layout, None);
            return self.apply_stretch(dimensions, proposal);
        }

        if let Some((_, dimensions)) = self
            .measure_cache
            .borrow()
            .iter()
            .find(|(cached_proposal, _)| *cached_proposal == proposal)
        {
            return dimensions.clone();
        }

        let dimensions = {
            let mut state = self.state.borrow_mut();
            measure_view_dimensions_with_proposal(*self.view, proposal, &mut state, &self.env)
        };
        let dimensions = self.apply_stretch(dimensions, proposal);

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
        self.resolved_text.is_none()
    }
}

/// Resolve a text leaf (`Str`/`Text`, possibly behind environment metadata or a
/// passthrough wrapper) into a `Send` layout input on the main thread.
///
/// Mirrors the text handling in
/// [`measure_view_dimensions_with_proposal`](super::measure_view_dimensions_with_proposal)
/// so the worker-thread fast path and the main-thread recursion produce the same
/// dimensions. Returns `None` for any other view, which then measures on the
/// main thread.
fn try_resolve_text_leaf(view: &AnyView, env: &Environment) -> Option<ResolvedTextLayoutInput> {
    let (view, scoped_env) = flatten_environment_metadata_ref(view, env);

    if let Some(content) = passthrough_content(view) {
        return try_resolve_text_leaf(content, &scoped_env);
    }

    if let Some(text) = view.downcast_ref::<Str>() {
        return Some(resolve_text_layout_input(
            &StyledStr::plain(text.clone()),
            HorizontalAlignment::Leading,
            &scoped_env,
        ));
    }

    if let Some(text) = view.downcast_ref::<Text>() {
        let resolved = text.resolve(&scoped_env);
        return Some(resolve_text_layout_input(
            &resolved.content.get(),
            resolved.paragraph_alignment.get(),
            &scoped_env,
        ));
    }

    None
}

impl core::fmt::Debug for HydroSubview<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroSubview")
            .field("stretch_axis", &self.stretch_axis)
            .field("worker_safe", &self.resolved_text.is_some())
            .finish_non_exhaustive()
    }
}

impl core::fmt::Debug for HydrolysisRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisRenderer")
            .field("dispatcher", &self.dispatcher)
            .finish_non_exhaustive()
    }
}
