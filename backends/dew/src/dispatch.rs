//! View dispatch: turning a `WaterUI` view tree into a [`DisplayList`].
//!
//! Mirrors the hydrolysis dispatcher at a smaller scale: a type-id → handler
//! map over [`AnyView`], with unknown views resolved through their `body()`
//! (Vue-like reconstruction). Before dispatch, the tree is *normalized*:
//! every node is body-expanded until it is a renderable native type, and
//! container children are normalized recursively so measurement can walk
//! the tree without re-evaluating bodies.
//!
//! Reactive inputs are read through [`DewRenderer::read_signal`], which
//! registers a watcher that requests a structural rebuild on change. The
//! runtime then diffs the rebuilt display list against the previous one so
//! only genuinely changed screen regions are re-rasterized and flushed —
//! the rebuild is cheap Rust-side work; the flush economy stays
//! fine-grained. Per-`Dynamic` retained patching (hydrolysis-style) is the
//! planned next refinement.

use core::any::TypeId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use kurbo::{Affine, Rect};
use nami::Signal;
use waterui_backend_core::frame_signals::FrameSignals;
use waterui_backend_core::time::Instant;
use waterui_core::layout::{
    ProposalSize, Rect as LayoutRect, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_core::{AnyView, Environment, Metadata, Native, Str, View};
use waterui_graphics::color::ResolvedColor;
use waterui_layout::container::FixedContainer;
use waterui_layout::spacer::Spacer;
use waterui_text::TextConfig;

use crate::display_list::DisplayList;
use crate::text::{DewState, emit_text_commands, styled_to_plain};

/// Maximum `body()` expansions before normalization gives up — a guard
/// against views whose body never reaches a native type.
const MAX_BODY_DEPTH: usize = 64;

/// Where a view draws: the accumulated transform and its local bounds.
#[derive(Clone, Copy, Debug)]
pub struct RenderContext {
    /// Local-to-window transform for this view.
    pub transform: Affine,
    /// Bounds in local coordinates (origin is this view's top-left).
    pub bounds: Rect,
}

impl RenderContext {
    /// The root context covering a `width` × `height` window.
    #[must_use]
    pub const fn root(width: f64, height: f64) -> Self {
        Self {
            transform: Affine::IDENTITY,
            bounds: Rect::new(0.0, 0.0, width, height),
        }
    }

    /// A child context placed at `frame` (logical pixels) inside this one.
    #[must_use]
    pub fn child(self, frame: LayoutRect) -> Self {
        Self {
            transform: self.transform
                * Affine::translate((f64::from(frame.x()), f64::from(frame.y()))),
            bounds: Rect::new(
                0.0,
                0.0,
                f64::from(frame.width()),
                f64::from(frame.height()),
            ),
        }
    }
}

type DewHandlerFn = Box<dyn Fn(&mut DewRenderer, RenderContext, AnyView, &Environment)>;

/// Type-id keyed handler table, cheaply cloneable so handlers can re-enter
/// dispatch while the renderer is mutably borrowed.
#[derive(Clone, Default)]
struct DewDispatcher {
    handlers: Rc<HashMap<TypeId, DewHandlerFn>>,
}

impl DewDispatcher {
    fn register<V: View>(
        &mut self,
        handler: impl 'static + Fn(&mut DewRenderer, RenderContext, V, &Environment),
    ) {
        let handlers = Rc::get_mut(&mut self.handlers)
            .unwrap_or_else(|| panic!("dew dispatcher handlers cannot be mutated after cloning"));
        handlers.insert(
            TypeId::of::<V>(),
            Box::new(move |renderer, ctx, view, env| {
                let view = *view.downcast::<V>().expect("dew dispatch type mismatch");
                handler(renderer, ctx, view, env);
            }),
        );
    }

    fn supports(&self, type_id: TypeId) -> bool {
        self.handlers.contains_key(&type_id)
    }

    fn dispatch_boxed(
        &self,
        renderer: &mut DewRenderer,
        view: AnyView,
        env: &Environment,
        ctx: RenderContext,
    ) {
        if let Some(handler) = self.handlers.get(&view.type_id()) {
            handler(renderer, ctx, view, env);
            return;
        }
        let body = AnyView::new(view.body(env));
        self.dispatch_boxed(renderer, body, env, ctx);
    }
}

/// The dew renderer: dispatches a view tree into a retained
/// [`DisplayList`] and tracks the reactive inputs it read along the way.
pub struct DewRenderer {
    dispatcher: DewDispatcher,
    signals: FrameSignals,
    state: RefCell<DewState>,
    list: DisplayList,
    watch_guards: Vec<Box<dyn core::any::Any>>,
}

impl core::fmt::Debug for DewDispatcher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DewDispatcher")
            .field("handlers", &self.handlers.len())
            .finish()
    }
}

