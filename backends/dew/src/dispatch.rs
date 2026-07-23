//! Persistent `WaterUI` view dispatch for the Dew backend.
//!
//! Dew expands each view body exactly once into a retained [`DewNode`] tree.
//! Scalar signals request a refresh; a refresh re-reads those signals, runs
//! layout, and emits a new display list without evaluating any view body.
//! [`Dynamic`] is the sole structural seam and replaces only its own child node.

use core::any::TypeId;
use core::cell::RefCell;
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
use waterui_core::dynamic::Dynamic;
use waterui_core::layout::{
    ProposalSize, Rect as LayoutRect, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_core::{AnyView, Environment, MainThreadBound, Metadata, Native, Retain, Str, View};
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

    /// A child context placed at `frame` inside this context.
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

/// One persistent Dew render-tree node.
pub(crate) trait DewNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions;
    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext);

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn patch(&mut self, _renderer: &mut DewRenderer) -> bool {
        false
    }
}

/// Dew's retained renderer and signal subscriptions.
pub struct DewRenderer {
    signals: FrameSignals,
    state: RefCell<DewState>,
    list: DisplayList,
    watch_guards: Vec<Box<dyn core::any::Any>>,
    root: Option<Box<dyn DewNode>>,
}

impl core::fmt::Debug for DewRenderer {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DewRenderer")
            .field("list", &self.list)
            .field("watchers", &self.watch_guards.len())
            .field("has_root", &self.root.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for DewRenderer {
    fn default() -> Self {
        Self::new(FrameSignals::new(Instant::now()))
    }
}

impl DewRenderer {
    /// Creates a retained renderer wired to `signals`.
    #[must_use]
    pub fn new(signals: FrameSignals) -> Self {
        Self {
            signals,
            state: RefCell::new(DewState::default()),
            list: DisplayList::new(),
            watch_guards: Vec::new(),
            root: None,
        }
    }

    /// The frame-trigger handle shared with the runtime loop.
    #[must_use]
    pub fn signals(&self) -> FrameSignals {
        self.signals.clone()
    }

    /// Builds a fresh retained root and renders its first display list.
    ///
    /// Runtime code calls this exactly once. Explicit callers may use it to
    /// render unrelated one-shot trees in tests and tools.
    pub fn render_tree(
        &mut self,
        view: AnyView,
        env: &Environment,
        width: f64,
        height: f64,
    ) -> DisplayList {
        self.root = Some(build_node(self, view, env, 0));
        self.refresh_tree(width, height)
    }

    /// Re-layouts and re-renders the retained root without evaluating bodies.
    pub(crate) fn refresh_tree(&mut self, width: f64, height: f64) -> DisplayList {
        let mut root = self
            .root
            .take()
            .expect("Dew refresh requires an initialized retained root");
        self.list.clear();
        self.watch_guards.clear();
        root.patch(self);
        root.render(self, RenderContext::root(width, height));
        self.root = Some(root);
        core::mem::take(&mut self.list)
    }

    /// Reads a reactive input and schedules a retained-tree refresh on change.
    pub(crate) fn read_signal<S>(&mut self, signal: &S) -> S::Output
    where
        S: Signal + Clone + 'static,
    {
        let signals = self.signals.clone();
        let guard = signal.watch(move |_| signals.request_refresh());
        self.watch_guards.push(Box::new(guard));
        signal.get()
    }

    pub(crate) const fn list_mut(&mut self) -> &mut DisplayList {
        &mut self.list
    }

    pub(crate) const fn state_cell(&self) -> &RefCell<DewState> {
        &self.state
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "type-directed retained node construction is clearer as one exhaustive dispatch chain"
)]
pub(crate) fn build_node(
    renderer: &mut DewRenderer,
    mut view: AnyView,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    assert!(
        depth < MAX_BODY_DEPTH,
        "dew: view did not resolve within {MAX_BODY_DEPTH} body expansions"
    );

    let type_id = view.type_id();
    if type_id == TypeId::of::<Metadata<Environment>>() {
        let Metadata { content, value } = *view
            .downcast::<Metadata<Environment>>()
            .expect("dew environment metadata downcast must match its type id");
        return build_node(renderer, content, &value, depth + 1);
    }
    if type_id == TypeId::of::<Metadata<Retain>>() {
        let Metadata { content, value } = *view
            .downcast::<Metadata<Retain>>()
            .expect("dew retain metadata downcast must match its type id");
        return Box::new(RetainNode {
            _retain: value,
            child: build_node(renderer, content, env, depth + 1),
        });
    }
    if type_id == TypeId::of::<Native<FixedContainer>>() {
        let native = *view
            .downcast::<Native<FixedContainer>>()
            .expect("dew container downcast must match its type id");
        let (layout, contents) = native.into_inner().into_inner();
        let children = contents
            .into_iter()
            .map(|child| build_node(renderer, child, env, depth + 1))
            .collect();
        return Box::new(ContainerNode { layout, children });
    }
    if type_id == TypeId::of::<Dynamic>() {
        let dynamic = *view
            .downcast::<Dynamic>()
            .expect("dew Dynamic downcast must match its type id");
        return build_dynamic_node(renderer, dynamic, env, depth + 1);
    }
    if type_id == TypeId::of::<Native<ScrollView>>() {
        let scroll = *view
            .downcast::<Native<ScrollView>>()
            .expect("dew ScrollView downcast must match its type id");
        return views::scroll::build(renderer, scroll.into_inner(), env, depth + 1);
    }
    if type_id == TypeId::of::<Native<Color>>() {
        let color = *view
            .downcast::<Native<Color>>()
            .expect("dew Color downcast must match its type id");
        return Box::new(ColorNode {
            color: color.into_inner().resolve(env),
        });
    }
    if type_id == TypeId::of::<Native<ResolvedColor>>() {
        let color = *view
            .downcast::<Native<ResolvedColor>>()
            .expect("dew ResolvedColor downcast must match its type id");
        return Box::new(ResolvedColorNode(color.into_inner()));
    }
    if type_id == TypeId::of::<Native<TextConfig>>() {
        let text = *view
            .downcast::<Native<TextConfig>>()
            .expect("dew TextConfig downcast must match its type id");
        return Box::new(TextNode {
            config: text.into_inner(),
            env: env.clone(),
        });
    }
    if type_id == TypeId::of::<Str>() {
        return Box::new(StrNode {
            value: *view
                .downcast::<Str>()
                .expect("dew Str downcast must match its type id"),
        });
    }
    if type_id == TypeId::of::<Native<Spacer>>() {
        let spacer = *view
            .downcast::<Native<Spacer>>()
            .expect("dew Spacer downcast must match its type id");
        return Box::new(EmptyNode {
            stretch: spacer.stretch_axis(),
        });
    }
    if type_id == TypeId::of::<Native<()>>() {
        return Box::new(EmptyNode {
            stretch: StretchAxis::None,
        });
    }
    if type_id == TypeId::of::<Divider>() {
        return views::divider::build(env);
    }
    if type_id == TypeId::of::<Native<ToggleConfig>>() {
        let toggle = *view
            .downcast::<Native<ToggleConfig>>()
            .expect("dew ToggleConfig downcast must match its type id");
        return views::toggle::build(toggle.into_inner(), env);
    }
    if type_id == TypeId::of::<Native<SliderConfig>>() {
        let slider = *view
            .downcast::<Native<SliderConfig>>()
            .expect("dew SliderConfig downcast must match its type id");
        return views::slider::build(renderer, slider.into_inner(), env, depth + 1);
    }
    if type_id == TypeId::of::<Native<StepperConfig>>() {
        let stepper = *view
            .downcast::<Native<StepperConfig>>()
            .expect("dew StepperConfig downcast must match its type id");
        return views::stepper::build(stepper.into_inner(), env);
    }
    if type_id == TypeId::of::<Native<ResolvedTextFieldConfig>>() {
        let field = *view
            .downcast::<Native<ResolvedTextFieldConfig>>()
            .expect("dew text-field downcast must match its type id");
        return views::text_field::build(field.into_inner(), env);
    }
    #[cfg(feature = "progress")]
    if type_id == TypeId::of::<Native<ProgressConfig>>() {
        let progress = *view
            .downcast::<Native<ProgressConfig>>()
            .expect("dew ProgressConfig downcast must match its type id");
        return views::progress::build(renderer, progress.into_inner(), env, depth + 1);
    }

    view = AnyView::new(view.body(env));
    build_node(renderer, view, env, depth + 1)
}

struct NodeSubview<'a> {
    node: MainThreadBound<&'a dyn DewNode>,
    state: MainThreadBound<&'a RefCell<DewState>>,
}

impl<'a> NodeSubview<'a> {
    fn new(node: &'a dyn DewNode, state: &'a RefCell<DewState>) -> Self {
        Self {
            node: MainThreadBound::new(node),
            state: MainThreadBound::new(state),
        }
    }
}

impl SubView for NodeSubview<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.node.measure(*self.state, proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.node.stretch_axis()
    }

