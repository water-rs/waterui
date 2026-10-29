use super::{test_environment, test_renderer};
use crate::renderer::{HydrolysisRenderer, tree::RenderNode};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use waterui::{AnyView, Color, Environment, View, ViewExt};
use waterui_graphics::FilteredView;
use waterui_graphics::filtrate::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRenderResult, EffectSetupResult,
    RetainedMemory,
};
use waterui_layout::container::FixedContainer;
use waterui_layout::stack::vstack;
use waterui_layout::{
    Layout, Point, ProposalSize, Rect, Size, Spacer, SubView, SubviewPlacement, scroll,
    scroll_horizontal, spacer, spacer_min,
};

#[derive(Debug)]
struct Observation {
    proposal: ProposalSize,
    frames: Vec<Rect>,
}

type Trace = Rc<RefCell<Vec<Observation>>>;

#[derive(Debug)]
struct ProbeLayout {
    vertical: bool,
    trace: Trace,
}

impl Layout for ProbeLayout {
    fn size_that_fits(&self, _proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        axis_size(self.vertical, 160.0, 20.0)
    }

    fn place(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        assert_eq!(children.len(), 2);
        let main = if self.vertical {
            proposal.height
        } else {
            proposal.width
        };
        let extents = if main.is_none() {
            [40.0, 120.0]
        } else {
            [80.0, 80.0]
        };
        let mut cursor = 0.0;
        let placements: Vec<_> = extents
            .into_iter()
            .map(|extent| {
                let origin = if self.vertical {
                    Point::new(bounds.x(), bounds.y() + cursor)
                } else {
                    Point::new(bounds.x() + cursor, bounds.y())
                };
                cursor += extent;
                let size = axis_size(self.vertical, extent, 20.0);
                SubviewPlacement::new(
                    Rect::new(origin, size),
                    ProposalSize::new(Some(size.width), Some(size.height)),
                )
            })
            .collect();
        self.trace.borrow_mut().push(Observation {
            proposal,
            frames: placements.iter().map(|placement| placement.frame).collect(),
        });
        placements
    }
}

struct ProbeContent {
    vertical: bool,
    trace: Trace,
    builds: Rc<Cell<usize>>,
}

impl View for ProbeContent {
    fn body(self, _env: &Environment) -> impl View {
        self.builds.set(self.builds.get() + 1);
        FixedContainer::new(
            ProbeLayout {
                vertical: self.vertical,
                trace: self.trace,
            },
            (Color::srgb_hex("#2563EB"), Color::srgb_hex("#DC2626")),
        )
    }
}

struct Fixture {
    node: RenderNode<crate::text::ParleyEngine>,
    trace: Trace,
    builds: Rc<Cell<usize>>,
}

impl Fixture {
    fn new<V: View>(
        vertical: bool,
        wrap: impl FnOnce(ProbeContent) -> V,
        renderer: &mut HydrolysisRenderer<crate::text::ParleyEngine>,
        env: &Environment,
    ) -> Self {
        let trace = Rc::new(RefCell::new(Vec::new()));
        let builds = Rc::new(Cell::new(0));
        let content = ProbeContent {
            vertical,
            trace: Rc::clone(&trace),
            builds: Rc::clone(&builds),
        };
        Self {
            node: RenderNode::build(AnyView::new(wrap(content)), env, renderer),
            trace,
            builds,
        }
    }

    fn assert_placement(&self, vertical: bool, proposal: ProposalSize, extents: [f32; 2]) {
        let trace = self.trace.borrow();
        let observation = trace.last().expect("the retained subtree must be placed");
        assert_eq!(observation.proposal, proposal);
        assert_eq!(
            observation
                .frames
                .iter()
                .map(|frame| *frame.size())
                .collect::<Vec<_>>(),
            extents
                .map(|extent| axis_size(vertical, extent, 20.0))
                .to_vec(),
        );
        assert_eq!(self.builds.get(), 1);
    }
}

