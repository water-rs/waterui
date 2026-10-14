//! water-rs/hydrolysis#162 — `hstack((scene_pane, handle.width(20), scene_pane))`
//! in an 800-point window placed the children [400, 20, 400] = 820 instead of
//! [390, 20, 390] = 800: a leaf answered more than its offer, so the row
//! overflowed the window.
//!
//! These tests rebuild the issue's tree in the retained render tree with the
//! real input-receiving leaves (`GpuSurface` and merged `SceneView`), then
//! re-drive the container's `Layout` through a recording `SubView` that logs
//! every probe — ideal (main `None`), minimum (main `0`), maximum (main
//! `INFINITY`) — and every negotiated offer alongside the answer the leaf gave.
//! A leaf answering more than the offer it was just measured at is the contract
//! break; the layout must never clip it back (layout-spec §4.2).

use std::cell::RefCell;
use std::rc::Rc;

use waterui::{AnyView, Color, View, ViewExt as _};
use waterui_core::Environment;
use waterui_core::id::SelfId;
use waterui_core::layout::{StretchAxis, ViewDimensions};
use waterui_core::views::ForEach;
use waterui_graphics::draw::Recorder;
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui_graphics::input::SurfaceInputEvent;
use waterui_graphics::{
    GpuContent, GpuContentView, RecordingResources, SceneContent, SceneInvalidator, SceneView,
    SceneViewMergeToParent,
};
use waterui_layout::LazyContainer;
use waterui_layout::stack::hstack;
use waterui_layout::{Divider, ProposalSize, Rect, Size, SubView};

use super::{test_environment, test_renderer};
use crate::engine::WidgetTheme;
use crate::renderer::HydroState;
use crate::renderer::tree::RenderNode;

const WINDOW_WIDTH: f32 = 800.0;
const WINDOW_HEIGHT: f32 = 600.0;
const HANDLE_WIDTH: f32 = 20.0;
const DIVIDER_WIDTH: f32 = 7.0;

/// The issue's pane: a GPU surface that accepts its own input events, so the
/// leaf is the same kind a terminal or embedded engine installs.
struct PaneProbe;

impl GpuContent for PaneProbe {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {}

    fn render(&mut self, _frame: &mut GpuFrame<'_>) {}
}

/// The same pane as self-drawn scene content — the merged `SceneView` leaf a
/// hydroterm-style window carries when `SceneViewMergeToParent` is installed.
struct ScenePane;

impl SceneContent for ScenePane {
    fn build_scene(
        &mut self,
        _recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> bool {
        false
    }

    fn set_invalidator(&mut self, _invalidator: Option<SceneInvalidator>) {}

    fn rebuild_for_engine(&mut self) {}

    fn wants_input_events(&self) -> bool {
        true
    }

    fn input(&mut self, _event: &SurfaceInputEvent) {}
}

/// The same pane but content-sized: an input-receiving scene that reports a
/// natural size, the way a terminal pane sizes to its cell grid.
struct IntrinsicScenePane;

impl SceneContent for IntrinsicScenePane {
    fn build_scene(
        &mut self,
        _recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> bool {
        false
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(Size::new(400.0, 600.0))
    }

    fn wants_input_events(&self) -> bool {
        true
    }

    fn input(&mut self, _event: &SurfaceInputEvent) {}

    fn rebuild_for_engine(&mut self) {}
}

/// Which kind of input-receiving leaf the panes are built from.
#[derive(Clone, Copy)]
enum Pane {
    GpuSurface,
    Scene,
}

/// Every measure a child answered during a negotiation: the proposal it was
/// given, the size it reported before the stretch fill, and what the
/// `SubView`-level answer became after stretch was applied.
type MeasureLog = Rc<RefCell<Vec<MeasureEntry>>>;

struct MeasureEntry {
    label: &'static str,
    proposal: ProposalSize,
    answer: Size,
    reported: Size,
}

/// A `SubView` over a retained child node that forwards the live proposal into
/// `RenderNode::measure` exactly like `NodeSubView` does, and records each
/// probe so a test can see which leaf answered more than its offer.
struct OfferProbe<'a> {
    label: &'static str,
    node: &'a RenderNode,
    state: &'a RefCell<&'a mut HydroState>,
    env: Environment,
    theme: Rc<dyn WidgetTheme>,
    stretch: StretchAxis,
    log: MeasureLog,
}

impl SubView for OfferProbe<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let answer = {
            let mut state = self.state.borrow_mut();
            self.node
                .measure(&mut state, &self.env, &self.theme, proposal)
        };
        let mut reported = answer.clone();
        if self.stretch.stretches_horizontal()
            && let Some(width) = proposal.width
        {
            reported.size.width = reported.size.width.max(width);
        }
        if self.stretch.stretches_vertical()
            && let Some(height) = proposal.height
        {
            reported.size.height = reported.size.height.max(height);
        }
        self.log.borrow_mut().push(MeasureEntry {
            label: self.label,
            proposal,
            answer: answer.size,
            reported: reported.size,
        });
        reported
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch
    }