impl core::fmt::Debug for DewRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DewRenderer")
            .field("dispatcher", &self.dispatcher)
            .field("list", &self.list)
            .field("watchers", &self.watch_guards.len())
            .finish_non_exhaustive()
    }
}

impl Default for DewRenderer {
    fn default() -> Self {
        Self::new(FrameSignals::new(Instant::now()))
    }
}

impl DewRenderer {
    /// Creates a renderer wired to `signals` for rebuild requests.
    #[must_use]
    pub fn new(signals: FrameSignals) -> Self {
        let mut dispatcher = DewDispatcher::default();
        register_core_handlers(&mut dispatcher);
        Self {
            dispatcher,
            signals,
            state: RefCell::new(DewState::default()),
            list: DisplayList::new(),
            watch_guards: Vec::new(),
        }
    }

    /// The frame-trigger handle shared with the runtime loop.
    #[must_use]
    pub fn signals(&self) -> FrameSignals {
        self.signals.clone()
    }

    /// Dispatches `view` into a fresh display list for a `width` × `height`
    /// window, replacing all reactive watchers from the previous tree.
    pub fn render_tree(
        &mut self,
        view: AnyView,
        env: &Environment,
        width: f64,
        height: f64,
    ) -> DisplayList {
        self.list.clear();
        self.watch_guards.clear();
        let normalized = self.normalize(view, env);
        let dispatcher = self.dispatcher.clone();
        dispatcher.dispatch_boxed(self, normalized, env, RenderContext::root(width, height));
        core::mem::take(&mut self.list)
    }

    /// Reads a reactive input, registering a rebuild-on-change watcher whose
    /// lifetime is the current tree.
    pub(crate) fn read_signal<S>(&mut self, signal: &S) -> S::Output
    where
        S: Signal + Clone + 'static,
    {
        let signals = self.signals.clone();
        let guard = signal.watch(move |_| signals.request_rebuild());
        self.watch_guards.push(Box::new(guard));
        signal.get()
    }

    /// Body-expands `view` until it is a renderable native type, recursing
    /// into container children so the whole tree becomes measurable.
    ///
    /// # Panics
    ///
    /// Panics when a view fails to normalize within [`MAX_BODY_DEPTH`]
    /// expansions — dew does not silently skip unsupported views.
    fn normalize(&mut self, view: AnyView, env: &Environment) -> AnyView {
        let mut view = view;
        for _ in 0..MAX_BODY_DEPTH {
            let type_id = view.type_id();
            if type_id == TypeId::of::<Metadata<Environment>>() {
                let Metadata { content, value } = *view
                    .downcast::<Metadata<Environment>>()
                    .expect("dew normalize environment metadata downcast");
                let content = self.normalize(content, &value);
                return AnyView::new(Metadata { content, value });
            }
            if type_id == TypeId::of::<Native<FixedContainer>>() {
                let native = *view
                    .downcast::<Native<FixedContainer>>()
                    .expect("dew normalize container downcast");
                let (layout, contents) = native.into_inner().into_inner();
                let contents: Vec<AnyView> = contents
                    .into_iter()
                    .map(|child| self.normalize(child, env))
                    .collect();
                return AnyView::new(Native::new(FixedContainer::from_parts(layout, contents)));
            }
            if self.dispatcher.supports(type_id) {
                return view;
            }
            view = AnyView::new(view.body(env));
        }
        panic!(
            "dew: view did not normalize to a renderable type within {MAX_BODY_DEPTH} body expansions"
        );
    }
}

/// Measures a *normalized* view.
///
/// # Panics
///
/// Panics on un-normalized view types: measurement never evaluates bodies.
pub(crate) fn measure_view(
    state: &RefCell<DewState>,
    view: &AnyView,
    env: &Environment,
    proposal: ProposalSize,
) -> ViewDimensions {
    if let Some(metadata) = view.downcast_ref::<Metadata<Environment>>() {
        return measure_view(state, &metadata.content, &metadata.value, proposal);
    }
    if let Some(native) = view.downcast_ref::<Native<FixedContainer>>() {
        let (layout, contents) = native.as_inner().as_parts();
        let subviews: Vec<DewSubview> = contents
            .iter()
            .map(|child| DewSubview::new(child, state, env))
            .collect();
        let refs: Vec<&dyn SubView> = subviews.iter().map(|s| s as &dyn SubView).collect();
        return ViewDimensions::new(layout.size_that_fits(proposal, &refs));
    }
    if let Some(text) = view.downcast_ref::<Native<TextConfig>>() {
        let styled = text.as_inner().content.get();
        let plain = styled_to_plain(&styled);
        let (width, height) = state.borrow_mut().measure_text(&plain, proposal.width);
        return ViewDimensions::new(Size::new(width, height));
    }
    if let Some(text) = view.downcast_ref::<Str>() {
        let (width, height) = state.borrow_mut().measure_text(text, proposal.width);
        return ViewDimensions::new(Size::new(width, height));
    }
    if view.downcast_ref::<Native<ResolvedColor>>().is_some() {
        return ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ));
    }
    if view.downcast_ref::<Native<Spacer>>().is_some() {
        return ViewDimensions::new(Size::new(0.0, 0.0));
    }
    panic!("dew cannot measure un-normalized view; normalize the tree before measuring");
}