const fn axis_size(vertical: bool, main: f32, cross: f32) -> Size {
    if vertical {
        Size::new(cross, main)
    } else {
        Size::new(main, cross)
    }
}

fn axis_proposal(vertical: bool, main: Option<f32>, cross: Option<f32>) -> ProposalSize {
    if vertical {
        ProposalSize::new(cross, main)
    } else {
        ProposalSize::new(main, cross)
    }
}

#[test]
fn equal_bounds_keep_the_selected_proposal_after_other_probes() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let theme = renderer.theme();
    for vertical in [false, true] {
        let mut fixture = Fixture::new(vertical, |content| content, &mut renderer, &env);
        for main in [None, Some(160.0), None] {
            for probe in [Some(0.0), Some(80.0), Some(f32::INFINITY)] {
                let dimensions = fixture.node.measure(
                    &mut renderer.state,
                    &env,
                    &theme,
                    axis_proposal(vertical, probe, Some(20.0)),
                );
                assert_eq!(dimensions.size, axis_size(vertical, 160.0, 20.0));
            }
            fixture.trace.borrow_mut().clear();
            let proposal = axis_proposal(vertical, main, Some(20.0));
            fixture.node.layout(
                &mut renderer,
                &env,
                None,
                proposal,
                axis_size(vertical, 160.0, 20.0),
            );
            fixture.assert_placement(
                vertical,
                proposal,
                if main.is_none() {
                    [40.0, 120.0]
                } else {
                    [80.0, 80.0]
                },
            );
        }
    }
}

/// Runs `f` inside the root's record: a sub-view flush always runs under a
/// recording reader, which owns what it registers and anchors its placement.
fn under_root_record<R, E: crate::text::TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    f: impl FnOnce(&mut HydrolysisRenderer<E>) -> R,
) -> R {
    let root = renderer.core.root_core.clone();
    renderer
        .program
        .push(crate::renderer::mount::ProgramBuilder::new(Rc::clone(
            &root.cell,
        )));
    let result = renderer.with_reader(&root, crate::renderer::ReaderPhase::Record, f);
    let _ = renderer
        .program
        .pop()
        .expect("the root program under_root_record opened")
        .finish();
    result
}

#[test]
fn retained_scene_capture_preserves_proposal_and_viewport_boundaries() {
    use crate::renderer::{LazyViewport, RenderContext, tree::RetainedSubview};
    use kurbo::{Affine, Rect as SceneRect};

    let env = test_environment();
    let mut renderer = test_renderer();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let mut retained = RetainedSubview::new(AnyView::new(ProbeContent {
        vertical: false,
        trace: trace.clone(),
        builds: Rc::new(Cell::new(0)),
    }));
    let size = Size::new(160.0, 20.0);
    let rect = SceneRect::new(0.0, 0.0, 160.0, 20.0);
    let ctx = RenderContext::new(Affine::IDENTITY, rect);
    let ideal = ProposalSize::new(None, Some(20.0));
    under_root_record(&mut renderer, |renderer| {
        retained.place(renderer, ctx, &env, ideal, rect, None);
    });
    let outer = LazyViewport {
        bounds: SceneRect::new(0.0, 800.0, 160.0, 820.0),
        transform: Affine::translate((0.0, -800.0)),
    };
    renderer.push_lazy_viewport(outer);
    under_root_record(&mut renderer, |renderer| {
        retained.record_built_page(renderer, &env, size, None);
    });
    assert_eq!(renderer.lazy.lazy_viewport_stack.len(), 1);
    assert_eq!(renderer.lazy.lazy_viewport_stack[0].bounds, outer.bounds);
    trace.borrow_mut().clear();
    under_root_record(&mut renderer, |renderer| {
        retained.place(renderer, ctx, &env, ideal, rect, None);
    });
    assert_eq!(
        trace.borrow().last().expect("offer changed").proposal,
        ideal
    );
    renderer.pop_lazy_viewport("test outer viewport");
}

