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

use crate::harness::{capture_target, marker, nonzero_texels};

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
            an_unpresented_panel_tracks_its_rows_pages_and_focus
        ),
        case!("view", display_immediately_targets_the_layer_not_the_view),
        case!(
            "capture",
            a_capture_claim_restores_containment_and_survives_release
        ),
        case!(
            "capture",
            a_detached_capture_renders_and_teardown_stays_clean
        ),
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

/// The popover panel's command surface — unpresented here, so only its
/// composition is covered: rows build from the menu tree —
/// separators stay non-activatable, focus moves across enabled buttons
/// and skips them — a submenu row pushes a page with a back row, Left
/// pops it, and activating a command runs its action.
fn an_unpresented_panel_tracks_its_rows_pages_and_focus() {
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

/// The `CARenderer` root claim is scoped to the frame it encodes: after
/// `ViewCapture::capture` the view reports the same parent, sibling
/// order, bounds, center, transform, and hidden flag as before — and it
/// keeps answering them after the cached renderer is released on a
/// later main-queue turn.
fn a_capture_claim_restores_containment_and_survives_release() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use cocoa_ui::capture::ViewCapture;
    use cocoa_ui::objc2_foundation::NSString;
    use cocoa_ui::objc2_quartz_core::CATransform3D;
    use cocoa_ui::objc2_ui_kit::{UIColor, UILabel};

    let mtm = marker();
    // SAFETY: `initWithFrame:` is `UIView`'s plain initializer; `mtm` is
    // the real main thread — same for `UILabel` below.
    let parent: Retained<UIView> = unsafe {
        msg_send![
            UIView::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(400.0, 400.0))
        ]
    };
    let content: Retained<UIView> = unsafe {
        msg_send![
            UIView::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(8.0, 40.0), CGSize::new(200.0, 200.0))
        ]
    };
    content.setBackgroundColor(Some(&UIColor::systemBlueColor()));
    // A perspective transform is the flattening case: an affine-only
    // `UIView.transform` restore would zero `m34`.
    let mut perspective = CATransform3D::new_rotation(0.3, 0.0, 1.0, 0.0);
    perspective.m34 = -1.0 / 600.0;
    content.layer().setTransform(perspective);
    let label: Retained<UILabel> = unsafe {
        msg_send![
            UILabel::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(4.0, 30.0), CGSize::new(150.0, 24.0))
        ]
    };
    label.setText(Some(&NSString::from_str("capture-claim")));
    content.addSubview(&label);
    parent.addSubview(&content);
    let sibling: Retained<UIView> = unsafe {
        msg_send![
            UIView::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(8.0, 280.0), CGSize::new(40.0, 40.0))
        ]
    };
    parent.addSubview(&sibling);
    let _window = attach(mtm, &parent);

    let bounds = content.bounds();
    let center = content.center();
    let layer_transform = content.layer().transform();

    let Some(target) = capture_target() else {
        return; // No Metal on this runner — nothing to check.
    };
    let capture = Rc::new(ViewCapture::new(mtm, content.clone(), |_| None));
    capture.set_on_redraw(|| {});
    let done = Arc::new(AtomicBool::new(false));
    capture.capture(&target, {
        let done = Arc::clone(&done);
        move |ok| done.store(ok, Ordering::Relaxed)
    });

    // The restore happens inside `capture`: by the time it returns the
    // claim must already be gone — parent, order, and geometry answer
    // their recorded values synchronously.
    let restored_parent = content
        .superview()
        .expect("the claim left the view detached from its parent");
    assert!(std::ptr::eq(
        &raw const *restored_parent,
        &raw const *parent
    ));
    let order = parent.subviews();
    assert_eq!(order.count(), 2);
    assert!(std::ptr::eq(
        &raw const *order.objectAtIndex(0),
        std::ptr::from_ref(&*content).cast()
    ));
    assert!(std::ptr::eq(
        &raw const *order.objectAtIndex(1),
        std::ptr::from_ref(&*sibling).cast()
    ));
    assert_eq!(content.bounds(), bounds);
    assert_eq!(content.center(), center);
    assert!(
        content
            .layer()
            .transform()
            .equal_to_transform(layer_transform),
        "the claim must preserve the layer's full transform, perspective included"
    );
    assert!(!content.isHidden());
    let hit = parent.hitTest_withEvent(CGPoint::new(center.x + 20.0, center.y), None);
    assert!(
        hit.is_some_and(|hit| std::ptr::eq(
            std::ptr::from_ref(&*hit).cast::<UIView>(),
            &raw const *content
        ) || std::ptr::eq(
            std::ptr::from_ref(&*hit).cast::<UILabel>(),
            &raw const *label
        )),
        "hit testing must reach the restored subtree"
    );

    assert!(
        crate::harness::pump_main_until(5.0, || done.load(Ordering::Relaxed)),
        "the capture fence never completed"
    );
    let texels = nonzero_texels(&target);
    assert!(
        texels > 10_000,
        "the claim must render real content, not an empty frame (nonzero texels: {texels})"
    );

    // Releasing the cached renderer must not invalidate the layer it
    // claimed: on the next main-queue turn the same layer answers, still
    // attached, and the tree tears down normally.
    capture.shutdown();
    drop(capture);
    crate::harness::pump_main_turn();
    let still_attached = content
        .superview()
        .expect("renderer release re-severed the view");
    assert!(std::ptr::eq(&raw const *still_attached, &raw const *parent));
    assert!(
        content.layer().superlayer().is_some(),
        "the backing layer lost its parent when the renderer dropped"
    );
    assert_eq!(content.bounds(), bounds);
    assert!(
        content
            .layer()
            .transform()
            .equal_to_transform(layer_transform),
        "renderer release must not disturb the restored transform"
    );

    content.removeFromSuperview();
    sibling.removeFromSuperview();
    parent.removeFromSuperview();
}

