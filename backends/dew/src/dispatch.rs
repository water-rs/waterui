//! Persistent `WaterUI` view dispatch for the Dew backend.
//!
//! Dew expands each view body exactly once into a retained `DewNode` tree.
//! Scalar signals request a refresh; a refresh re-reads those signals, runs
//! layout, and emits a new display list without evaluating any view body.
//! [`Dynamic`] is the sole structural seam and replaces only its own child node.

use core::any::TypeId;
use core::cell::{Cell, RefCell};
use std::rc::Rc;

use kurbo::{Affine, Rect};
use nami::{Computed, Signal};
#[cfg(feature = "progress")]
use waterui::component::progress::ProgressConfig;
use waterui_backend_core::frame_signals::FrameSignals;
#[cfg(feature = "system-fonts")]
use waterui_backend_core::time::Instant;
use waterui_controls::button::ButtonConfig;
use waterui_controls::slider::SliderConfig;
use waterui_controls::stepper::StepperConfig;
use waterui_controls::text_field::ResolvedTextFieldConfig;
use waterui_controls::toggle::ToggleConfig;
use waterui_core::dynamic::Dynamic;
use waterui_core::layout::{
    ProposalSize, Rect as LayoutRect, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_core::views::Views;
use waterui_core::{AnyView, Environment, MainThreadBound, Metadata, Native, Retain, Str, View};
use waterui_graphics::color::{Color, ResolvedColor};
use waterui_layout::Divider;
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::scroll::ScrollView;
use waterui_layout::spacer::Spacer;
use waterui_text::{TextConfig, styled::StyledStr};

use crate::display_list::DisplayList;
use crate::pointer::{PointerRouter, PointerTargetHandle};
use crate::text::{DewState, TextLayoutCache, TextLayoutKey};
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

    /// Bounds transformed into window coordinates for hit testing.
    #[must_use]
    pub fn window_bounds(self) -> Rect {
        self.transform.transform_rect_bbox(self.bounds)
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

/// A node paired with a per-frame cache of its measurement results.
///
/// Container layouts negotiate by probing: `size_that_fits` asks every child
/// for a size, then `place` asks again, and each child that is itself a
/// container repeats the pattern with its own children. The number of measure
/// calls therefore multiplies with depth even though the answers are
/// identical within a frame — and for a text leaf each answer costs a full
/// `parley` layout.
///
/// Wrapping every node in this cache collapses that to one measurement per
/// distinct [`ProposalSize`] per node per frame. The cache is cleared in
/// [`DewNode::patch`], which the runtime runs over the whole tree immediately
/// before rendering, so a cached size can never outlive the signal values it
/// came from. The per-frame lifetime is the point: a longer-lived cache would
/// have to track every signal each measurement happened to read.
///
/// This is also where `AGENTS.md` places the cache — measurement caching is
/// the `SubView`'s responsibility, never the `Layout`'s. Dew confines
/// measurement to the render thread ([`NodeSubview`] returns `true` from
/// `require_main_thread`), so a plain [`RefCell`] suffices and no
/// synchronization is needed.
struct MeasuredNode {
    inner: Box<dyn DewNode>,
    cache: RefCell<Vec<(ProposalSize, ViewDimensions)>>,
}

impl MeasuredNode {
    fn wrap(inner: Box<dyn DewNode>) -> Box<dyn DewNode> {
        Box::new(Self {
            inner,
            cache: RefCell::new(Vec::new()),
        })
    }
}

impl DewNode for MeasuredNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        let cached = self
            .cache
            .borrow()
            .iter()
            .find(|(key, _)| *key == proposal)
            .map(|(_, dimensions)| dimensions.clone());
        if let Some(dimensions) = cached {
            state.borrow_mut().work.measures_reused += 1;
            return dimensions;
        }
        let dimensions = self.inner.measure(state, proposal);
        state.borrow_mut().work.measures_computed += 1;
        self.cache.borrow_mut().push((proposal, dimensions.clone()));
        dimensions
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        self.inner.render(renderer, ctx);
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.inner.stretch_axis()
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.cache.borrow_mut().clear();
        self.inner.patch(renderer)
    }
}

