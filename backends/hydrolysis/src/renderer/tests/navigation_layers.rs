//! Content a navigation page presents through its own compositor layer — a
//! `GpuContentView`, a `SceneView` — belongs to the page: the stack records
//! each page in the page's own space and presents it at the stack's place in
//! the window, and those layers have to be placed the same way and only when
//! the page itself is presented.
//!
//! The assertions read the frame's `render_layers` before presentation, which
//! is where a layer's window placement and its presence are decided.

use kurbo::{Affine, Rect};
use waterui::ViewExt as _;
use waterui_core::layout::Size;
use waterui_core::{AnyView, Environment};
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui_graphics::{GpuContent, GpuContentView};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::vstack;
use waterui_navigation::{NavigationPath, NavigationStack, NavigationView, navigation_transition};
use waterui_shape::Rectangle;

use super::{test_environment, test_renderer};
use crate::platform::WindowSafeArea;
use crate::renderer::HydrolysisRenderer;
use crate::renderer::RenderLayer;

const WINDOW: Rect = Rect::new(0.0, 0.0, 390.0, 844.0);
const TOP_INSET: f32 = 48.0;
const ROOT_CONTENT: Size = Size::new(40.0, 30.0);
const DETAIL_CONTENT: Size = Size::new(60.0, 20.0);

struct Probe;

impl GpuContent for Probe {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {}

    fn render(&mut self, _frame: &mut GpuFrame<'_>) {}
}

fn gpu(size: Size) -> impl waterui_core::View {
    GpuContentView::new(Probe).size(size.width, size.height)
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
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(view),
        env,
        WINDOW,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
    renderer
}

/// Every GPU content layer the frame presents, as its window-space rect.
fn presented_gpu_rects(renderer: &HydrolysisRenderer) -> Vec<Rect> {
    fn walk(layers: &[RenderLayer], out: &mut Vec<Rect>) {
        for layer in layers {
            match layer {
                RenderLayer::GpuContent(layer) => {
                    out.push(layer.transform.transform_rect_bbox(layer.bounds));
                }
                RenderLayer::Filtered(layer) => walk(&layer.children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&renderer.compositor.render_layers, &mut out);
    out
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
/// compositor's scope bookkeeping.
#[test]
fn a_clipped_stack_presents_its_page_gpu_layer() {
    let renderer = capture(root_stack().clip(Rectangle), &env_with_top_inset(TOP_INSET));
    let rect = single(&presented_gpu_rects(&renderer));
    assert!(
        rect.y0 >= f64::from(TOP_INSET),
        "the clipped stack's GPU layer should sit below the inset, got {rect:?}"
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

/// A page in a push transition presents its GPU layer under the transition's
/// clip/opacity scope, the same scope its drawing is shown under. At the
/// first frame of the default transition only the outgoing root is visible.
#[test]
fn a_transitioning_page_gpu_layer_is_shown_under_the_transition_scope() {
    let path = NavigationPath::<u8>::new();
    path.push(1);
    let stack = NavigationStack::with_path(
        path,
        NavigationView::new("Root", vstack((gpu(ROOT_CONTENT),))),
    )
    .destination(|_| NavigationView::new("Detail", vstack((gpu(DETAIL_CONTENT),))));
    let renderer = capture(stack, &env_with_top_inset(TOP_INSET));
    let layers: Vec<_> = renderer
        .compositor
        .render_layers
        .iter()
        .filter_map(|layer| match layer {
            RenderLayer::GpuContent(layer) => Some(layer),
            _ => None,
        })
        .collect();
    assert_eq!(
        layers.len(),
        1,
        "only the outgoing root is visible at the transition's first frame"
    );
    let rect = layers[0].transform.transform_rect_bbox(layers[0].bounds);
    assert!(
        (rect.width() - f64::from(ROOT_CONTENT.width)).abs() <= 0.5,
        "the visible GPU layer should be the root page's, got {rect:?}"
    );
    assert_eq!(
        layers[0].active_layers.len(),
        1,
        "the root's GPU layer should be shown under the transition scope"
    );
}
