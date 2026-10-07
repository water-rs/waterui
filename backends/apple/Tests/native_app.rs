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
/// and `scrollToRow(_:at:animated: true)` advance the model
/// `contentOffset` on the application's update cycle, so they only move
/// inside a running `UIApplication` with a connected scene. Each runs on
/// the platform's wall clock: the offset is still at its start when the
/// request returns, and it lands on the target no sooner than a native
/// animation takes; a process outside the application never moves it.
/// This is where the `UIKit` `Animation::Default` arm of the `native`
/// scroll suites lives.
#[cfg(target_os = "ios")]
mod scroll_animation {
    use cocoa_ui::geometry::{Point, Size};
    use cocoa_ui::objc2_ui_kit::UIWindow;
    use cocoa_ui::uikit::{ScrollView, native_test};
    use cocoa_ui::{MainThreadMarker, PlatformView, Rect, Retained, view};
    use waterui::animation::Animation;
    use waterui::component::list::ListItem;
    use waterui::layout::scroll::ScrollController;
    use waterui::reactive::binding;
    use waterui_apple::native_test_support::{
        APPROACH_LIST_ROWS, APPROACH_ROW, APPROACH_TARGET_ROW, assert_native_scroll,
        assert_native_scroll_from, list_row_top, row_item, row_list, scroll_surface,
    };
    use waterui_core::layout::Point as LayoutPoint;

    /// The list's rows — far taller than the window.
    const ROWS: usize = 200;
    /// The row the list case aims at: below the fold, far from the end.
    const TARGET_ROW: usize = 40;

    /// The `scroll_animation::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        let cases: [(&str, fn()); 4] = [
            (
                "animated_content_offset_lands_after_a_native_animation",
                animated_content_offset_lands_after_a_native_animation,
            ),
            (
                "a_default_surface_request_lands_after_a_native_animation",
                a_default_surface_request_lands_after_a_native_animation,
            ),
            (
                "a_default_list_request_lands_after_a_native_animation",
                a_default_list_request_lands_after_a_native_animation,
            ),
            (
                "a_far_default_list_request_jumps_to_the_approach_row_first",
                a_far_default_list_request_jumps_to_the_approach_row_first,
            ),
        ];
        cases
            .into_iter()
            .map(|(name, case)| {
                libtest_mimic::Trial::test(format!("scroll_animation::{name}"), move || {
                    case();
                    Ok(())
                })
            })
            .collect()
    }

    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("native_app cases run on the main thread")
    }

    /// A key, visible window in the application's scene with `content`
    /// filling it, laid out.
    fn scene_window(mtm: MainThreadMarker, content: &PlatformView) -> Retained<UIWindow> {
        let window = native_test::window(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
        window.addSubview(content);
        view::set_frame(content, view::bounds(&window));
        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        window
    }

    fn animated_content_offset_lands_after_a_native_animation() {
        let mtm = mtm();
        let scroll = ScrollView::new(mtm, true, false);
        let window = scene_window(mtm, &scroll);
        let viewport = scroll.viewport_size();
        scroll.set_content_extent(Size::new(viewport.width, viewport.height * 10.0));
        scroll.layout_if_needed();

        let start = scroll.content_offset();
        let target = Point::new(start.x, viewport.height.mul_add(3.0, start.y));
        assert_native_scroll(
            "setContentOffset(_:animated: true)",
            || scroll.set_content_offset(target, true),
            || scroll.content_offset(),
            || target,
        );
        window.setHidden(true);
    }

    /// `Animation::Default` on a scroll surface is `UIKit`'s own
    /// `setContentOffset(_:animated: true)`: driven through the
    /// controller, it does not jump, and it lands where the jump to the
    /// same target lands no sooner than a native animation takes.
    fn a_default_surface_request_lands_after_a_native_animation() {
        let mtm = mtm();
        let origin = LayoutPoint::new(0.0, 0.0);
        let target = LayoutPoint::new(0.0, 800.0);
        let controller = ScrollController::new(origin);
        let reported = binding(origin);
        let (leaf, surface) = scroll_surface(&controller, &reported);
        let window = scene_window(mtm, leaf.view());
        surface.set_needs_layout();
        surface.layout_if_needed();

        controller.scroll_to(target);
        let landing = surface.content_offset();
        controller.scroll_to(origin);
        assert_native_scroll(
            "a Default surface request",
            || controller.animate_to(target, Animation::Default),
            || surface.content_offset(),
            || landing,
        );
        window.setHidden(true);
    }

    /// `Animation::Default` on a list is `UIKit`'s own
    /// `scrollToRow(_:at:animated: true)`: driven through the controller
    /// on a cold table, it does not jump, and it lands with the row's top
    /// at the viewport's top no sooner than a native animation takes.
    /// Rows `UIKit` has not shown are sized by estimate, and the scroll
    /// measures them on the way, so the row's top is read as it stands when the offset lands —
    /// not taken from an earlier jump, which resolves against estimates.
    fn a_default_list_request_lands_after_a_native_animation() {
        let mtm = mtm();
        let controller = ScrollController::new(0usize);
        let (leaf, table) = row_list(vec![row_item as fn() -> ListItem; ROWS], &controller);
        let window = scene_window(mtm, leaf.view());
        table.layout_if_needed();
        let offset = || Point::from(table.contentOffset());

        assert_native_scroll(
            "a Default list request",
            || controller.animate_to(TARGET_ROW, Animation::Default),
            offset,
            || list_row_top(&table, TARGET_ROW),
        );
        window.setHidden(true);
    }

    /// A far `Animation::Default` list request animates only the final
    /// stretch: `UIKit`'s native row scroll toward a target further than
    /// the approach bound first jumps unanimated to the approach row's
    /// top — the offset right after the request — and lands from there
    /// with the target row's top at the viewport's top, read as it
    /// stands at landing since unseen rows are sized by estimate.
    fn a_far_default_list_request_jumps_to_the_approach_row_first() {
        let mtm = mtm();
        let controller = ScrollController::new(0usize);
        let (leaf, table) = row_list(
            vec![row_item as fn() -> ListItem; APPROACH_LIST_ROWS],
            &controller,
        );
        let window = scene_window(mtm, leaf.view());
        table.layout_if_needed();

        assert_native_scroll_from(
            "a far Default list request",
            || controller.animate_to(APPROACH_TARGET_ROW, Animation::Default),
            || list_row_top(&table, APPROACH_ROW),
            || Point::from(table.contentOffset()),
            || list_row_top(&table, APPROACH_TARGET_ROW),
        );
        window.setHidden(true);
    }
}