/// Dew's retained renderer and signal subscriptions.
pub struct DewRenderer {
    signals: FrameSignals,
    state: RefCell<DewState>,
    list: DisplayList,
    pointer: PointerRouter,
    root: Option<Box<dyn DewNode>>,
    theme: Option<theme::ThemePalette>,
}

impl core::fmt::Debug for DewRenderer {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DewRenderer")
            .field("list", &self.list)
            .field("pointer", &self.pointer)
            .field("has_root", &self.root.is_some())
            .finish_non_exhaustive()
    }
}

/// Desktop-only default: system fonts and a wall-clock frame origin.
#[cfg(feature = "system-fonts")]
impl Default for DewRenderer {
    fn default() -> Self {
        Self::new(
            FrameSignals::new(Instant::now()),
            crate::board::FontSources::System,
        )
    }
}

impl DewRenderer {
    /// Creates a retained renderer wired to `signals`, shaping text with
    /// `fonts`.
    #[must_use]
    pub fn new(signals: FrameSignals, fonts: crate::board::FontSources) -> Self {
        Self {
            signals,
            state: RefCell::new(DewState::new(fonts)),
            list: DisplayList::new(),
            pointer: PointerRouter::default(),
            root: None,
            theme: None,
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
        self.theme = Some(theme::ThemePalette::new(env, self.signals()));
        self.root = Some(build_node(self, view, env, 0));
        self.refresh_tree(width, height)
    }

    pub(crate) const fn theme(&self) -> &theme::ThemePalette {
        self.theme
            .as_ref()
            .expect("Dew theme palette requires an initialized retained root")
    }

    /// Re-layouts and re-renders the retained root without evaluating bodies.
    pub(crate) fn refresh_tree(&mut self, width: f64, height: f64) -> DisplayList {
        let mut root = self
            .root
            .take()
            .expect("Dew refresh requires an initialized retained root");
        self.list.clear();
        self.pointer.begin_frame();
        // Discard measure counters left over from tree construction so the
        // frame reports only its own work.
        self.state.borrow_mut().take_work();
        root.patch(self);
        root.render(self, RenderContext::root(width, height));
        self.pointer.finish_frame();
        self.root = Some(root);
        let measure_work = self.state.borrow_mut().take_work();
        self.list.add_work(measure_work);
        core::mem::take(&mut self.list)
    }

    pub(crate) const fn list_mut(&mut self) -> &mut DisplayList {
        &mut self.list
    }

    /// The display list and shaping state, split-borrowed so a caller can
    /// emit into the list while a build closure reaches the state.
    pub(crate) const fn list_and_state(&mut self) -> (&mut DisplayList, &RefCell<DewState>) {
        (&mut self.list, &self.state)
    }

    pub(crate) const fn state_cell(&self) -> &RefCell<DewState> {
        &self.state
    }

    pub(crate) fn register_pointer_target(&mut self, bounds: Rect, handler: PointerTargetHandle) {
        self.pointer.register(bounds, handler);
    }

    pub(crate) fn handle_pointer(
        &mut self,
        sample: crate::board::PointerSample,
        env: &Environment,
    ) -> bool {
        self.pointer.dispatch(sample, env)
    }
}

/// Expands `view` into a retained node, wrapped in its measurement cache.
///
/// Every node in the tree is wrapped, including the ones widget handlers
/// build, so the layout pass never measures the same node twice with the same
/// proposal within a frame. See [`MeasuredNode`].
pub(crate) fn build_node(
    renderer: &mut DewRenderer,
    view: AnyView,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    MeasuredNode::wrap(build_unmeasured_node(renderer, view, env, depth))
}

#[expect(
    clippy::too_many_lines,
    reason = "type-directed retained node construction is clearer as one exhaustive dispatch chain"
)]
fn build_unmeasured_node(
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
        // Environment metadata is a pass-through, not a node of its own: build
        // the content unwrapped so it does not collect a second, redundant
        // measurement cache underneath the one this call is already wrapped in.
        return build_unmeasured_node(renderer, content, &value, depth + 1);
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
    if type_id == TypeId::of::<Native<LazyContainer>>() {
        let native = *view
            .downcast::<Native<LazyContainer>>()
            .expect("dew lazy container downcast must match its type id");
        let (layout, contents) = native.into_inner().into_inner();
        // Dew materializes the whole collection instead of virtualizing it. Its
        // screens hold a handful of widgets — the budgeted simulation is a
        // twelve-product panel — so a visible-window index would cost more
        // bookkeeping and allocation than the rows it avoids building, and this
        // backend is budgeted on exactly that. Rendering nothing, which is what
        // an unhandled `LazyContainer` did before, is not the cheaper option.
        let count = contents.len().get();
        let children = (0..count)
            .map(|index| {
                let child = contents.get_view(index).unwrap_or_else(|| {
                    panic!("dew LazyContainer failed to materialize child at index {index}")
                });
                build_node(renderer, child, env, depth + 1)
            })
            .collect();
        return Box::new(ContainerNode { layout, children });
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
            color: WatchedSignal::new(color.into_inner().resolve(env), renderer.signals()),
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
        let config = text.into_inner();
        return Box::new(TextNode {
            content: WatchedSignal::new(config.content, renderer.signals()),
            env: env.clone(),
            cache: RefCell::new(TextLayoutCache::default()),
            line_limit: config.line_limit.map(core::num::NonZeroUsize::get),
        });
    }
    if type_id == TypeId::of::<Str>() {
        return Box::new(StrNode {
            value: *view
                .downcast::<Str>()
                .expect("dew Str downcast must match its type id"),
            cache: RefCell::new(TextLayoutCache::default()),
            env: env.clone(),
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
    if type_id == TypeId::of::<Native<ButtonConfig>>() {
        let button = *view
            .downcast::<Native<ButtonConfig>>()
            .expect("dew ButtonConfig downcast must match its type id");
        return views::button::build(renderer, button.into_inner(), env);
    }
    if type_id == TypeId::of::<Native<ToggleConfig>>() {
        let toggle = *view
            .downcast::<Native<ToggleConfig>>()
            .expect("dew ToggleConfig downcast must match its type id");
        return views::toggle::build(renderer, toggle.into_inner(), env);
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
        return views::stepper::build(renderer, stepper.into_inner(), env);
    }
    if type_id == TypeId::of::<Native<ResolvedTextFieldConfig>>() {
        let field = *view
            .downcast::<Native<ResolvedTextFieldConfig>>()
            .expect("dew text-field downcast must match its type id");
        return views::text_field::build(renderer, field.into_inner(), env);
    }
    #[cfg(feature = "progress")]
    if type_id == TypeId::of::<Native<ProgressConfig>>() {
        let progress = *view
            .downcast::<Native<ProgressConfig>>()
            .expect("dew ProgressConfig downcast must match its type id");
        return views::progress::build(renderer, progress.into_inner(), env, depth + 1);
    }

    // Expanding a composite body yields the same logical node, so it stays
    // unwrapped; the caller's wrapper caches the whole expansion.
    view = AnyView::new(view.body(env));
    build_unmeasured_node(renderer, view, env, depth + 1)
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
            // Placed at the size the parent assigned, never at this layout's own
            // measurement of itself: a stack is content-sized on its cross axis
            // (`HStack::stretch_axis` is `None`, like SwiftUI's), so re-measuring
            // here would shrink a row of cross-stretching children — a row of
            // colours, say — to zero height and draw nothing. Distributing the
            // assigned box among the children is exactly what `place` is for.
            self.layout.place(placement_rect(ctx.bounds), &refs)
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
    color: WatchedSignal<Computed<ResolvedColor>>,
}

impl DewNode for ColorNode {
    fn measure(&self, _state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        render_color(renderer, ctx, self.color.get());
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
    content: WatchedSignal<Computed<StyledStr>>,
    env: Environment,
    cache: RefCell<TextLayoutCache>,
    /// Maximum laid-out lines, from `TextConfig::line_limit`.
    line_limit: Option<usize>,
}

impl DewNode for TextNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        let foreground = theme::foreground(&self.env);
        let revision = self.content.revision();
        let mut cache = self.cache.borrow_mut();
        let ((width, height), outcome) = cache.measure(
            revision,
            TextLayoutKey::new(proposal.width, foreground),
            self.line_limit,
            || {
                state.borrow_mut().build_styled_layout(
                    &self.content.get(),
                    &self.env,
                    proposal.width,
                    foreground,
                )
            },
        );
        let dimensions = ViewDimensions::new(Size::new(width, height));
        state.borrow_mut().record_layout(outcome);
        dimensions
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let foreground = renderer.theme().foreground();
        let revision = self.content.revision();
        let max_width = max_width_from_bounds(ctx.bounds);
        let transform = ctx.transform * Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
        let outcome = self.cache.borrow_mut().emit(
            revision,
            TextLayoutKey::new(max_width, foreground),
            self.line_limit,
            transform,
            &mut renderer.list,
            || {
                renderer.state.borrow_mut().build_styled_layout(
                    &self.content.get(),
                    &self.env,
                    max_width,
                    foreground,
                )
            },
        );
        renderer.state.borrow_mut().record_layout(outcome);
    }
}

struct StrNode {
    value: Str,
    cache: RefCell<TextLayoutCache>,
    env: Environment,
}

impl DewNode for StrNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        let foreground = theme::foreground(&self.env);
        let mut cache = self.cache.borrow_mut();
        let ((width, height), outcome) = cache.measure(
            0,
            TextLayoutKey::new(proposal.width, foreground),
            None,
            || {
                state
                    .borrow_mut()
                    .build_plain_layout(&self.value, proposal.width, foreground)
            },
        );
        let dimensions = ViewDimensions::new(Size::new(width, height));
        state.borrow_mut().record_layout(outcome);
        dimensions
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let foreground = renderer.theme().foreground();
        let max_width = max_width_from_bounds(ctx.bounds);
        let transform = ctx.transform * Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
        let outcome = self.cache.borrow_mut().emit(
            0,
            TextLayoutKey::new(max_width, foreground),
            None,
            transform,
            &mut renderer.list,
            || {
                renderer
                    .state
                    .borrow_mut()
                    .build_plain_layout(&self.value, max_width, foreground)
            },
        );
        renderer.state.borrow_mut().record_layout(outcome);
    }
}

