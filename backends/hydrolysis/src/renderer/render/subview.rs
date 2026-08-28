use super::*;
use std::cell::RefCell;
use std::sync::Arc;
use waterui_core::MainThreadBound;

/// Layout proxy that measures a child view through the hydrolysis renderer.
///
/// A text leaf is fully resolved during tree-build (reading its signals and theme)
/// into a [`ResolvedTextLayoutInput`], and `measure` then shapes it through the
/// content-keyed [`TextMeasureService`] directly. That shortcut skips the general
/// path, which measures by recursing into arbitrary (possibly reactive) bodies and
/// so has to borrow the renderer's `HydroState` and `Environment`.
pub(crate) struct HydroSubview<'a> {
    view: MainThreadBound<&'a AnyView>,
    state: MainThreadBound<&'a RefCell<&'a mut HydroState>>,
    env: MainThreadBound<Environment>,
    stretch_axis: StretchAxis,
    /// Per-proposal memo for this layout pass (containers probe children with
    /// repeated proposals). Only the recursion path caches here; the text path
    /// memoizes in the content-keyed [`TextMeasureService`] instead.
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
    /// Maximum laid-out lines, from the leaf's `TextConfig::line_limit`.
    max_lines: Option<usize>,
}

impl<'a> HydroSubview<'a> {
    pub(crate) fn from_view(
        view: &'a AnyView,
        state: &'a RefCell<&'a mut HydroState>,
        env: &'a Environment,
    ) -> Self {
        let resolved_text =
            try_resolve_text_leaf(view, env).map(|(input, max_lines)| ResolvedTextMeasure {
                input,
                service: Arc::clone(&state.borrow().text),
                max_lines,
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

        if self.stretch_axis.stretches_vertical()
            && let Some(height) = proposal.height
        {
            dimensions.size.height = dimensions.size.height.max(height);
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
            let dimensions = text_dimensions_from_layout(&layout, resolved.max_lines);
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
}

/// Resolve a text leaf into a `Send` layout input on the main thread, or `None`
/// for any non-text view (which then measures on the main thread).
///
/// Covers every shape text reaches the layout in: `Str`, `Text`, the lowered
/// `Native<TextConfig>`, and the string-likes (`&str`/`String`/`Cow`) that render
/// as plain text. The string-likes are bodied on the main thread (cheap) and
/// recursed so the heavy shaping still happens off the main thread. Mirrors the
/// text handling in
/// [`measure_view_dimensions_with_proposal`](super::measure_view_dimensions_with_proposal)
/// so the worker-thread fast path and the main-thread recursion agree.
fn try_resolve_text_leaf(
    view: &AnyView,
    env: &Environment,
) -> Option<(ResolvedTextLayoutInput, Option<usize>)> {
    let (view, scoped_env) = flatten_environment_metadata_ref(view, env);

    if let Some(content) = passthrough_content(view) {
        return try_resolve_text_leaf(content, &scoped_env);
    }

    if let Some(text) = view.downcast_ref::<Str>() {
        return Some((
            resolve_text_layout_input(
                &StyledStr::plain(text.clone()),
                HorizontalAlignment::Leading,
                &scoped_env,
            ),
            None,
        ));
    }

    if let Some(text) = view.downcast_ref::<Text>() {
        let resolved = text.resolve(&scoped_env);
        return Some((
            resolve_text_layout_input(
                &resolved.content.get(),
                resolved.paragraph_alignment.get(),
                &scoped_env,
            ),
            resolved.line_limit.map(core::num::NonZeroUsize::get),
        ));
    }

    if let Some(text) = view.downcast_ref::<Native<TextConfig>>() {
        let config = text.as_inner();
        return Some((
            resolve_text_layout_input(
                &config.content.get(),
                config.paragraph_alignment.get(),
                &scoped_env,
            ),
            config.line_limit.map(core::num::NonZeroUsize::get),
        ));
    }

    if let Some(text) = view.downcast_ref::<&'static str>() {
        let body = AnyView::new((*text).body(&scoped_env));
        return try_resolve_text_leaf(&body, &scoped_env);
    }
    if let Some(text) = view.downcast_ref::<String>() {
        let body = AnyView::new(text.clone().body(&scoped_env));
        return try_resolve_text_leaf(&body, &scoped_env);
    }
    if let Some(text) = view.downcast_ref::<Cow<'static, str>>() {
        let body = AnyView::new(text.clone().body(&scoped_env));
        return try_resolve_text_leaf(&body, &scoped_env);
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
        f.debug_struct("HydrolysisRenderer").finish_non_exhaustive()
    }
}