    fn priority(&self) -> i32 {
        0
    }

    fn is_empty(&self) -> bool {
        false
    }
}

fn pane_view(kind: Pane) -> AnyView {
    match kind {
        Pane::GpuSurface => AnyView::new(GpuContentView::new(PaneProbe).on_input(|_| {})),
        Pane::Scene => AnyView::new(SceneView::new(ScenePane)),
    }
}

/// The issue's tree: two stretch panes flanking a rigid 20-point handle with
/// no spacing, so the fair share is exactly `(800 - 20) / 2 = 390`.
fn issue_tree(kind: Pane) -> impl View {
    hstack((
        pane_view(kind),
        Color::srgb_hex("#3F3F46").width(HANDLE_WIDTH),
        pane_view(kind),
    ))
    .spacing(0.0)
}

fn build_issue_tree(kind: Pane) -> (Environment, crate::renderer::HydrolysisRenderer, RenderNode) {
    let env = test_environment().extending(SceneViewMergeToParent);
    let mut renderer = test_renderer();
    let node = RenderNode::build(AnyView::new(issue_tree(kind)), &env, &mut renderer);
    (env, renderer, node)
}

/// Runs the issue's hstack through the real negotiation with the recording
/// probes in place of `NodeSubView`, and returns the probe log plus the
/// placements `place` produced.
fn probe_issue_stack(
    kind: Pane,
) -> (
    Environment,
    crate::renderer::HydrolysisRenderer,
    RenderNode,
    MeasureLog,
) {
    let (env, mut renderer, node) = build_issue_tree(kind);
    let theme = renderer.theme();
    let container = node
        .transparent_container()
        .expect("an hstack must build a container node");
    assert_eq!(container.children.len(), 3, "pane, handle, pane");

    let labels = ["pane (leading)", "handle", "pane (trailing)"];
    let stretches = [
        StretchAxis::Both, // the stretch panes fill whatever they are offered
        StretchAxis::None, // `.width(20)` pins the handle's main axis
        StretchAxis::Both,
    ];
    let log: MeasureLog = Rc::new(RefCell::new(Vec::new()));
    let cell = RefCell::new(&mut renderer.state);
    let subs: Vec<OfferProbe<'_>> = container
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| OfferProbe {
            label: labels[index],
            node: child,
            state: &cell,
            env: env.clone(),
            theme: Rc::clone(&theme),
            stretch: stretches[index],
            log: Rc::clone(&log),
        })
        .collect();
    let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();

    let proposal = ProposalSize::new(Some(WINDOW_WIDTH), Some(WINDOW_HEIGHT));
    let size = container.layout.size_that_fits(proposal, &refs);
    let placements = container
        .layout
        .place(Rect::from_size(size), proposal, &refs);

    dump_probe_log(&log, &placements);
    (env, renderer, node, log)
}