/// Capturing a view with no superview is supported — the claim restores
/// nothing, the frame still renders real content, and teardown after
/// the cached renderer's release stays clean.
fn a_detached_capture_renders_and_teardown_stays_clean() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use cocoa_ui::capture::ViewCapture;
    use cocoa_ui::objc2_foundation::NSString;
    use cocoa_ui::objc2_ui_kit::{UIColor, UILabel};

    let mtm = marker();
    // SAFETY: `initWithFrame:` is `UIView`'s plain initializer; `mtm` is
    // the real main thread — same for `UILabel` below.
    let content: Retained<UIView> = unsafe {
        msg_send![
            UIView::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(200.0, 200.0))
        ]
    };
    content.setBackgroundColor(Some(&UIColor::systemOrangeColor()));
    let label: Retained<UILabel> = unsafe {
        msg_send![
            UILabel::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(4.0, 30.0), CGSize::new(150.0, 24.0))
        ]
    };
    label.setText(Some(&NSString::from_str("detached")));
    content.addSubview(&label);
    assert!(content.superview().is_none());

    let Some(target) = capture_target() else {
        return; // No Metal on this runner — nothing to check.
    };
    let capture = Rc::new(ViewCapture::new(mtm, content.clone(), |_| None));
    capture.set_on_redraw(|| {});
    let done = Arc::new(AtomicBool::new(false));
    capture.capture(&target, {
        let done = Arc::clone(&done);
        move |ok| done.store(ok, Ordering::Relaxed)
    });
    assert!(
        content.superview().is_none(),
        "a detached capture must not invent a parent"
    );

    assert!(
        crate::harness::pump_main_until(5.0, || done.load(Ordering::Relaxed)),
        "the capture fence never completed"
    );
    let texels = nonzero_texels(&target);
    assert!(
        texels > 10_000,
        "a detached capture must still render real content (nonzero texels: {texels})"
    );

    capture.shutdown();
    drop(capture);
    crate::harness::pump_main_turn();
    let _layer = content.layer(); // crashes on an invalidated layer
    label.removeFromSuperview();
}
