//! Native tests that need a running `UIApplication`.
//!
//! `UIKit` drives part of its machinery — the animation behind
//! `UIScrollView.setContentOffset(_:animated: true)` among it — only from
//! the application's update cycle, which the bare `simctl spawn`ed `native`
//! suite never reaches. The `nextest-ios-sim.sh` target runner launches
//! this binary into a real `UIApplication` with a connected window scene,
//! one case per launch, from the bundle manifest embedded below;
//! `cocoa_ui::uikit::native_test::run` refuses to start outside that
//! launch. A launch costs far more than a spawn, so only cases that need
//! the application live here; everything else stays in `native`.
//!
//! The cases are iOS-only: on macOS the binary lists no trials.

use libtest_mimic::Arguments;

// The target runner launches this binary as an application from the
// bundle manifest embedded here.
#[cfg(target_os = "ios")]
cocoa_ui::native_test_info_plist!();

fn main() {
    let mut arguments = Arguments::from_args();
    // `UIKit` objects may only be built on the real main thread; a single
    // runner keeps every trial on the application's main thread.
    arguments.test_threads = Some(1);
    #[cfg(target_os = "ios")]
    cocoa_ui::uikit::native_test::run(arguments, scroll_animation::trials);
    #[cfg(target_os = "macos")]
    libtest_mimic::run(&arguments, Vec::new()).exit();
}

/// `UIKit`-driven animation (#2000): `setContentOffset(_:animated: true)`
/// advances the model `contentOffset` on the application's update cycle,
/// so it only moves inside a running `UIApplication` with a connected
/// scene. The offset must pass through intermediate values and land on the
/// target; a process outside the application never moves it.
#[cfg(target_os = "ios")]
mod scroll_animation {
    use std::cell::Cell;

    use cocoa_ui::geometry::{Point, Size};
    use cocoa_ui::uikit::{ScrollView, main_screen_scale, native_test};
    use cocoa_ui::{MainThreadMarker, Rect, view};
    use waterui_apple::native_test_support::pump_main_until;

    /// The bound an animated scroll gets to land before the case fails;
    /// `UIKit`'s scroll animation finishes in well under a second.
    const ANIMATION_DEADLINE: f64 = 5.0;

    /// The `scroll_animation::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![libtest_mimic::Trial::test(
            "scroll_animation::animated_content_offset_passes_through_intermediate_offsets",
            || {
                animated_content_offset_passes_through_intermediate_offsets();
                Ok(())
            },
        )]
    }

    fn animated_content_offset_passes_through_intermediate_offsets() {
        let mtm = MainThreadMarker::new().expect("native_app cases run on the main thread");
        let scroll = ScrollView::new(mtm, true, false);
        let window = native_test::window(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
        window.addSubview(&scroll);
        view::set_frame(&scroll, view::bounds(&window));
        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        let viewport = scroll.viewport_size();
        scroll.set_content_extent(Size::new(viewport.width, viewport.height * 10.0));
        scroll.layout_if_needed();

        let start = scroll.content_offset();
        let target = Point::new(start.x, viewport.height.mul_add(3.0, start.y));
        // Offsets land on the device's pixel grid, so "at" means within one.
        let pixel = 1.0 / main_screen_scale();
        let distance = |offset: Point| (target.x - offset.x).abs().max((target.y - offset.y).abs());
        scroll.set_content_offset(target, true);
        let intermediate = Cell::new(None);
        let landed = pump_main_until(ANIMATION_DEADLINE, || {
            let offset = scroll.content_offset();
            if offset.y - start.y >= pixel && distance(offset) >= pixel {
                intermediate.set(Some(offset));
            }
            distance(offset) < pixel
        });
        assert!(
            landed,
            "the animated content offset never reached {target:?} from {start:?}; it stayed at {:?}",
            scroll.content_offset()
        );
        assert!(
            intermediate.get().is_some(),
            "the content offset jumped from {start:?} to {target:?} without an intermediate frame"
        );
        window.setHidden(true);
    }
}