fn dump_probe_log(log: &MeasureLog, placements: &[waterui_layout::SubviewPlacement]) {
    eprintln!("probe log for the issue hstack at 800x600:");
    for entry in log.borrow().iter() {
        let probe = match entry.proposal.width {
            None => "ideal".to_owned(),
            Some(0.0) => "min".to_owned(),
            Some(w) if w == f32::INFINITY => "max".to_owned(),
            Some(w) => format!("offer {w}"),
        };
        eprintln!(
            "  {:<14} probe={probe:<9} proposal=({:?}, {:?})  answer=({:.1}, {:.1})  reported=({:.1}, {:.1})",
            entry.label,
            entry.proposal.width,
            entry.proposal.height,
            entry.answer.width,
            entry.answer.height,
            entry.reported.width,
            entry.reported.height,
        );
    }
    eprintln!("placements:");
    for placement in placements {
        eprintln!(
            "  frame=({:.1}, {:.1})+({:.1}, {:.1})  proposal=({:?}, {:?})",
            placement.frame.x(),
            placement.frame.y(),
            placement.frame.width(),
            placement.frame.height(),
            placement.proposal.width,
            placement.proposal.height,
        );
    }
}

/// No leaf may answer more on an axis than the finite extent it was just
/// offered — except when its own measured minimum requires more. The min
/// probe (main `0`) recorded per label supplies that floor (layout-spec
/// §4.2: "never more unless its measured minimum requires it").
fn assert_no_answer_beats_its_offer(log: &MeasureLog) {
    let minima: std::collections::HashMap<&'static str, f32> = log
        .borrow()
        .iter()
        .filter(|entry| entry.proposal.width == Some(0.0))
        .map(|entry| (entry.label, entry.answer.width))
        .collect();
    for entry in log.borrow().iter() {
        if let Some(offered) = entry.proposal.width.filter(|w| w.is_finite() && *w > 0.0) {
            let floor = minima.get(entry.label).copied().unwrap_or(0.0);
            let allowed = offered.max(floor);
            assert!(
                entry.answer.width <= allowed + f32::EPSILON,
                "{} answered {} wide to a {} offer (its measured minimum is {}) \
                 — a leaf may not out-answer its offer unless its minimum \
                 requires it; the stack cannot clip it back",
                entry.label,
                entry.answer.width,
                offered,
                floor,
            );
        }
        if let Some(offered) = entry.proposal.height.filter(|h| h.is_finite()) {
            assert!(
                entry.answer.height <= offered + f32::EPSILON,
                "{} answered {} tall to a {} offer — a leaf may not out-answer \
                 its offer; the stack cannot clip it back",
                entry.label,
                entry.answer.height,
                offered,
            );
        }
    }
}