/// A `FixedContainer` inside a `RetainedSubview` must re-run `place` when one of
/// its layout signals invalidates — even when the slot's rect and proposal are
/// unchanged — or the constraint latched at mount is the only one it ever sees
/// (water-rs/waterui#1214).
#[test]
fn retained_subview_relayouts_when_a_layout_signal_invalidates() {
    use crate::renderer::{RenderContext, tree::RetainedSubview};
    use kurbo::{Affine, Rect as SceneRect};
    use waterui_layout::frame::Frame;

    let env = test_environment();
    let mut renderer = test_renderer();
    let max_width = nami::Binding::container(120.0f32);
    let constraint = max_width.clone();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let mut retained = RetainedSubview::new(AnyView::new(
        Frame::new(ProbeContent {
            vertical: false,
            trace: Rc::clone(&trace),
            builds: Rc::new(Cell::new(0)),
        })
        .max_width(constraint),
    ));
    let rect = SceneRect::new(0.0, 0.0, 800.0, 600.0);
    let ctx = RenderContext::new(Affine::IDENTITY, rect);
    let proposal = ProposalSize::new(Some(800.0), Some(600.0));
    under_root_record(&mut renderer, |renderer| {
        retained.place(renderer, ctx, &env, proposal, rect, None);
    });
    assert_eq!(
        trace.borrow().last().expect("mount places").proposal.width,
        Some(120.0)
    );
    max_width.set(400.0);
    assert!(
        renderer.root_is_dirty(),
        "the constraint signal must still schedule a refresh"
    );
    under_root_record(&mut renderer, |renderer| {
        retained.place(renderer, ctx, &env, proposal, rect, None);
    });
    assert_eq!(
        trace
            .borrow()
            .last()
            .expect("a layout-signal change must re-place")
            .proposal
            .width,
        Some(400.0),
        "max_width 120 → 400 must re-place the retained subtree at the same rect"
    );
}

/// A `Widget` leaf measures from the `RetainedSubview`s it owns — a toggle's
/// label sub-view is not part of the retained node's own tree, so a layout
/// signal firing *inside* that label marks the label's cells only. The leaf's
/// `take_layout_dirty` arm must fold the mark out or the outer sub-view keeps
/// the toggle at its first measured box (water-rs/waterui#1957).
///
/// Both styles run on every platform: `Automatic` resolves per target, and the
/// two place the label differently — a checkbox at its fitted width, a switch
/// across the row up to the control.
#[test]
fn retained_subview_relayouts_when_a_widget_leaf_label_invalidates() {
    use crate::renderer::{RenderContext, tree::RetainedSubview};
    use kurbo::{Affine, Rect as SceneRect};
    use waterui_controls::label::Label;
    use waterui_controls::toggle::{ToggleStyle, toggle};
    use waterui_layout::frame::Frame;

    // The placed width once the cap lifts: the probe's own 160 for a checkbox,
    // the 400 cap for a switch. A stale first box would still report 120.
    for (style, relaid) in [(ToggleStyle::Checkbox, 160.0), (ToggleStyle::Switch, 400.0)] {
        let env = test_environment();
        let mut renderer = test_renderer();
        let max_width = nami::Binding::container(120.0f32);
        let constraint = max_width.clone();
        let on = nami::Binding::container(false);
        let trace = Rc::new(RefCell::new(Vec::new()));
        let probe_trace = Rc::clone(&trace);
        // The toggle always wraps its label in a `RetainedSubview`; a
        // custom-content label keeps that sub-view in charge of the probe's
        // layout.
        let mut retained = RetainedSubview::new(AnyView::new(
            toggle(
                Label::new("flag", move || {
                    Frame::new(ProbeContent {
                        vertical: false,
                        trace: Rc::clone(&probe_trace),
                        builds: Rc::new(Cell::new(0)),
                    })
                    .max_width(constraint.clone())
                }),
                &on,
            )
            .style(style),
        ));
        let rect = SceneRect::new(0.0, 0.0, 800.0, 600.0);
        let ctx = RenderContext::new(Affine::IDENTITY, rect);
        let proposal = ProposalSize::new(Some(800.0), Some(600.0));
        under_root_record(&mut renderer, |renderer| {
            retained.place(renderer, ctx, &env, proposal, rect, None);
        });
        assert_eq!(
            trace.borrow().last().expect("mount places").proposal.width,
            Some(120.0),
            "{style:?}"
        );
        max_width.set(400.0);
        assert!(
            retained.take_layout_dirty(),
            "{style:?}: the label sub-view's layout mark must reach the enclosing \
             retained sub-view through the widget leaf"
        );
        under_root_record(&mut renderer, |renderer| {
            retained.place(renderer, ctx, &env, proposal, rect, None);
        });
        assert_eq!(
            trace
                .borrow()
                .last()
                .expect("the widget's label mark must re-place the retained subtree")
                .proposal
                .width,
            Some(relaid),
            "{style:?}: max_width 120 → 400 must re-measure the label"
        );
    }
}