/// A signal paired with a counter that increments whenever it changes.
///
/// The counter is what layout and glyph caches key on: it changes exactly when
/// the shaped result would, and comparing it costs one load.
pub(crate) struct WatchedSignal<S: Signal> {
    signal: S,
    revision: Rc<Cell<u64>>,
    _guard: S::Guard,
}

impl<S: Signal> WatchedSignal<S> {
    pub(crate) fn new(signal: S, signals: FrameSignals) -> Self {
        let revision = Rc::new(Cell::new(0u64));
        let guard = signal.watch({
            let revision = Rc::clone(&revision);
            move |_| {
                revision.set(
                    revision
                        .get()
                        .checked_add(1)
                        .expect("Dew signal revision overflow"),
                );
                signals.request_refresh();
            }
        });
        Self {
            signal,
            revision,
            _guard: guard,
        }
    }

    pub(crate) fn get(&self) -> S::Output {
        self.signal.get()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision.get()
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

/// The box a container distributes among its children: the one its parent
/// placed it in.
#[expect(
    clippy::cast_possible_truncation,
    reason = "logical-pixel geometry is far below f32 precision limits"
)]
fn placement_rect(bounds: Rect) -> LayoutRect {
    LayoutRect::from_size(Size::new(bounds.width() as f32, bounds.height() as f32))
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