    fn priority(&self) -> i32 {
        0
    }

    fn require_main_thread(&self) -> bool {
        true
    }
}

struct ContainerNode {
    layout: Box<dyn waterui_core::layout::Layout>,
    children: Vec<Box<dyn DewNode>>,
}

impl DewNode for ContainerNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        let subviews = self
            .children
            .iter()
            .map(|child| NodeSubview::new(child.as_ref(), state))
            .collect::<Vec<_>>();
        let refs = subviews
            .iter()
            .map(|subview| subview as &dyn SubView)
            .collect::<Vec<_>>();
        ViewDimensions::new(self.layout.size_that_fits(proposal, &refs))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let frames = {
            let subviews = self
                .children
                .iter()
                .map(|child| NodeSubview::new(child.as_ref(), &renderer.state))
                .collect::<Vec<_>>();
            let refs = subviews
                .iter()
                .map(|subview| subview as &dyn SubView)
                .collect::<Vec<_>>();
            let proposal = proposal_from_bounds(ctx.bounds);
            let size = self.layout.size_that_fits(proposal, &refs);
            self.layout.place(LayoutRect::from_size(size), &refs)
        };
        for (child, frame) in self.children.iter_mut().zip(frames) {
            child.render(renderer, ctx.child(frame));
        }
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.children
            .iter_mut()
            .fold(false, |changed, child| child.patch(renderer) | changed)
    }
}

