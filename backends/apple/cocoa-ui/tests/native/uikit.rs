//! `UIKit` cases — real `UIKit` objects built through the public API on the
//! real main thread.

use std::cell::Cell;
use std::rc::Rc;

use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::Bool;
use cocoa_ui::objc2::{MainThreadMarker, MainThreadOnly, msg_send, sel};
use cocoa_ui::objc2_core_foundation::{CGPoint, CGRect, CGSize};
use cocoa_ui::objc2_ui_kit::{UIView, UIWindow};
use cocoa_ui::uikit::input_view::InputView;
use cocoa_ui::uikit::{HostView, Label};
use cocoa_ui::{PlatformView, Rect};
use libtest_mimic::Trial;

use crate::harness::marker;

/// The suite's `UIKit` cases, named after the module and test they moved
/// from.
pub fn trials() -> Vec<Trial> {
    macro_rules! case {
        ($module:literal, $test:ident) => {
            Trial::test(concat!($module, "::", stringify!($test)), || {
                $test();
                Ok(())
            })
        };
    }
    crate::harness::trials(vec![
        case!(
            "uikit::colors",
            extended_linear_display_p3_preserves_space_and_hdr_channels
        ),
        case!(
            "uikit::host_view",
            a_child_invalidation_inside_layout_does_not_reenter_the_pass
        ),
        case!(
            "uikit::host_view",
            a_host_view_in_a_window_reports_its_window
        ),
        case!(
            "uikit::input_view",
            the_text_input_selectors_register_under_uikits_names
        ),
        case!(
            "uikit::context_menu",
            an_overlay_rooted_on_the_platter_claims_only_accessory_hits
        ),
        case!("view", display_immediately_targets_the_layer_not_the_view),
    ])
}

/// The extended linear Display-P3 constructor must land in that space —
/// not extended linear sRGB — and pass HDR channels straight through
/// instead of clamping or baking headroom away.
fn extended_linear_display_p3_preserves_space_and_hdr_channels() {
    use cocoa_ui::objc2_core_graphics::{
        CGColor, CGColorSpace, kCGColorSpaceExtendedLinearDisplayP3,
    };
    use cocoa_ui::uikit::colors;

    let color = colors::extended_linear_display_p3(1.5, 0.25, 0.5, 0.8);
    // SAFETY: `CGColor()` is UIColor's plain accessor; the color is a live
    // `UIColor` created above.
    let cg = unsafe { color.CGColor() };
    let space = CGColor::color_space(Some(&cg)).expect("an RGB CGColor has a color space");
    let name = CGColorSpace::name(Some(&space)).expect("a named color space reports its name");
    // SAFETY: the static is a `CFString` constant exported by Core Graphics.
    let expected = unsafe { kCGColorSpaceExtendedLinearDisplayP3 }.to_string();
    assert_eq!(name.to_string(), expected);
    assert_eq!(CGColor::number_of_components(Some(&cg)), 4);
    // SAFETY: `components` is valid for `number_of_components` entries.
    let channels = unsafe { std::slice::from_raw_parts(CGColor::components(Some(&cg)), 4) };
    assert!((channels[0] - 1.5).abs() < f64::EPSILON);
    assert!((channels[3] - 0.8).abs() < f64::EPSILON);
}

/// A `UIWindow` with no scene hosting `content`, kept hidden — the smallest
/// host in which `layoutSubviews` and `window`-dependent paths see a real
/// window instead of `nil`.
fn attach(mtm: MainThreadMarker, content: &PlatformView) -> Retained<UIWindow> {
    // SAFETY: `initWithFrame:` is `UIWindow`'s plain initializer; `mtm` is
    // the real main thread.
    let window: Retained<UIWindow> = unsafe {
        msg_send![
            UIWindow::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(390.0, 844.0))
        ]
    };
    window.addSubview(content);
    window
}

/// Same defect as the `AppKit` `layout` loop: a label whose width changes
/// inside `layoutSubviews` must mark ancestors dirty, not synchronously
/// re-enter the parent's pass.
fn a_child_invalidation_inside_layout_does_not_reenter_the_pass() {
    let mtm = marker();
    let host = HostView::new(mtm, Rect::new(0.0, 0.0, 400.0, 300.0));
    let label = Label::new(mtm);
    host.add_subview(&label);
    let _window = attach(mtm, &host);

    let calls = Rc::new(Cell::new(0));
    host.set_layout_handler({
        let calls = Rc::clone(&calls);
        move |_view| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                let frame = label.frame();
                cocoa_ui::view::set_frame(
                    &label,
                    Rect::new(
                        frame.origin.x,
                        frame.origin.y,
                        frame.size.width + 20.0,
                        frame.size.height,
                    ),
                );
            }
        }
    });

    host.set_needs_layout();
    host.layout_if_needed();
    // The second pass is itself the flush the mid-pass invalidation waits
    // on: `layoutIfNeeded` is the platform's synchronous layout API — no
    // run-loop wait stands in for it.
    host.layout_if_needed();

    assert!(
        calls.get() <= 4,
        "layout re-entered {} times — invalidation must mark, not recurse",
        calls.get(),
    );
    assert!(calls.get() >= 1);
}