/// The failing half of #162: a pane that is a `Dynamic` wrapping an
/// input-receiving `SceneView` — the shape a terminal pane takes when its
/// scene content is swapped reactively — answers through the dispatch path
/// (`measure_dynamic`) once the retained tree has connected it, and the
/// dispatch answer must still honor the offer.
///
/// Sequence: an intrinsic probe while the `Dynamic`s are unconnected fills
/// `dynamic_intrinsic` with each pane's natural size; the retained build then
/// connects the `Dynamic` sources; a container that later sizes the same
/// `Dynamic` leaves via `measure_layout_dimensions` (a scroll sample, a lazy
/// estimate, a transient content measure) must get the offer-shaped answer,
/// not the stale intrinsic.
#[test]
fn connected_dynamic_panes_answer_the_offer_not_their_intrinsic() {
    use crate::renderer::{
        measure_view_dimensions, measure_view_dimensions_with_proposal, normalize_layout_view,
    };
    use waterui_core::dynamic::Dynamic;

    let (h1, d1) = Dynamic::new();
    h1.set(SceneView::new(IntrinsicScenePane));
    let (h2, d2) = Dynamic::new();
    h2.set(SceneView::new(IntrinsicScenePane));

    let make_stack = |d1: &Dynamic, d2: &Dynamic| -> AnyView {
        AnyView::new(
            hstack((
                d1.clone(),
                Color::srgb_hex("#3F3F46").width(HANDLE_WIDTH),
                d2.clone(),
            ))
            .spacing(0.0),
        )
    };
    let env = test_environment().extending(SceneViewMergeToParent);
    let mut renderer = test_renderer();
    let theme = renderer.theme();

    // A: an intrinsic probe while the dynamics are still unconnected populates
    // `dynamic_intrinsic` with each pane's natural (400x600) size — the same
    // thing a container intrinsic estimate does before the window exists.
    let stack_a = normalize_layout_view(make_stack(&d1, &d2), &env);
    let intrinsic = measure_view_dimensions(&stack_a, &mut renderer.state, &env, &theme);
    eprintln!("intrinsic of the stack = {:?}", intrinsic.size);

    // B: the retained build connects each Dynamic source (its content moves
    // into DynamicHostNode children and the unconnected snapshot is gone).
    let stack_b = normalize_layout_view(make_stack(&d1, &d2), &env);
    let _node = RenderNode::build(stack_b, &env, &mut renderer);

    // Print every connected leaf's answer to its min probe, its max probe and
    // the offer the stack computes for it — the leaf-level evidence.
    let proposal = ProposalSize::new(Some(WINDOW_WIDTH), Some(WINDOW_HEIGHT));
    for (label, dynamic) in [("pane (leading)", &d1), ("pane (trailing)", &d2)] {
        for (probe, probe_proposal) in [
            ("ideal", ProposalSize::new(None, Some(WINDOW_HEIGHT))),
            ("min", ProposalSize::new(Some(0.0), Some(WINDOW_HEIGHT))),
            (
                "max",
                ProposalSize::new(Some(f32::INFINITY), Some(WINDOW_HEIGHT)),
            ),
            ("offer", ProposalSize::new(Some(390.0), Some(WINDOW_HEIGHT))),
        ] {
            let leaf = normalize_layout_view(AnyView::new(dynamic.clone()), &env);
            let answer = measure_view_dimensions_with_proposal(
                &leaf,
                probe_proposal,
                &mut renderer.state,
                &env,
                &theme,
            );
            eprintln!(
                "  {label:<14} probe={probe:<6} proposal=({:?}, {:?})  answer=({:.1}, {:.1})",
                probe_proposal.width, probe_proposal.height, answer.size.width, answer.size.height,
            );
        }
    }

    // C: the dispatch measure of the same stack at the window's offer — what
    // `measure_layout_dimensions` returns to a container sizing it.
    let stack_c = normalize_layout_view(make_stack(&d1, &d2), &env);
    let dims = measure_view_dimensions_with_proposal(
        &stack_c,
        proposal,
        &mut renderer.state,
        &env,
        &theme,
    );
    eprintln!("dispatch dims of the stack at 800x600 = {:?}", dims.size);
    assert!(
        approx::relative_eq!(dims.size.width, WINDOW_WIDTH),
        "the hstack must fit the 800-point window: a connected `Dynamic` pane \
         must answer its offered share, not the intrinsic cached before the \
         retained tree connected it (the issue saw 820 = 400 + 20 + 400): left {:?}, right {:?}",
        dims.size.width,
        WINDOW_WIDTH
    );
}

#[test]
fn gpu_surface_panes_take_their_offered_share() {
    let (env, mut renderer, mut node, log) = probe_issue_stack(Pane::GpuSurface);
    assert_no_answer_beats_its_offer(&log);

    // And the real pipeline agrees: measure + place the built node and check
    // the cached child frames the flush pass will draw at.
    let theme = renderer.theme();
    let proposal = ProposalSize::new(Some(WINDOW_WIDTH), Some(WINDOW_HEIGHT));
    let measured = node.measure(&mut renderer.state, &env, &theme, proposal);
    node.layout(&mut renderer, &env, None, proposal, measured.size);
    let widths: Vec<f32> = node
        .transparent_container()
        .expect("an hstack must build a container node")
        .placed
        .iter()
        .map(Rect::width)
        .collect();
    assert_eq!(
        widths,
        vec![390.0, HANDLE_WIDTH, 390.0],
        "the three children must share the 800-point row: the issue saw \
         [400, 20, 400] = 820 because a leaf answered its ideal instead of \
         its offer"
    );
}

#[test]
fn scene_view_panes_take_their_offered_share() {
    let (env, mut renderer, mut node, log) = probe_issue_stack(Pane::Scene);
    assert_no_answer_beats_its_offer(&log);

    let theme = renderer.theme();
    let proposal = ProposalSize::new(Some(WINDOW_WIDTH), Some(WINDOW_HEIGHT));
    let measured = node.measure(&mut renderer.state, &env, &theme, proposal);
    node.layout(&mut renderer, &env, None, proposal, measured.size);
    let widths: Vec<f32> = node
        .transparent_container()
        .expect("an hstack must build a container node")
        .placed
        .iter()
        .map(Rect::width)
        .collect();
    assert_eq!(
        widths,
        vec![390.0, HANDLE_WIDTH, 390.0],
        "the three children must share the 800-point row: the issue saw \
         [400, 20, 400] = 820 because a leaf answered its ideal instead of \
         its offer"
    );
}