/// A measurable child handed to a [`waterui_core::layout::Layout`]
/// implementation.
struct DewSubview<'a> {
    view: &'a AnyView,
    state: &'a RefCell<DewState>,
    env: Environment,
    stretch: StretchAxis,
}

impl<'a> DewSubview<'a> {
    fn new(view: &'a AnyView, state: &'a RefCell<DewState>, env: &Environment) -> Self {
        Self {
            view,
            state,
            env: env.clone(),
            stretch: effective_stretch_axis(view),
        }
    }
}

impl SubView for DewSubview<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure_view(self.state, self.view, &self.env, proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The stretch axis of a view, looking through environment-metadata
/// wrappers (which would otherwise report no stretch).
fn effective_stretch_axis(view: &AnyView) -> StretchAxis {
    view.downcast_ref::<Metadata<Environment>>().map_or_else(
        || view.stretch_axis(),
        |metadata| effective_stretch_axis(&metadata.content),
    )
}

fn register_core_handlers(dispatcher: &mut DewDispatcher) {
    dispatcher.register::<Native<FixedContainer>>(render_container);
    dispatcher.register::<Native<ResolvedColor>>(render_resolved_color);
    dispatcher.register::<Native<Spacer>>(render_spacer);
    dispatcher.register::<Native<TextConfig>>(render_text_config);
    dispatcher.register::<Str>(render_str);
    dispatcher.register::<Metadata<Environment>>(render_environment_metadata);
}

fn render_environment_metadata(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    metadata: Metadata<Environment>,
    _env: &Environment,
) {
    let dispatcher = renderer.dispatcher.clone();
    dispatcher.dispatch_boxed(renderer, metadata.content, &metadata.value, ctx);
}

fn render_container(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<FixedContainer>,
    env: &Environment,
) {
    let (layout, contents) = view.into_inner().into_inner();
    let frames = {
        let subviews: Vec<DewSubview> = contents
            .iter()
            .map(|child| DewSubview::new(child, &renderer.state, env))
            .collect();
        let refs: Vec<&dyn SubView> = subviews.iter().map(|s| s as &dyn SubView).collect();
        let proposal = proposal_from_bounds(ctx.bounds);
        let size = layout.size_that_fits(proposal, &refs);
        layout.place(LayoutRect::from_size(size), &refs)
    };
    let dispatcher = renderer.dispatcher.clone();
    for (child, frame) in contents.into_iter().zip(frames) {
        dispatcher.dispatch_boxed(renderer, child, env, ctx.child(frame));
    }
}

fn render_resolved_color(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<ResolvedColor>,
    _env: &Environment,
) {
    let color = view.into_inner();
    let srgb = color.to_srgb_with_headroom();
    let paint = peniko::Color::new([srgb.red, srgb.green, srgb.blue, color.opacity]);
    renderer.list.fill(&ctx.bounds, ctx.transform, paint);
}

fn render_spacer(
    _renderer: &mut DewRenderer,
    _ctx: RenderContext,
    _view: Native<Spacer>,
    _env: &Environment,
) {
}

fn render_text_config(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<TextConfig>,
    _env: &Environment,
) {
    let config = view.into_inner();
    let styled = renderer.read_signal(&config.content);
    render_plain_text(renderer, ctx, &styled_to_plain(&styled));
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "dispatcher handlers receive views by value per the handler contract"
)]
fn render_str(renderer: &mut DewRenderer, ctx: RenderContext, text: Str, _env: &Environment) {
    render_plain_text(renderer, ctx, text.as_str());
}

fn render_plain_text(renderer: &mut DewRenderer, ctx: RenderContext, text: &str) {
    let layout = renderer
        .state
        .borrow_mut()
        .build_text_layout(text, max_width_from_bounds(ctx.bounds));
    let transform = ctx.transform * Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
    emit_text_commands(&mut renderer.list, &layout, transform);
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "logical-pixel bounds are far below f32 precision limits"
)]
fn proposal_from_bounds(bounds: Rect) -> ProposalSize {
    ProposalSize::new(Some(bounds.width() as f32), Some(bounds.height() as f32))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "logical-pixel bounds are far below f32 precision limits"
)]
fn max_width_from_bounds(bounds: Rect) -> Option<f32> {
    (bounds.width() > 0.0).then(|| bounds.width() as f32)
}