struct RetainNode {
    _retain: Retain,
    child: Box<dyn DewNode>,
}

impl DewNode for RetainNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        self.child.measure(state, proposal)
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        self.child.render(renderer, ctx);
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.child.stretch_axis()
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.child.patch(renderer)
    }
}

struct DynamicNode {
    _source: Dynamic,
    pending: Rc<RefCell<Option<AnyView>>>,
    child: Box<dyn DewNode>,
    env: Environment,
    depth: usize,
}

fn build_dynamic_node(
    renderer: &mut DewRenderer,
    dynamic: Dynamic,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    let initial = dynamic
        .with_unconnected_view_mut(Option::take)
        .expect("Dew cannot attach the same Dynamic instance more than once")
        .unwrap_or_else(|| AnyView::new(()));
    let pending = Rc::new(RefCell::new(None));
    let signals = renderer.signals();
    dynamic.clone().connect({
        let pending = Rc::clone(&pending);
        move |context| {
            pending.borrow_mut().replace(context.into_value());
            signals.request_refresh();
        }
    });
    Box::new(DynamicNode {
        _source: dynamic,
        pending,
        child: build_node(renderer, initial, env, depth),
        env: env.clone(),
        depth,
    })
}

impl DewNode for DynamicNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        self.child.measure(state, proposal)
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        self.child.render(renderer, ctx);
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.child.stretch_axis()
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        let changed = if let Some(view) = self.pending.borrow_mut().take() {
            self.child = build_node(renderer, view, &self.env, self.depth);
            true
        } else {
            false
        };
        self.child.patch(renderer) | changed
    }
}

struct ColorNode {
    color: nami::Computed<ResolvedColor>,
}

impl DewNode for ColorNode {
    fn measure(&self, _state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let color = renderer.read_signal(&self.color);
        render_color(renderer, ctx, color);
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

struct ResolvedColorNode(ResolvedColor);

impl DewNode for ResolvedColorNode {
    fn measure(&self, _state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        render_color(renderer, ctx, self.0);
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

fn render_color(renderer: &mut DewRenderer, ctx: RenderContext, color: ResolvedColor) {
    let srgb = color.to_srgb_with_headroom();
    renderer.list.fill(
        &ctx.bounds,
        ctx.transform,
        peniko::Color::new([srgb.red, srgb.green, srgb.blue, color.opacity]),
    );
}

struct TextNode {
    config: TextConfig,
    env: Environment,
}

impl DewNode for TextNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        let styled = self.config.content.get();
        let (width, height) = state
            .borrow_mut()
            .measure_styled(&styled, &self.env, proposal.width);
        ViewDimensions::new(Size::new(width, height))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let styled = renderer.read_signal(&self.config.content);
        let layout = renderer.state.borrow_mut().build_styled_layout(
            &styled,
            &self.env,
            max_width_from_bounds(ctx.bounds),
            theme::FOREGROUND,
        );
        let transform = ctx.transform * Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
        emit_text_commands(&mut renderer.list, &layout, transform);
    }
}

struct StrNode {
    value: Str,
}

impl DewNode for StrNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        let (width, height) = state
            .borrow_mut()
            .measure_plain(&self.value, proposal.width);
        ViewDimensions::new(Size::new(width, height))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let layout = renderer
            .state
            .borrow_mut()
            .build_plain_layout(&self.value, max_width_from_bounds(ctx.bounds));
        let transform = ctx.transform * Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
        emit_text_commands(&mut renderer.list, &layout, transform);
    }
}

struct EmptyNode {
    stretch: StretchAxis,
}

impl DewNode for EmptyNode {
    fn measure(&self, _state: &RefCell<DewState>, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::zero())
    }

    fn render(&mut self, _renderer: &mut DewRenderer, _ctx: RenderContext) {}

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "logical-pixel geometry is far below f32 precision limits"
)]
pub(crate) fn proposal_from_bounds(bounds: Rect) -> ProposalSize {
    ProposalSize::new(Some(bounds.width() as f32), Some(bounds.height() as f32))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "logical-pixel geometry is far below f32 precision limits"
)]
fn max_width_from_bounds(bounds: Rect) -> Option<f32> {
    (bounds.width() > 0.0).then(|| bounds.width() as f32)
}
