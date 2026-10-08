//! Content a navigation page presents through its own compositor layer — a
//! `GpuContentView`, a `SceneView` — belongs to the page: the stack records
//! each page in the page's own space and presents it at the stack's place in
//! the window, and those layers have to be placed the same way and only when
//! the page itself is presented.
//!
//! Most assertions read the layer tree a frame commits through the test
//! `MirrorTarget`, where a producer's window placement and its presence are
//! decided; the last test drives the real engine frame, where a page's
//! layers are mounted on engine layers.

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use std::sync::Arc;
use std::time::Instant;

use kurbo::{Affine, Rect};
use waterui::ViewExt as _;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::layout::Size;
use waterui_core::{AnyView, Environment};
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui_graphics::{GpuContent, GpuContentView};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::vstack;
use waterui_navigation::{NavigationPath, NavigationStack, NavigationView, navigation_transition};
use waterui_shape::Rectangle;

use super::{MinimalTestTheme, pumped_test_environment, test_environment, test_renderer};
use crate::HeadlessRuntime;
use crate::platform::WindowSafeArea;
use crate::renderer::HydrolysisRenderer;

const WINDOW: Rect = Rect::new(0.0, 0.0, 390.0, 844.0);
const TOP_INSET: f32 = 48.0;
const ROOT_CONTENT: Size = Size::new(40.0, 30.0);
const DETAIL_CONTENT: Size = Size::new(60.0, 20.0);

/// GPU content that asks for a frame from every frame it renders, and counts
/// them: it renders exactly while one of its bindings is drawn.
#[derive(Clone, Default)]
struct Probe(Arc<AtomicU32>);

