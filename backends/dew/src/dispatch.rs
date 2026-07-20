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
#[cfg(feature = "progress")]
use waterui::component::progress::ProgressConfig;
use waterui_backend_core::frame_signals::FrameSignals;
use waterui_backend_core::time::Instant;
use waterui_controls::slider::SliderConfig;
use waterui_controls::stepper::StepperConfig;
use waterui_controls::text_field::ResolvedTextFieldConfig;
use waterui_controls::toggle::ToggleConfig;
use waterui_core::layout::{
    ProposalSize, Rect as LayoutRect, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_core::{AnyView, Environment, MainThreadBound, Metadata, Native, Str, View};
use waterui_graphics::color::{Color, ResolvedColor};
use waterui_layout::Divider;
use waterui_layout::container::FixedContainer;
use waterui_layout::scroll::ScrollView;
use waterui_layout::spacer::Spacer;
use waterui_text::TextConfig;

use crate::display_list::DisplayList;
use crate::text::{DewState, emit_text_commands};
use crate::theme;
use crate::views;

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
pub(crate) struct DewDispatcher {
    handlers: Rc<HashMap<TypeId, DewHandlerFn>>,
}

impl DewDispatcher {
    pub(crate) fn register<V: View>(
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

    /// The retained scene being built for the current tree.
    pub(crate) const fn list_mut(&mut self) -> &mut DisplayList {
        &mut self.list
    }

    /// The shared text-shaping state, for measurement inside handlers.
    pub(crate) const fn state_cell(&self) -> &RefCell<DewState> {
        &self.state
    }

    /// Re-enters dispatch for a child view (a control label, scroll content,
    /// or any other nested subtree) at `ctx`.
    pub(crate) fn dispatch_child(&mut self, view: AnyView, env: &Environment, ctx: RenderContext) {
        let dispatcher = self.dispatcher.clone();
        dispatcher.dispatch_boxed(self, view, env, ctx);
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
            if type_id == TypeId::of::<Native<ScrollView>>() {
                let native = *view
                    .downcast::<Native<ScrollView>>()
                    .expect("dew normalize scroll downcast");
                let (axis, content) = native.into_inner().into_inner();
                let content = self.normalize(content, env);
                return AnyView::new(Native::new(ScrollView::new(axis, content)));
            }
            if type_id == TypeId::of::<Native<SliderConfig>>() {
                let mut config = view
                    .downcast::<Native<SliderConfig>>()
                    .expect("dew normalize slider downcast")
                    .into_inner();
                config.min_value_label = self.normalize(config.min_value_label, env);
                config.max_value_label = self.normalize(config.max_value_label, env);
                return AnyView::new(Native::new(config));
            }
            #[cfg(feature = "progress")]
            if type_id == TypeId::of::<Native<ProgressConfig>>() {
                let mut config = view
                    .downcast::<Native<ProgressConfig>>()
                    .expect("dew normalize progress downcast")
                    .into_inner();
                config.label = self.normalize(config.label, env);
                config.value_label = self.normalize(config.value_label, env);
                return AnyView::new(Native::new(config));
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
        let (width, height) = state
            .borrow_mut()
            .measure_styled(&styled, env, proposal.width);
        return ViewDimensions::new(Size::new(width, height));
    }
    if let Some(text) = view.downcast_ref::<Str>() {
        let (width, height) = state.borrow_mut().measure_plain(text, proposal.width);
        return ViewDimensions::new(Size::new(width, height));
    }
    if view.downcast_ref::<Native<Color>>().is_some()
        || view.downcast_ref::<Native<ResolvedColor>>().is_some()
    {
        return ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ));
    }
    if view.downcast_ref::<Native<Spacer>>().is_some()
        || view.downcast_ref::<Native<()>>().is_some()
    {
        return ViewDimensions::new(Size::new(0.0, 0.0));
    }
    if let Some(scroll) = view.downcast_ref::<Native<ScrollView>>() {
        return views::scroll::measure(state, scroll.as_inner(), env, proposal);
    }
    if view.downcast_ref::<Divider>().is_some() {
        return ViewDimensions::new(views::divider::measure());
    }
    if let Some(toggle) = view.downcast_ref::<Native<ToggleConfig>>() {
        return ViewDimensions::new(views::toggle::measure(state, toggle.as_inner(), env));
    }
    if let Some(slider) = view.downcast_ref::<Native<SliderConfig>>() {
        return ViewDimensions::new(views::slider::measure(state, slider.as_inner(), env));
    }
    if let Some(stepper) = view.downcast_ref::<Native<StepperConfig>>() {
        return ViewDimensions::new(views::stepper::measure(state, stepper.as_inner(), env));
    }
    if let Some(field) = view.downcast_ref::<Native<ResolvedTextFieldConfig>>() {
        return ViewDimensions::new(views::text_field::measure(state, field.as_inner(), env));
    }
    #[cfg(feature = "progress")]
    if let Some(progress) = view.downcast_ref::<Native<ProgressConfig>>() {
        return ViewDimensions::new(views::progress::measure(state, progress.as_inner(), env));
    }
    panic!("dew cannot measure un-normalized view; normalize the tree before measuring");
}

/// A measurable child handed to a [`waterui_core::layout::Layout`]
/// implementation.
struct DewSubview<'a> {
    // Measurement borrows the `!Send` dew render state and `Environment` and recurses
    // into child bodies, so the subview is confined to the main thread
    // (`require_main_thread() == true`); the `MainThreadBound` assertions catch a
    // worker-thread misschedule.
    view: MainThreadBound<&'a AnyView>,
    state: MainThreadBound<&'a RefCell<DewState>>,
    env: MainThreadBound<Environment>,
    stretch: StretchAxis,
}