/// #162 reopened — the residual: `hstack((pane, divider.width(7), pane))` in
/// an 800-point window, each pane a lazy horizontal stack of input-receiving
/// `SceneView`s — the shape a custom pager or split row takes when it builds
/// on `LazyContainer` directly (the `Lazy::hstack` helper wraps a
/// `ScrollView`, which answers proposals itself and hides this path).
///
/// The row mapped at 807 because `LazyStackNode::measure` answered the
/// extent index's cached intrinsic total (`total_extent()`) to every probe —
/// including the negotiation's `with_main(0)` minimum probe — so the pane's
/// measured minimum was its ideal 400, `total_minima` (807) exceeded the
/// 800 budget, and `negotiate`'s `minima_overflow` arm offered each pane its
/// own minimum (waterui `distribute.rs`): an intrinsic cached as a minimum.
/// A lazy stack virtualizes along its main axis — it can report any extent
/// up to its content's — so a finite offer caps the answer while an open
/// axis still reads the full extent (spec §5, §6 `ScrollView`).
#[test]
fn lazy_stack_panes_split_the_divider_width() {
    use crate::renderer::{measure_view_dimensions_with_proposal, normalize_layout_view};
    use waterui_layout::stack::HStackLayout;

    let env = test_environment().extending(SceneViewMergeToParent);
    let mut renderer = test_renderer();
    let lazy_pane = || {
        LazyContainer::new(
            HStackLayout::default(),
            ForEach::new(vec![SelfId::new(0_usize)], |_| {
                SceneView::new(IntrinsicScenePane)
            }),
        )
    };
    let tree = hstack((lazy_pane(), Divider.width(DIVIDER_WIDTH), lazy_pane())).spacing(0.0);
    let node = RenderNode::build(AnyView::new(tree), &env, &mut renderer);
    let theme = renderer.theme();
    let container = node
        .transparent_container()
        .expect("an hstack must build a container node");
    assert_eq!(container.children.len(), 3, "pane, divider, pane");

    // The leaf-level evidence: each child's answer to its min, ideal and max
    // probes, and to every offer the negotiation computes for it.
    let labels = ["pane (leading)", "divider", "pane (trailing)"];
    let log: MeasureLog = Rc::new(RefCell::new(Vec::new()));
    let cell = RefCell::new(&mut renderer.state);
    let subs: Vec<OfferProbe<'_>> = container
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| OfferProbe {
            label: labels[index],
            node: child,
            state: &cell,
            env: env.clone(),
            theme: Rc::clone(&theme),
            stretch: child.stretch_for_test(),
            log: Rc::clone(&log),
        })
        .collect();
    let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();

    let proposal = ProposalSize::new(Some(WINDOW_WIDTH), Some(WINDOW_HEIGHT));
    let size = container.layout.size_that_fits(proposal, &refs);
    let placements = container
        .layout
        .place(Rect::from_size(size), proposal, &refs);
    dump_probe_log(&log, &placements);
    eprintln!("size_that_fits = ({:.1}, {:.1})", size.width, size.height);

    assert_no_answer_beats_its_offer(&log);
    assert!(
        approx::relative_eq!(size.width, WINDOW_WIDTH),
        "the hstack must fit the 800-point window: a lazy pane must answer \
         its offered share, not the intrinsic cached in its extent index \
         (the issue saw 807 = 400 + 7 + 400): left {:?}, right {:?}",
        size.width,
        WINDOW_WIDTH
    );
    let widths: Vec<f32> = placements
        .iter()
        .map(|placement| placement.frame.width())
        .collect();
    assert_eq!(
        widths,
        vec![396.5, DIVIDER_WIDTH, 396.5],
        "the three children must share the 800-point row"
    );

    // The dispatch path (`Native<LazyContainer>::dimensions`, used whenever a
    // lazy stack is measured transiently — a `FixedContainer` intrinsic
    // estimate, a scroll sample, a button's content) answers by the same
    // rule: an open main axis reads the membership's intrinsic extent, a
    // finite offer caps it, a zero probe reports zero.
    let leaf = normalize_layout_view(AnyView::new(lazy_pane()), &env);
    for (probe, probe_proposal, expected) in [
        ("ideal", ProposalSize::new(None, Some(WINDOW_HEIGHT)), 400.0),
        (
            "min",
            ProposalSize::new(Some(0.0), Some(WINDOW_HEIGHT)),
            0.0,
        ),
        (
            "max",
            ProposalSize::new(Some(f32::INFINITY), Some(WINDOW_HEIGHT)),
            400.0,
        ),
        (
            "offer",
            ProposalSize::new(Some(396.5), Some(WINDOW_HEIGHT)),
            396.5,
        ),
    ] {
        let dims = measure_view_dimensions_with_proposal(
            &leaf,
            probe_proposal,
            &mut renderer.state,
            &env,
            &theme,
        );
        eprintln!(
            "  lazy pane probe={probe:<6} proposal=({:?}, {:?})  answer=({:.1}, {:.1})",
            probe_proposal.width, probe_proposal.height, dims.size.width, dims.size.height,
        );
        assert!(
            approx::relative_eq!(dims.size.width, expected),
            "a lazy hstack measured through the dispatch path must answer \
             {expected} to its {probe} probe: left {:?}, right {:?}",
            dims.size.width,
            expected
        );
    }
}