/// Scene content whose natural width is driven by a cell — the drift a
/// retained sub-view must notice (water-rs/waterui#1957).
struct Drifting {
    natural: Rc<Cell<f32>>,
}

impl waterui_graphics::SceneContent for Drifting {
    fn build_scene(
        &mut self,
        _recorder: &mut waterui_graphics::draw::Recorder,
        _resources: &mut waterui_graphics::RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> bool {
        false
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(Size::new(self.natural.get(), 50.0))
    }

    fn rebuild_for_engine(&mut self) {}
}

/// A two-child row that places each child at the size it measures under an
/// open proposal and records every placement — the scene's own frame is the
/// observable a stale box would freeze.
#[derive(Debug)]
struct RecordingRow {
    trace: Trace,
}

impl Layout for RecordingRow {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let sizes = children
            .iter()
            .map(|child| child.measure(proposal).size)
            .collect::<Vec<_>>();
        Size::new(
            sizes.iter().map(|size| size.width).sum(),
            sizes.iter().map(|size| size.height).fold(0.0, f32::max),
        )
    }

    fn place(
        &self,
        bounds: Rect,
        _proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        let open = ProposalSize::UNSPECIFIED;
        let mut cursor = bounds.x();
        let placements: Vec<_> = children
            .iter()
            .map(|child| {
                let size = child.measure(open).size;
                let placement = SubviewPlacement::new(
                    Rect::new(Point::new(cursor, bounds.y()), size),
                    ProposalSize::new(Some(size.width), Some(size.height)),
                );
                cursor += size.width;
                placement
            })
            .collect();
        self.trace.borrow_mut().push(Observation {
            proposal: open,
            frames: placements.iter().map(|placement| placement.frame).collect(),
        });
        placements
    }
}

/// `[scene | color]` — the row's first frame is the scene's laid-out box.
struct DriftingRow {
    scene: waterui_graphics::SceneView,
    trace: Trace,
}

impl View for DriftingRow {
    fn body(self, _env: &Environment) -> impl View {
        FixedContainer::new(
            RecordingRow { trace: self.trace },
            (self.scene, Color::srgb_hex("#2563EB")),
        )
    }
}

