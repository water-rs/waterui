//! Tab containers rendered end to end.
//!
//! Run with `--no-capture` to export `/tmp/waterui_dew_tabs.png` for visual
//! review.

use nami::binding;
use waterui::prelude::*;
use waterui_backend_core::input::TouchPhase;
use waterui_controls::toggle::toggle;
use waterui_dew::{DewRuntime, HostBoard, PointerSample};
use waterui_navigation::{NavigationView, Tab, Tabs, tab_style};

mod support;

use support::{display_srgb, full_width_solid_fills, only_solid_fill};

const WIDTH: u32 = 240;
const HEIGHT: u32 = 240;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Screen {
    Now,
    Later,
}

fn tap(runtime: &mut DewRuntime<HostBoard>, x: f64, y: f64) -> Option<waterui_dew::Frame> {
    runtime.board_mut().push_pointer(PointerSample {
        x,
        y,
        phase: TouchPhase::Started,
    });
    runtime.board_mut().push_pointer(PointerSample {
        x,
        y,
        phase: TouchPhase::Ended,
    });
    runtime.pump()
}

fn tap_labeled(runtime: &mut DewRuntime<HostBoard>, label: &str) -> Option<waterui_dew::Frame> {
    let bounds = runtime
        .board()
        .accessibility_tree()
        .expect("a rendered host frame publishes accessibility bounds")
        .nodes
        .iter()
        .find_map(|(_, node)| {
            (node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
        .unwrap_or_else(|| panic!("the visible control `{label}` has accessibility bounds"));
    tap(
        runtime,
        f64::midpoint(bounds.x0, bounds.x1),
        f64::midpoint(bounds.y0, bounds.y1),
    )
}

/// The tab bar's top edge is where the page stops — one edge, not two.
///
/// The bar is painted in kurbo's `f64` and the page is framed in layout's
/// `f32`, so an edge computed on both sides at its own precision describes two
/// positions up to half an `f32` ULP apart: a hairline of page under the bar,
/// or a hairline of window background between them. The bar's height comes from
/// the tallest tab label, so whether it happens to land on the `f32` grid is a
/// property of the installed font. The window is a fractional height here so
/// the seam is off that grid whatever the font measures — a scaled display
/// hands a backend exactly such bounds — and the assertion is bit-for-bit,
/// because half an ULP is the whole defect.
#[test]
fn the_tab_bar_starts_exactly_where_the_page_ends() {
    /// A window height a scaled display produces, and one whose distance from
    /// the bar's height is not representable in `f32`.
    const FRACTIONAL_HEIGHT: f64 = 240.1;

    let selection = binding(Screen::Now);
    let mut renderer = support::test_renderer();
    let list = renderer.render_tree(
        AnyView::new(Tabs::new(
            &selection,
            vec![
                Tab::new(Screen::Now, "Now", || {
                    NavigationView::new("Now", Color::srgb(255, 0, 0))
                }),
                Tab::new(Screen::Later, "Later", || {
                    NavigationView::new("Later", Color::srgb(0, 0, 255))
                }),
            ],
        )),
        &support::test_environment(),
        f64::from(WIDTH),
        FRACTIONAL_HEIGHT,
    );

    let page = only_solid_fill(&list, display_srgb(255, 0, 0));
    // The page sits between two full-width surfaces: the navigation bar its own
    // destination draws, and the tab bar at the foot of the window.
    let mut bars = full_width_solid_fills(&list, f64::from(WIDTH))
        .into_iter()
        .filter(|bounds| bounds.height() > 1.0 && *bounds != page)
        .collect::<Vec<_>>();
    bars.sort_by(|left, right| left.y0.total_cmp(&right.y0));
    let [_navigation_bar, bar] = bars.as_slice() else {
        panic!("a tab container over a navigation view emits two bars, got {bars:?}")
    };
    assert_eq!(
        page.y1.to_bits(),
        bar.y0.to_bits(),
        "the page {page:?} and the bar {bar:?} disagree about the seam"
    );
}

/// Selecting a tab shows its page, and coming back shows the first page with
/// the state it had — the tab's page is retained, not rebuilt.
#[test]
fn a_tab_keeps_its_page_across_a_round_trip() {
    let selection = binding(Screen::Now);
    let flag = binding(false);
    let observed = flag.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(WIDTH, HEIGHT),
        support::test_environment(),
        16,
        move || {
            let flag = flag.clone();
            AnyView::new(Tabs::new(
                &selection,
                vec![
                    Tab::new(Screen::Now, "Now", move || {
                        NavigationView::new("Now", toggle("Ready", &flag))
                    }),
                    Tab::new(Screen::Later, "Later", || {
                        NavigationView::new("Later", Color::blue())
                    }),
                ],
            ))
        },
    );
    runtime.pump().expect("the first frame renders");

    tap_labeled(&mut runtime, "Ready");
    assert!(observed.get(), "the first tab's toggle flips");

    // The tab bar splits the foot of the panel in two.
    tap(&mut runtime, 180.0, 225.0);
    assert!(
        runtime.pump().is_none(),
        "switching tabs is instantaneous: nothing animates afterwards"
    );
    tap(&mut runtime, 60.0, 225.0);
    assert!(
        observed.get(),
        "the first tab's page is retained, so its state comes back with it"
    );
}

/// A disabled tab refuses selection.
#[test]
fn a_disabled_tab_cannot_be_selected() {
    let selection = binding(Screen::Now);
    let observed = selection.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(WIDTH, HEIGHT),
        support::test_environment(),
        16,
        move || {
            AnyView::new(Tabs::new(
                &selection,
                vec![
                    Tab::new(Screen::Now, "Now", || {
                        NavigationView::new("Now", Color::red())
                    }),
                    Tab::new(Screen::Later, "Later", || {
                        NavigationView::new("Later", Color::blue())
                    })
                    .enabled(false),
                ],
            ))
        },
    );
    runtime.pump().expect("the first frame renders");
    tap(&mut runtime, 180.0, 225.0);
    assert_eq!(
        observed.get(),
        Screen::Now,
        "a disabled tab leaves the selection alone"
    );
}

/// Visual review artifact: two tabs, one selected, one carrying a badge.
#[test]
fn export_tabs_for_visual_review() {
    let selection = binding(Screen::Now);
    let mut runtime = DewRuntime::new(
        HostBoard::new(WIDTH, HEIGHT),
        support::test_environment(),
        16,
        move || {
            AnyView::new(Tabs::new(
                &selection,
                vec![
                    Tab::new(Screen::Now, "Now", || {
                        NavigationView::new("Now", vstack((text("21.5 °C"), text("Heating"))))
                    }),
                    Tab::new(Screen::Later, "Later", || {
                        NavigationView::new("Later", text("Nothing scheduled"))
                    })
                    .badge(3),
                ],
            ))
        },
    );
    runtime.pump().expect("the first frame renders");
    std::fs::write(
        support::export_path("tabs", "tabs"),
        runtime.board().framebuffer().to_png(),
    )
    .expect("export visual review PNG");
}

/// Sidebar style places tab targets down the leading edge while retaining the
/// same lazily opened pages as the bottom-bar realization.
#[test]
fn sidebar_tabs_select_and_retain_pages() {
    let selection = binding(Screen::Now);
    let observed = selection.clone();
    let mut runtime = DewRuntime::new(
        HostBoard::new(480, HEIGHT),
        support::test_environment(),
        16,
        move || {
            AnyView::new(
                Tabs::new(
                    &selection,
                    vec![
                        Tab::new(Screen::Now, "Now", || {
                            NavigationView::new("Now", Color::red())
                        }),
                        Tab::new(Screen::Later, "Later", || {
                            NavigationView::new("Later", Color::blue())
                        }),
                    ],
                )
                .style(tab_style::sidebar()),
            )
        },
    );
    runtime.pump().expect("the sidebar's first frame renders");
    std::fs::write(
        support::export_path("tabs", "sidebar"),
        runtime.board().framebuffer().to_png(),
    )
    .expect("export sidebar visual review PNG");

    tap(&mut runtime, 20.0, 180.0).expect("the second sidebar row selects its page");
    assert_eq!(observed.get(), Screen::Later);
    assert!(
        runtime.pump().is_none(),
        "sidebar selection is instantaneous"
    );

    tap(&mut runtime, 20.0, 60.0).expect("the first retained page returns");
    assert_eq!(observed.get(), Screen::Now);
}