impl Probe {
    fn renders(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

impl GpuContent for Probe {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {}

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        self.0.fetch_add(1, Ordering::Relaxed);
        frame.request_redraw();
    }
}

fn gpu(size: Size) -> impl waterui_core::View {
    probe_view(Probe::default(), size)
}

fn probe_view(probe: Probe, size: Size) -> impl waterui_core::View {
    GpuContentView::new(probe).size(size.width, size.height)
}

fn env_with_top_inset(top: f32) -> Environment {
    let mut env = test_environment();
    env.insert(WindowSafeArea(nami::binding(EdgeInsets::new(
        top, 0.0, 0.0, 0.0,
    ))));
    env
}

fn capture(view: impl waterui_core::View, env: &Environment) -> HydrolysisRenderer {
    let mut renderer = test_renderer();
    render_frame(&mut renderer, AnyView::new(view), env);
    renderer
}

/// Renders one frame of the window tree; the tree is built by the first frame
/// and later frames reuse it, so `view` matters only the first time.
fn render_frame(renderer: &mut HydrolysisRenderer, view: AnyView, env: &Environment) {
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(view, env, WINDOW, Affine::IDENTITY, Affine::IDENTITY);
    renderer.finish_rebuild_frame();
    renderer.commit_mirror();
}

/// One presented GPU content layer: its window-space rect, the opacity it
/// composites at, and how many of the layers above it clip.
struct Presented {
    rect: Rect,
    alpha: f32,
    clips: usize,
}

/// Every GPU content layer the frame presents.
fn presented_gpu_layers(renderer: &HydrolysisRenderer) -> Vec<Presented> {
    let mirror = renderer.mirror();
    mirror
        .installs()
        .into_iter()
        .map(|(layer, (width, height))| {
            let ancestry = mirror.ancestry(layer);
            Presented {
                rect: mirror.world(layer).transform_rect_bbox(Rect::new(
                    0.0,
                    0.0,
                    f64::from(width),
                    f64::from(height),
                )),
                alpha: ancestry.iter().map(|node| node.opacity).product(),
                clips: ancestry.iter().filter(|node| node.clip.is_some()).count(),
            }
        })
        .collect()
}

/// Every GPU content layer the frame presents, as its window-space rect.
fn presented_gpu_rects(renderer: &HydrolysisRenderer) -> Vec<Rect> {
    presented_gpu_layers(renderer)
        .into_iter()
        .map(|layer| layer.rect)
        .collect()
}

fn single(rects: &[Rect]) -> Rect {
    assert_eq!(
        rects.len(),
        1,
        "exactly one GPU content layer should be presented, got {rects:?}"
    );
    rects[0]
}

fn root_stack() -> impl waterui_core::View {
    NavigationStack::new(NavigationView::new("Root", vstack((gpu(ROOT_CONTENT),))))
}

/// The page's GPU layer sits where the page is drawn: one top inset lower
/// than it sits without insets.
#[test]
fn a_page_gpu_layer_follows_the_stack_placement() {
    let bare = single(&presented_gpu_rects(&capture(
        root_stack(),
        &env_with_top_inset(0.0),
    )));
    let inset = single(&presented_gpu_rects(&capture(
        root_stack(),
        &env_with_top_inset(TOP_INSET),
    )));
    assert!(
        (inset.y0 - bare.y0 - f64::from(TOP_INSET)).abs() <= 0.5
            && (inset.x0 - bare.x0).abs() <= 0.5,
        "the GPU layer should move down by the top inset with the page: \
         {bare:?} without insets, {inset:?} with a {TOP_INSET} top inset"
    );
}

/// A stack inside a clip presents its pages' GPU content without breaking the
/// compositor's scope bookkeeping, at the same place as an unclipped stack and
/// under the stack's clip.
#[test]
fn a_clipped_stack_presents_its_page_gpu_layer() {
    let env = env_with_top_inset(TOP_INSET);
    let unclipped_renderer = capture(root_stack(), &env);
    let unclipped = single(&presented_gpu_rects(&unclipped_renderer));
    let renderer = capture(root_stack().clip(Rectangle), &env);
    let rect = single(&presented_gpu_rects(&renderer));
    assert!(
        (rect.x0 - unclipped.x0).abs() <= 0.5 && (rect.y0 - unclipped.y0).abs() <= 0.5,
        "the clipped stack's GPU layer should sit where the unclipped one does: \
         {rect:?} clipped, {unclipped:?} unclipped"
    );
    assert_eq!(
        presented_gpu_layers(&renderer)[0].clips,
        presented_gpu_layers(&unclipped_renderer)[0].clips + 1,
        "the page's GPU layer should be shown under the stack's clip"
    );
}

/// Only the presented page's layers reach the frame: the page under a pushed
/// destination is recorded every frame for the back gesture, not shown.
#[test]
fn a_covered_page_presents_no_gpu_layer() {
    let path = NavigationPath::<u8>::new();
    path.push(1);
    let stack = NavigationStack::with_path(
        path,
        NavigationView::new("Root", vstack((gpu(ROOT_CONTENT),))),
    )
    .destination(|_| NavigationView::new("Detail", vstack((gpu(DETAIL_CONTENT),))))
    // Settled at once: the frame shows the detail page alone.
    .transition(navigation_transition::none());
    let renderer = capture(stack, &env_with_top_inset(TOP_INSET));
    let rect = single(&presented_gpu_rects(&renderer));
    assert!(
        (rect.width() - f64::from(DETAIL_CONTENT.width)).abs() <= 0.5
            && (rect.height() - f64::from(DETAIL_CONTENT.height)).abs() <= 0.5,
        "the presented GPU layer should be the detail page's, got {rect:?}"
    );
}

/// Decision 3 through the engine's own mount log: a page that stops being
/// presented unmounts, so a covered page's GPU layer leaves the engine when a
/// destination settles over it — and mounts again when the stack pops back.
#[test]
fn a_non_presented_page_unmounts_and_remounts_its_gpu_layers() {
    let path = NavigationPath::<u8>::new();
    let stack = NavigationStack::with_path(
        path.clone(),
        NavigationView::new("Root", vstack((gpu(ROOT_CONTENT),))),
    )
    .destination(|_| NavigationView::new("Detail", vstack((gpu(DETAIL_CONTENT),))))
    .transition(navigation_transition::none());
    let env = test_environment();
    let mut renderer = test_renderer();
    let installed = |renderer: &HydrolysisRenderer| -> Vec<(f64, f64)> {
        renderer
            .mirror()
            .installs()
            .iter()
            .map(|(_, (w, h))| (f64::from(*w), f64::from(*h)))
            .collect()
    };
    let px = |size: Size| -> (f64, f64) { (f64::from(size.width), f64::from(size.height)) };

    render_frame(&mut renderer, AnyView::new(stack), &env);
    assert_eq!(
        installed(&renderer),
        vec![px(ROOT_CONTENT)],
        "the root page's GPU content mounts"
    );

    path.push(1);
    render_frame(&mut renderer, AnyView::new(()), &env);
    assert_eq!(
        installed(&renderer),
        vec![px(DETAIL_CONTENT)],
        "the covered page unmounts: only the presented page's install remains"
    );

    path.pop();
    render_frame(&mut renderer, AnyView::new(()), &env);
    assert_eq!(
        installed(&renderer),
        vec![px(ROOT_CONTENT)],
        "the page's layers mount again when it is presented again"
    );
}

/// A page in a push transition presents its GPU layer under the transition's
/// clip/opacity scope and moves with it. At the transition's first frame only
/// the outgoing root is visible, at its settled place; a few frames in, it
/// has slid toward the leading edge and is fading out.
#[test]
fn a_transitioning_page_gpu_layer_follows_the_transition() {
    let env = env_with_top_inset(TOP_INSET);
    let settled = single(&presented_gpu_rects(&capture(root_stack(), &env)));

    let path = NavigationPath::<u8>::new();
    path.push(1);
    let stack = NavigationStack::with_path(
        path,
        NavigationView::new("Root", vstack((gpu(ROOT_CONTENT),))),
    )
    .destination(|_| NavigationView::new("Detail", vstack((gpu(DETAIL_CONTENT),))));
    let mut renderer = test_renderer();
    let start = renderer.frame_instant();
    render_frame(&mut renderer, AnyView::new(stack), &env);

    let first = presented_gpu_layers(&renderer);
    assert_eq!(
        first.len(),
        1,
        "only the outgoing root is visible at the transition's first frame"
    );
    let rect = first[0].rect;
    assert!(
        (rect.x0 - settled.x0).abs() <= 0.5 && (rect.y0 - settled.y0).abs() <= 0.5,
        "the root's GPU layer should start at its settled place {settled:?}, got {rect:?}"
    );
    assert!(
        first[0].alpha > 0.0,
        "the root's GPU layer should be shown under the transition scope, got alpha {}",
        first[0].alpha
    );

    renderer.set_frame_instant(
        start
            .checked_add(Duration::from_millis(32))
            .expect("test frame instant overflow"),
    );
    render_frame(&mut renderer, AnyView::new(()), &env);
    let moving = presented_gpu_layers(&renderer);
    assert_eq!(
        moving.len(),
        1,
        "early in the transition only the outgoing root is visible"
    );
    let rect = moving[0].rect;
    assert!(
        rect.x0 < settled.x0 - 0.5 && (rect.y0 - settled.y0).abs() <= 0.5,
        "the root's GPU layer should slide toward the leading edge with its page: \
         settled at {settled:?}, at {rect:?} mid-transition"
    );
    let alpha = moving[0].alpha;
    assert!(
        alpha > 0.0 && alpha < 1.0,
        "the root's GPU layer should fade with its page, got scope alpha {alpha}"
    );
}

/// A page's GPU content keeps rendering after the page is covered and shown
/// again: the page's mount is dropped while it is covered, and the mount it
/// comes back on has to be bound to the content again.
#[test]
fn a_page_gpu_content_renders_again_after_push_and_pop() {
    let probe = Probe::default();
    let path = NavigationPath::<u8>::new();
    let views = RefCell::new(Some((probe.clone(), path.clone())));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let (probe, path) = views
            .borrow_mut()
            .take()
            .expect("the navigation window is built once");
        AnyView::new(
            NavigationStack::with_path(
                path,
                NavigationView::new("Root", vstack((probe_view(probe, ROOT_CONTENT),))),
            )
            .destination(|_| NavigationView::new("Detail", vstack((gpu(DETAIL_CONTENT),))))
            .transition(navigation_transition::none()),
        )
    });
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the test window size is a small positive constant"
    )]
    let mut runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WINDOW.width() as u32,
        WINDOW.height() as u32,
        MinimalTestTheme::default(),
    );
    let start = Instant::now();
    let mut next = 0;
    let mut pump = |runtime: &mut HeadlessRuntime, count: u64| {
        for _ in 0..count {
            let _ = runtime.pump_at(false, start + Duration::from_millis(next * 16));
            next += 1;
        }
    };

    pump(&mut runtime, 4);
    assert!(probe.renders() > 0, "the root page's GPU content renders");

    path.push(1);
    pump(&mut runtime, 4);
    path.pop();
    pump(&mut runtime, 2);
    let shown_again = probe.renders();
    pump(&mut runtime, 4);
    assert!(
        probe.renders() > shown_again,
        "the root page's GPU content should render again once the page is shown \
         again, got {} renders before and after four more frames",
        probe.renders()
    );
}