/// A `SceneView` leaf's measured box comes from `intrinsic_size()` — a value,
/// not a consumable mark — so a natural-size change after mount (a reloaded
/// image's viewBox) leaves no flag behind. The leaf's `take_layout_dirty` arm
/// must re-probe the last proposal and report the drift, or the enclosing
/// retained sub-view keeps the scene at its first box forever
/// (water-rs/waterui#1957).
#[test]
fn retained_subview_relayouts_when_a_scene_leaves_natural_size_moves() {
    use crate::renderer::{RenderContext, tree::RetainedSubview};
    use kurbo::{Affine, Rect as SceneRect};
    use waterui_graphics::SceneView;

    let env = test_environment();
    let mut renderer = test_renderer();
    let natural = Rc::new(Cell::new(100.0f32));
    let trace = Rc::new(RefCell::new(Vec::new()));
    let mut retained = RetainedSubview::new(AnyView::new(DriftingRow {
        scene: SceneView::new(Drifting {
            natural: Rc::clone(&natural),
        }),
        trace: Rc::clone(&trace),
    }));
    let rect = SceneRect::new(0.0, 0.0, 800.0, 600.0);
    let ctx = RenderContext::new(Affine::IDENTITY, rect);
    let proposal = ProposalSize::new(Some(800.0), Some(600.0));
    under_root_record(&mut renderer, |renderer| {
        retained.place(renderer, ctx, &env, proposal, rect, None);
    });
    let scene_width = || {
        trace
            .borrow()
            .last()
            .expect("the retained subtree must be placed")
            .frames[0]
            .size()
            .width
    };
    assert_eq!(
        scene_width(),
        100.0,
        "the scene mounts at its natural width"
    );
    natural.set(200.0);
    assert!(
        retained.take_layout_dirty(),
        "a changed natural size must surface as layout drift through the leaf"
    );
    under_root_record(&mut renderer, |renderer| {
        retained.place(renderer, ctx, &env, proposal, rect, None);
    });
    assert_eq!(
        scene_width(),
        200.0,
        "the scene's new natural width must re-place the row at the same rect"
    );
}

#[test]
fn scroll_preserves_its_unconstrained_content_axis() {
    let env = test_environment();
    let mut renderer = test_renderer();
    for vertical in [false, true] {
        let mut fixture = Fixture::new(
            vertical,
            |content| {
                if vertical {
                    scroll(content)
                } else {
                    scroll_horizontal(content)
                }
            },
            &mut renderer,
            &env,
        );
        let size = axis_size(vertical, 160.0, 40.0);
        fixture.node.layout(
            &mut renderer,
            &env,
            None,
            ProposalSize::new(Some(size.width), Some(size.height)),
            size,
        );
        fixture.assert_placement(
            vertical,
            axis_proposal(vertical, None, Some(40.0)),
            [40.0, 120.0],
        );
    }
}

#[test]
fn transparent_metadata_preserves_the_selected_proposal() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let mut fixture = Fixture::new(
        false,
        |content| content.opacity(0.5).layout_priority(7),
        &mut renderer,
        &env,
    );
    let proposal = axis_proposal(false, None, Some(20.0));
    fixture.node.layout(
        &mut renderer,
        &env,
        None,
        proposal,
        axis_size(false, 160.0, 20.0),
    );
    fixture.assert_placement(false, proposal, [40.0, 120.0]);
}

#[derive(Debug)]
struct PriorityProbe(Rc<Cell<Option<i32>>>);

impl Layout for PriorityProbe {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        children[0].measure(proposal).size
    }

    fn place(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        self.0.set(Some(children[0].priority()));
        vec![SubviewPlacement::new(
            Rect::new(
                Point::new(bounds.x(), bounds.y()),
                children[0].measure(proposal).size,
            ),
            proposal,
        )]
    }
}

#[test]
fn spacer_default_priority_survives_wrappers_and_explicit_overrides() {
    let env = test_environment();
    let mut renderer = test_renderer();
    for explicit in [None, Some(0), Some(7)] {
        let priority = Rc::new(Cell::new(None));
        let gap = spacer().opacity(0.5);
        let content = match explicit {
            Some(value) => AnyView::new(gap.layout_priority(value)),
            None => AnyView::new(gap),
        };
        let mut node = RenderNode::build(
            AnyView::new(FixedContainer::new(
                PriorityProbe(Rc::clone(&priority)),
                (content,),
            )),
            &env,
            &mut renderer,
        );
        node.layout(
            &mut renderer,
            &env,
            None,
            ProposalSize::new(Some(160.0), Some(20.0)),
            Size::new(160.0, 20.0),
        );
        assert_eq!(
            priority.get(),
            Some(explicit.unwrap_or(Spacer::DEFAULT_LAYOUT_PRIORITY))
        );
    }
}

