//! Tab containers rendered end to end.
//!
//! Run with `--no-capture` to export `/tmp/waterui_dew_tabs.png` for visual
//! review.

use nami::binding;
use waterui::prelude::*;
use waterui_backend_core::input::TouchPhase;
use waterui_controls::toggle::toggle;
use waterui_dew::{DewRuntime, HostBoard, PointerSample};
use waterui_navigation::{NavigationView, Tab, Tabs};

mod support;

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

    // The toggle is the first row of the selected page, under its bar.
    tap(&mut runtime, 210.0, 43.0);
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
        "/tmp/waterui_dew_tabs.png",
        runtime.board().framebuffer().to_png(),
    )
    .expect("export visual review PNG");
}
