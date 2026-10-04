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
            "uikit::menu_panel",
            a_presented_panel_tracks_its_rows_pages_and_focus
        ),
        case!(
            "uikit::menu_panel",
            a_presenter_driven_dismissal_ends_the_panel
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

/// The popover panel's command surface: rows build from the menu tree —
/// separators stay non-activatable, focus moves across enabled buttons
/// and skips them — a submenu row pushes a page with a back row, Left
/// pops it, and activating a command runs its action.
fn a_presented_panel_tracks_its_rows_pages_and_focus() {
    use cocoa_ui::menu::{Command, MenuTreeNode};
    use cocoa_ui::objc2::rc::Weak;
    use cocoa_ui::objc2_ui_kit::UIColor;
    use cocoa_ui::uikit::{ContextMenuPopover, PanelPalette};

    let mtm = marker();
    let palette = PanelPalette {
        label: UIColor::labelColor(),
        muted: UIColor::secondaryLabelColor(),
        destructive: UIColor::systemRedColor(),
        separator: UIColor::separatorColor(),
        focus_fill: UIColor::tertiarySystemFillColor(),
        surface: UIColor::systemBackgroundColor(),
    };
    let popover = ContextMenuPopover::new(mtm, &palette);
    let picked = Rc::new(Cell::new(0));
    let nodes = vec![
        MenuTreeNode::Command(
            Command {
                label: String::from("Copy"),
                enabled: true,
                ..Command::default()
            },
            {
                let picked = Rc::clone(&picked);
                Rc::new(move || picked.set(picked.get() + 1))
            },
        ),
        MenuTreeNode::Command(
            Command {
                label: String::from("Share"),
                subtitle: Some(String::from("Sends a link")),
                enabled: true,
                ..Command::default()
            },
            Rc::new(|| {}),
        ),
        MenuTreeNode::Divider,
        MenuTreeNode::Submenu(
            Command {
                label: String::from("More"),
                enabled: true,
                ..Command::default()
            },
            vec![MenuTreeNode::Command(
                Command {
                    label: String::from("Delete"),
                    destructive: true,
                    enabled: true,
                    ..Command::default()
                },
                {
                    let picked = Rc::clone(&picked);
                    Rc::new(move || picked.set(picked.get() + 1))
                },
            )],
        ),
    ];
    popover.set_commands(&nodes);

    let (depth, rows, focused) = popover.page_probe_for_test();
    assert_eq!(depth, 1);
    assert_eq!(
        rows.iter()
            .map(|(label, activatable)| (label.as_str(), *activatable))
            .collect::<Vec<_>>(),
        [("Copy", true), ("Share", true), ("", false), ("More", true)]
    );
    assert_eq!(focused, None);

    // Focus walks the enabled rows and the separator never lands.
    popover.press_key_for_test("down");
    assert_eq!(popover.page_probe_for_test().2, Some(0));
    popover.press_key_for_test("down");
    assert_eq!(popover.page_probe_for_test().2, Some(1));
    popover.press_key_for_test("down");
    assert_eq!(popover.page_probe_for_test().2, Some(2));

    // Activating the submenu row pushes its page under a back row titled
    // by the parent.
    popover.press_key_for_test("return");
    let (depth, rows, _) = popover.page_probe_for_test();
    assert_eq!(depth, 2);
    assert_eq!(rows[0].0, "More");
    assert_eq!(rows[1].0, "Delete");

    // Focus lands on the back row, then Delete; activation runs the
    // action — a command also dismisses when presented.
    popover.press_key_for_test("down");
    popover.press_key_for_test("down");
    popover.press_key_for_test("return");
    assert_eq!(picked.get(), 1);

    // Left returns to the root page; Escape dismisses harmlessly even
    // unpresented.
    popover.press_key_for_test("left");
    assert_eq!(popover.page_probe_for_test().0, 1);
    popover.press_key_for_test("escape");

    // Dropping the popover releases its controller — nothing else retains
    // it, so an unpresented panel tears the whole native tree down. The
    // mount-target view lives in the controller's ivars, so it is the
    // witness for the controller itself.
    let weak = Weak::new(&*popover.mount_target());
    drop(popover);
    assert!(weak.load().is_none());
}

/// A dismissal the presenter drives — `dismissViewController` on the
/// presenting controller — ends the panel through the controller's own
/// view lifecycle, not through `dismiss` or the adaptive delegate:
/// `UIKit` does not call `presentationControllerDidDismiss` for a
/// programmatic dismissal. The backstop in `viewDidDisappear` runs the
/// same once-only teardown and the panel presents again afterwards.
fn a_presenter_driven_dismissal_ends_the_panel() {
    use cocoa_ui::menu::{Command, MenuTreeNode};
    use cocoa_ui::objc2_foundation::{NSDate, NSRunLoop};
    use cocoa_ui::objc2_ui_kit::{UIColor, UIViewController};
    use cocoa_ui::uikit::{ContextMenuPopover, PanelPalette};

    let mtm = marker();
    let palette = PanelPalette {
        label: UIColor::labelColor(),
        muted: UIColor::secondaryLabelColor(),
        destructive: UIColor::systemRedColor(),
        separator: UIColor::separatorColor(),
        focus_fill: UIColor::tertiarySystemFillColor(),
        surface: UIColor::systemBackgroundColor(),
    };

    let root = UIViewController::new(mtm);
    // SAFETY: `initWithFrame:` is `UIWindow`'s plain initializer; `mtm`
    // is the real main thread.
    let window: Retained<UIWindow> = unsafe {
        msg_send![
            UIWindow::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(390.0, 844.0))
        ]
    };
    window.setRootViewController(Some(&root));
    window.makeKeyAndVisible();
    let host = HostView::new(mtm, Rect::new(0.0, 0.0, 100.0, 44.0));
    root.view()
        .expect("a plain controller loads its view on demand")
        .addSubview(&host);

    // A bounded main-runloop pump: presentation and dismissal settle on
    // later cycles.
    let pump = || {
        NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
    };
    let settle = |done: &dyn Fn() -> bool| {
        for _ in 0..40 {
            pump();
            if done() {
                break;
            }
        }
    };

    let popover = ContextMenuPopover::new(mtm, &palette);
    popover.set_commands(&[MenuTreeNode::Command(
        Command {
            label: String::from("Copy"),
            enabled: true,
            ..Command::default()
        },
        Rc::new(|| {}),
    )]);

    let dismissed = Rc::new(Cell::new(0));
    assert!(popover.present(&host, {
        let dismissed = Rc::clone(&dismissed);
        Rc::new(move || dismissed.set(dismissed.get() + 1))
    }));
    settle(&|| root.presentedViewController().is_some());
    assert!(
        root.presentedViewController().is_some(),
        "the panel presented under the window's root controller"
    );

    // The presenter dismisses with no completion and no outside tap:
    // only the panel's own lifecycle can carry the teardown.
    root.dismissViewControllerAnimated_completion(false, None);
    settle(&|| dismissed.get() > 0);
    assert_eq!(dismissed.get(), 1);
    assert!(root.presentedViewController().is_none());

    // The teardown released the panel: it presents again, and the next
    // presenter-driven dismissal runs its own teardown — the first
    // callback does not fire twice.
    let dismissed_again = Rc::new(Cell::new(0));
    assert!(popover.present(&host, {
        let dismissed_again = Rc::clone(&dismissed_again);
        Rc::new(move || dismissed_again.set(dismissed_again.get() + 1))
    }));
    settle(&|| root.presentedViewController().is_some());
    assert!(root.presentedViewController().is_some());
    root.dismissViewControllerAnimated_completion(false, None);
    settle(&|| dismissed_again.get() > 0);
    assert_eq!(dismissed_again.get(), 1);
    assert_eq!(dismissed.get(), 1);
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