/// #162 reopened: `hstack((pane, divider.width(7), pane))` in an 800-point
/// window, both panes input-receiving `SceneView`s — the shape a terminal
/// split takes. The row mapped at 807: each pane answered its intrinsic 400
/// against an offer of `(800 - 7) / 2 = 396.5`.
///
/// Same probe discipline as the original repro: the retained children are
/// driven through the real `HStackLayout` negotiation while an `OfferProbe`
/// per child logs every min/ideal/max probe answer and every negotiated
/// offer — the leaf-level evidence of which answer beat its offer.
#[test]
fn input_scene_view_panes_split_the_divider_width() {
    let env = test_environment().extending(SceneViewMergeToParent);
    let mut renderer = test_renderer();
    let tree = hstack((
        SceneView::new(IntrinsicScenePane),
        Divider.width(DIVIDER_WIDTH),
        SceneView::new(IntrinsicScenePane),
    ))
    .spacing(0.0);
    let node = RenderNode::build(AnyView::new(tree), &env, &mut renderer);
    let theme = renderer.theme();
    let container = node
        .transparent_container()
        .expect("an hstack must build a container node");
    assert_eq!(container.children.len(), 3, "pane, divider, pane");

    let labels = ["pane (leading)", "divider", "pane (trailing)"];
    let log: MeasureLog = Rc::new(RefCell::new(Vec::new()));
    let cell = RefCell::new(&mut renderer.state);
    let subs: Vec<OfferProbe<'_>> = container
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| OfferProbe {
            label: labels[index],
            node: child,
            state: &cell,
            env: env.clone(),
            theme: Rc::clone(&theme),
            stretch: child.stretch_for_test(),
            log: Rc::clone(&log),
        })
        .collect();
    let refs: Vec<&dyn SubView> = subs.iter().map(|sub| sub as &dyn SubView).collect();

    let proposal = ProposalSize::new(Some(WINDOW_WIDTH), Some(WINDOW_HEIGHT));
    let size = container.layout.size_that_fits(proposal, &refs);
    let placements = container
        .layout
        .place(Rect::from_size(size), proposal, &refs);
    dump_probe_log(&log, &placements);
    eprintln!("size_that_fits = ({:.1}, {:.1})", size.width, size.height);

    assert_no_answer_beats_its_offer(&log);
    assert!(
        approx::relative_eq!(size.width, WINDOW_WIDTH),
        "the hstack must fit the 800-point window: an input-receiving pane \
         must answer its offered share, not its intrinsic size (the issue \
         saw 807 = 400 + 7 + 400): left {:?}, right {:?}",
        size.width,
        WINDOW_WIDTH
    );
    let widths: Vec<f32> = placements
        .iter()
        .map(|placement| placement.frame.width())
        .collect();
    assert_eq!(
        widths,
        vec![396.5, DIVIDER_WIDTH, 396.5],
        "the three children must share the 800-point row"
    );
}