/// A `UIWindow` hosting a `HostView`: `view::window`/`has_window` must
/// forward to the real `window` property and report the live window.
fn a_host_view_in_a_window_reports_its_window() {
    let mtm = marker();
    let host = HostView::new(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
    let window = attach(mtm, &host);
    assert!(cocoa_ui::view::has_window(&host));
    let reported = cocoa_ui::view::window(&host).expect("a hosted view reports its window");
    assert!(std::ptr::eq(&raw const *reported, &raw const *window));
}

/// The `#[unsafe(method(..))]` names must install the `ObjC` selectors
/// `UIKit`'s text-input machinery calls, not a `snake_case` spelling.
/// Assert the `UITextInput`/`UIResponder` selectors resolve on instances.
fn the_text_input_selectors_register_under_uikits_names() {
    let mtm = marker();
    let view = InputView::new(mtm);
    for selector in [
        sel!(insertText:),
        sel!(deleteBackward),
        sel!(hasText),
        sel!(setMarkedText:selectedRange:),
        sel!(unmarkText),
        sel!(selectedTextRange),
        sel!(markedTextRange),
        sel!(firstRectForRange:),
        sel!(caretRectForPosition:),
        sel!(closestPositionToPoint:),
        sel!(characterRangeAtPoint:),
        sel!(textInRange:),
        sel!(replaceRange:withText:),
        sel!(positionFromPosition:offset:),
        sel!(canBecomeFirstResponder),
        sel!(becomeFirstResponder),
        sel!(touchesBegan:withEvent:),
        sel!(pressesBegan:withEvent:),
    ] {
        // SAFETY: `respondsToSelector:` is a plain `NSObject` query on the
        // live view.
        let responds: Bool = unsafe { msg_send![&*view, respondsToSelector: selector] };
        assert!(
            responds.as_bool(),
            "CocoaUiInputView must respond to {}",
            selector.name().to_string_lossy(),
        );
    }
}

/// The accessory overlay's window must claim hits only inside the
/// accessory: `hitTest` returns nil outside it (letting the host window's
/// menu and backdrop see the touch) and the real accessory descendant
/// inside it. `AccessoryOverlay::present` needs a `UIWindowScene` this
/// standalone suite lacks, so the case builds the window tree `present`
/// builds — an `AccessoryOverlayWindow` whose root view is the
/// `PassIfSelf` platter — and asserts the real window hit test, plus the
/// pre-fix shape (a plain root `UIView` containing the platter) which
/// re-claims those points.
fn an_overlay_rooted_on_the_platter_claims_only_accessory_hits() {
    use cocoa_ui::objc2::rc::Allocated;
    use cocoa_ui::objc2::runtime::AnyClass;
    use cocoa_ui::objc2_ui_kit::{UIEvent, UIViewController};
    use cocoa_ui::uikit::{AccessoryOverlay, HitTest};

    let mtm = marker();
    // The subclass registers lazily on the first `present` call; this
    // detached source has no window, so `present` registers the class and
    // returns `None`.
    let detached = HostView::new(mtm, Rect::ZERO);
    assert!(
        AccessoryOverlay::present(&detached, &detached, Rect::ZERO, || {
            cocoa_ui::Size::new(0.0, 0.0)
        })
        .is_none()
    );
    // The registered class the overlay presents: private to
    // `cocoa_ui::uikit::context_menu`, reached here through the
    // Objective-C runtime rather than a construction API.
    let overlay_class = AnyClass::get(c"CocoaUiAccessoryOverlayWindow")
        .expect("overlay window class is registered");
    // SAFETY: `alloc`/`initWithFrame:` are `UIWindow`'s plain
    // initializers on the real main thread.
    let allocated: Allocated<UIWindow> = unsafe { msg_send![overlay_class, alloc] };
    let window: Retained<UIWindow> = unsafe {
        msg_send![
            allocated,
            initWithFrame: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(390.0, 844.0))
        ]
    };
    let controller = UIViewController::new(mtm);
    window.setRootViewController(Some(&controller));

    let platter = HostView::new(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
    platter.set_hit_test_handler(|_view, _point| HitTest::PassIfSelf);
    let accessory = HostView::new(mtm, Rect::new(100.0, 100.0, 120.0, 40.0));
    let child = HostView::new(mtm, Rect::new(0.0, 0.0, 60.0, 40.0));
    accessory.add_subview(&child);
    platter.add_subview(&accessory);
    controller.setView(Some(&platter));
    // `UIWindow` only attaches the root controller's view once the window
    // is unhidden — `AccessoryOverlay::present` does the same.
    window.setHidden(false);

    // SAFETY: `hitTest:` takes a nil event on the main thread.
    let outside: Option<Retained<UIView>> = unsafe {
        msg_send![&*window, hitTest: CGPoint::new(20.0, 20.0), withEvent: std::ptr::null::<UIEvent>()]
    };
    // No content claims the point: the window maps its own self-hit to nil
    // so the scene passes the touch to the host window below.
    assert!(
        outside.is_none(),
        "a hit outside the accessory must not be claimed: {outside:?}"
    );
    // SAFETY: `hitTest:` takes a nil event on the main thread.
    let inside: Option<Retained<UIView>> = unsafe {
        msg_send![&*window, hitTest: CGPoint::new(110.0, 110.0), withEvent: std::ptr::null::<UIEvent>()]
    };
    assert!(
        inside
            .as_deref()
            .is_some_and(|hit| std::ptr::eq(hit, &raw const **child)),
        "a hit inside the accessory must resolve to its real descendant"
    );

    // The pre-fix shape — a plain `UIWindow` whose root view is a plain
    // `UIView` containing the platter — claims the same point: the old
    // overlay swallowed every touch outside the accessory.
    // SAFETY: `alloc`/`initWithFrame:` are `UIWindow`'s plain
    // initializers on the real main thread.
    let old_window: Retained<UIWindow> = unsafe {
        msg_send![
            UIWindow::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(390.0, 844.0))
        ]
    };
    let old_controller = UIViewController::new(mtm);
    old_window.setRootViewController(Some(&old_controller));
    // SAFETY: `alloc`/`initWithFrame:` are `UIView`'s plain initializers.
    let plain_root: Retained<UIView> =
        unsafe { msg_send![UIView::alloc(mtm), initWithFrame: old_window.bounds()] };
    let plain_platter = HostView::new(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
    plain_platter.set_hit_test_handler(|_view, _point| HitTest::PassIfSelf);
    plain_root.addSubview(&plain_platter);
    old_controller.setView(Some(&plain_root));
    old_window.setHidden(false);
    // SAFETY: `hitTest:` takes a nil event on the main thread.
    let swallowed: Option<Retained<UIView>> = unsafe {
        msg_send![&*old_window, hitTest: CGPoint::new(20.0, 20.0), withEvent: std::ptr::null::<UIEvent>()]
    };
    assert!(
        swallowed.is_some(),
        "a plain window and root view claim pass-through hits"
    );
}

/// Regression test for the `displayIfNeeded` defect: `UIView` has no
/// `displayIfNeeded` — only `CALayer` does — and sending it to the view
/// aborts on an unrecognized selector. `display_immediately` must send
/// `setNeedsDisplay` to the view and `displayIfNeeded` to its layer.
fn display_immediately_targets_the_layer_not_the_view() {
    let mtm = marker();
    // SAFETY: `initWithFrame:` is `UIView`'s plain initializer; `mtm` is
    // the real main thread.
    let view: Retained<UIView> =
        unsafe { msg_send![UIView::alloc(mtm), initWithFrame: CGRect::ZERO] };
    // The premise the pre-fix code missed: the selector does not exist on
    // `UIView` itself.
    // SAFETY: `respondsToSelector:` is a plain `NSObject` query.
    let view_responds: Bool =
        unsafe { msg_send![&*view, respondsToSelector: sel!(displayIfNeeded)] };
    assert!(!view_responds.as_bool());
    let layer = view.layer();
    // SAFETY: `respondsToSelector:` is a plain `NSObject` query.
    let layer_responds: Bool =
        unsafe { msg_send![&*layer, respondsToSelector: sel!(displayIfNeeded)] };
    assert!(layer_responds.as_bool());
    // Would abort on the unrecognized selector had it been sent to the view.
    cocoa_ui::view::display_immediately(&view);
}