impl<'a> DewSubview<'a> {
    fn new(view: &'a AnyView, state: &'a RefCell<DewState>, env: &Environment) -> Self {
        Self {
            view: MainThreadBound::new(view),
            state: MainThreadBound::new(state),
            env: MainThreadBound::new(env.clone()),
            stretch: effective_stretch_axis(view),
        }
    }
}

impl SubView for DewSubview<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure_view(*self.state, *self.view, &self.env, proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch
    }

    fn priority(&self) -> i32 {
        0
    }

    fn require_main_thread(&self) -> bool {
        true
    }
}

/// The stretch axis of a view, looking through environment-metadata
/// wrappers (which would otherwise report no stretch).
///
/// [`Divider`] is special-cased to stretch along its parent stack's cross
/// axis, mirroring hydrolysis: its `View` impl reports no stretch because
/// the axis is only known from the surrounding container.
fn effective_stretch_axis(view: &AnyView) -> StretchAxis {
    if view.downcast_ref::<Divider>().is_some() {
        return StretchAxis::CrossAxis;
    }
    view.downcast_ref::<Metadata<Environment>>().map_or_else(
        || view.stretch_axis(),
        |metadata| effective_stretch_axis(&metadata.content),
    )
}

fn register_core_handlers(dispatcher: &mut DewDispatcher) {
    dispatcher.register::<Native<FixedContainer>>(render_container);
    dispatcher.register::<Native<Color>>(render_color);
    dispatcher.register::<Native<ResolvedColor>>(render_resolved_color);
    dispatcher.register::<Native<Spacer>>(render_spacer);
    dispatcher.register::<Native<()>>(render_unit);
    dispatcher.register::<Native<TextConfig>>(render_text_config);
    dispatcher.register::<Str>(render_str);
    dispatcher.register::<Metadata<Environment>>(render_environment_metadata);
    views::register(dispatcher);
}

fn render_environment_metadata(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    metadata: Metadata<Environment>,
    _env: &Environment,
) {
    renderer.dispatch_child(metadata.content, &metadata.value, ctx);
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
    for (child, frame) in contents.into_iter().zip(frames) {
        renderer.dispatch_child(child, env, ctx.child(frame));
    }
}

fn render_color(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<Color>,
    env: &Environment,
) {
    let resolved = view.into_inner().resolve(env);
    let color = renderer.read_signal(&resolved);
    render_color_value(renderer, ctx, color);
}

fn render_resolved_color(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<ResolvedColor>,
    _env: &Environment,
) {
    render_color_value(renderer, ctx, view.into_inner());
}

fn render_color_value(renderer: &mut DewRenderer, ctx: RenderContext, color: ResolvedColor) {
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

fn render_unit(
    _renderer: &mut DewRenderer,
    _ctx: RenderContext,
    _view: Native<()>,
    _env: &Environment,
) {
}

fn render_text_config(
    renderer: &mut DewRenderer,
    ctx: RenderContext,
    view: Native<TextConfig>,
    env: &Environment,
) {
    let config = view.into_inner();
    let styled = renderer.read_signal(&config.content);
    let layout = renderer.state.borrow_mut().build_styled_layout(
        &styled,
        env,
        max_width_from_bounds(ctx.bounds),
        theme::FOREGROUND,
    );
    let transform = ctx.transform * Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
    emit_text_commands(&mut renderer.list, &layout, transform);
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "dispatcher handlers receive views by value per the handler contract"
)]
fn render_str(renderer: &mut DewRenderer, ctx: RenderContext, text: Str, _env: &Environment) {
    let layout = renderer
        .state
        .borrow_mut()
        .build_plain_layout(&text, max_width_from_bounds(ctx.bounds));
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