/// water-rs/hydrolysis#53: a `Native<Spacer>` must answer its `min_length`
/// on the enclosing stack's main axis — the stack keeps that answer as the
/// flexible child's floor under compression, so a zero answer collapses
/// `Spacer::new(40.0)` exactly like `Spacer::new(0.0)` (the
/// water-rs/waterui#1080 symptom).
#[test]
fn spacer_min_length_is_the_stack_compression_floor() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let theme = renderer.theme();

    let mut node = RenderNode::build(
        AnyView::new(vstack((
            Color::srgb_hex("#2563EB"),
            spacer_min(40.0),
            Color::srgb_hex("#DC2626"),
        ))),
        &env,
        &mut renderer,
    );
    let container = node
        .transparent_container()
        .expect("a vstack must build a container node");
    assert_eq!(container.children.len(), 3, "color, spacer, color");

    // Compress a 20x50 offer: the spacer's 40pt floor stands and the two
    // colors split what is left over.
    let proposal = ProposalSize::new(Some(20.0), Some(50.0));
    let measured = node.measure(&mut renderer.state, &env, &theme, proposal);
    node.layout(&mut renderer, &env, None, proposal, measured.size);
    let heights: Vec<f32> = node
        .transparent_container()
        .expect("a vstack must build a container node")
        .placed
        .iter()
        .map(Rect::height)
        .collect();
    assert!(
        approx::relative_eq!(heights[1], 40.0),
        "the spacer keeps its min_length floor under compression: left {:?}, right {:?}",
        heights[1],
        40.0
    );
}

struct LayoutOnlyEffect;

impl Effect for LayoutOnlyEffect {
    fn setup(
        &mut self,
        _ctx: &EffectContext<'_>,
    ) -> impl std::future::Future<Output = EffectSetupResult> {
        std::future::ready(Ok(()))
    }

    fn encode_render(
        &mut self,
        _input: &EffectInput<'_>,
        _output: &EffectOutput<'_>,
        _encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        panic!("layout-only verification must not invoke effect rendering");
    }

    fn retained_memory(&self) -> RetainedMemory {
        RetainedMemory::default()
    }

    fn trim(&mut self) {}
}

#[test]
fn view_effect_relayouts_equal_bounds_with_a_new_proposal() {
    let env = test_environment();
    let mut renderer = test_renderer();
    let mut fixture = Fixture::new(
        false,
        |content| FilteredView::new(content, LayoutOnlyEffect),
        &mut renderer,
        &env,
    );
    for main in [None, Some(160.0), None] {
        fixture.trace.borrow_mut().clear();
        let proposal = axis_proposal(false, main, Some(20.0));
        fixture
            .node
            .layout(&mut renderer, &env, None, proposal, Size::new(160.0, 20.0));
        fixture.assert_placement(
            false,
            proposal,
            if main.is_none() {
                [40.0, 120.0]
            } else {
                [80.0, 80.0]
            },
        );
    }
}

#[test]
fn nested_collections_preserve_intrinsic_cross_axes() {
    use waterui_core::id::SelfId;
    use waterui_layout::stack::{HStack, VStack};

    let env = test_environment();
    let mut renderer = test_renderer();
    let grid = VStack::for_each((0..16).map(SelfId::new).collect::<Vec<_>>(), |_| {
        HStack::for_each((0..10).map(SelfId::new).collect::<Vec<_>>(), |_| {
            Color::srgb_hex("#2563EB").size(28.0, 18.0)
        })
        .spacing(4.0)
    })
    .spacing(4.0);
    let node = RenderNode::build(AnyView::new(grid), &env, &mut renderer);
    let theme = renderer.theme();
    for width in [None, Some(0.0), Some(400.0), None] {
        let size = node
            .measure(
                &mut renderer.state,
                &env,
                &theme,
                ProposalSize::new(width, None),
            )
            .size;
        assert_eq!(size, Size::new(316.0, 348.0));
    }
}
