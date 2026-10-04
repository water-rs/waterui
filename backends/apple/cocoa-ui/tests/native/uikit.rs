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

use crate::harness::{capture_target, marker};

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
            "uikit::visibility",
            a_windowless_or_sceneless_view_is_not_presentable
        ),
        case!(
            "uikit::visibility",
            ancestor_emissions_reach_a_descendants_subscribed_wake
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

/// The scene-level gate (#1327): `presentable` follows the owning
/// `UIWindowScene`'s activation, so a view with no window — and a view in
/// a `UIWindow` that was never attached to a scene, as this harness
/// builds it — both answer `false`. Positive geometry paths need a live
/// foreground scene and are exercised by the component consumers.
fn a_windowless_or_sceneless_view_is_not_presentable() {
    let mtm = marker();
    let host = HostView::new(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
    let child = HostView::new(mtm, Rect::new(0.0, 0.0, 100.0, 50.0));
    host.add_subview(&child);

    assert!(!cocoa_ui::visibility::presentable(&child));

    let _window = attach(mtm, &host);
    // A `UIWindow` with `windowScene == nil` cannot present — the same
    // answer an unattached or backgrounded scene gives.
    assert!(!cocoa_ui::visibility::presentable(&child));
}

/// The typed-owned wake on `UIKit`: `VisibilityWatch` binds one closure
/// on every emitting `CocoaUi` ancestor, hidden→visible transitions keep
/// firing, a reparent's `refresh` detaches the former chain's tokens,
/// and a dropped watch stops delivery — no registry, no polling.
fn ancestor_emissions_reach_a_descendants_subscribed_wake() {
    let mtm = marker();
    let host = HostView::new(mtm, Rect::new(0.0, 0.0, 390.0, 844.0));
    let child = HostView::new(mtm, Rect::new(0.0, 0.0, 100.0, 50.0));
    host.add_subview(&child);
    let _window = attach(mtm, &host);

    let fires = Rc::new(Cell::new(0));
    let wake: Rc<dyn Fn()> = Rc::new({
        let fires = Rc::clone(&fires);
        move || fires.set(fires.get() + 1)
    });
    let watch = cocoa_ui::visibility::VisibilityWatch::new(&child, wake);
    let before = fires.get();
    cocoa_ui::view::set_hidden(&host, true);
    assert_eq!(fires.get() - before, 1);
    let before = fires.get();
    cocoa_ui::view::set_hidden(&host, false);
    assert_eq!(fires.get() - before, 1);

    // A reparent emits its own `didMoveToSuperview`/`didMoveToWindow`
    // wakes on the child, so counts are asserted as deltas. Reparenting
    // onto a detached sibling removes `host` from the chain: the refresh
    // binds only `other`, and an emission on `host` can no longer reach
    // the handler.
    cocoa_ui::view::remove_from_superview(&child);
    let other = HostView::new(mtm, Rect::new(0.0, 0.0, 100.0, 50.0));
    other.add_subview(&child);
    watch.refresh(&child);
    let before = fires.get();
    other.visibility_emitter().emit();
    assert_eq!(fires.get() - before, 1);
    let before = fires.get();
    host.visibility_emitter().emit();
    assert_eq!(
        fires.get() - before,
        0,
        "a detached ancestor still delivered a wake"
    );

    drop(watch);
    let before = fires.get();
    cocoa_ui::view::set_hidden(&host, true);
    other.visibility_emitter().emit();
    assert_eq!(
        fires.get() - before,
        0,
        "a dropped watch kept receiving wakes"
    );
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

/// The `CARenderer` root claim is scoped to the frame it encodes: two
/// cached `ViewCapture`s bound to nested views run child→parent claim
/// cycles, and after each the views report the same parent, sibling
/// order, bounds, center, transform, and hidden flag as before — and
/// they keep answering them after both cached renderers are released
/// on a later main-queue turn. This is the nested-claim shape the
/// production ownership fix exists for.
#[allow(clippy::too_many_lines)] // The nested fixture plus the containment contract it checks.
fn a_capture_claim_restores_containment_and_survives_release() {
    use std::rc::Rc;

    use crate::harness::{await_fence, count_pixels, fence_flag, readback};
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
    // SAFETY: `initWithFrame:` initializes this test-owned view on the main thread.
    let content: Retained<UIView> = unsafe {
        msg_send![
            UIView::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(8.0, 40.0), CGSize::new(200.0, 200.0))
        ]
    };
    content.setBackgroundColor(Some(&UIColor::blueColor()));
    // A perspective transform is the flattening case: an affine-only
    // `UIView.transform` restore would zero `m34`.
    let mut perspective = CATransform3D::new_rotation(0.3, 0.0, 1.0, 0.0);
    perspective.m34 = -1.0 / 600.0;
    content.layer().setTransform(perspective);
    // SAFETY: `initWithFrame:` is `UILabel`'s plain initializer; `mtm` is
    // a test-owned allocation on the main thread.
    let label: Retained<UILabel> = unsafe {
        msg_send![
            UILabel::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(4.0, 30.0), CGSize::new(150.0, 24.0))
        ]
    };
    label.setText(Some(&NSString::from_str("capture-claim")));
    // SAFETY: a test-owned label on the main thread; white is a valid color.
    unsafe { label.setTextColor(Some(&UIColor::whiteColor())) };
    content.addSubview(&label);
    // The nested claim: a second capturable subtree inside `content`,
    // with its own color and glyphs.
    // SAFETY: `initWithFrame:` is `UIView`'s plain initializer; `mtm` is
    // a test-owned allocation on the main thread.
    let child: Retained<UIView> = unsafe {
        msg_send![
            UIView::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(20.0, 60.0), CGSize::new(120.0, 120.0))
        ]
    };
    child.setBackgroundColor(Some(&UIColor::orangeColor()));
    // SAFETY: `initWithFrame:` is `UILabel`'s plain initializer; `mtm` is
    // a test-owned allocation on the main thread.
    let child_label: Retained<UILabel> = unsafe {
        msg_send![
            UILabel::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(4.0, 30.0), CGSize::new(100.0, 24.0))
        ]
    };
    child_label.setText(Some(&NSString::from_str("nested")));
    // SAFETY: a test-owned label on the main thread; white is a valid color.
    unsafe { child_label.setTextColor(Some(&UIColor::whiteColor())) };
    child.addSubview(&child_label);
    content.addSubview(&child);
    parent.addSubview(&content);
    // SAFETY: `initWithFrame:` is `UIView`'s plain initializer; `mtm` is
    // a test-owned allocation on the main thread.
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
    let child_bounds = child.bounds();

    let target = capture_target();
    let content_capture = Rc::new(ViewCapture::new(mtm, content.clone(), |_| None));
    content_capture.set_on_redraw(|| {});
    let child_capture = Rc::new(ViewCapture::new(mtm, child.clone(), |_| None));
    child_capture.set_on_redraw(|| {});

    // Everything the claim owes the tree, checked after every cycle and
    // again after both cached renderers are released.
    let check = || {
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
        let child_parent = child
            .superview()
            .expect("the nested claim left the child detached");
        assert!(std::ptr::eq(&raw const *child_parent, &raw const *content));
        let content_order = content.subviews();
        assert_eq!(content_order.count(), 2);
        assert!(std::ptr::eq(
            &raw const *content_order.objectAtIndex(0),
            std::ptr::from_ref(&*label).cast()
        ));
        assert!(std::ptr::eq(
            &raw const *content_order.objectAtIndex(1),
            std::ptr::from_ref(&*child).cast()
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
        assert_eq!(child.bounds(), child_bounds);
        assert!(!content.isHidden());
        let hit = parent.hitTest_withEvent(CGPoint::new(center.x + 20.0, center.y), None);
        assert!(
            hit.is_some_and(|hit| {
                let hit = std::ptr::from_ref(&*hit).cast::<UIView>();
                std::ptr::eq(hit, &raw const *content)
                    || std::ptr::eq(hit, &raw const *child)
                    || std::ptr::eq(hit, std::ptr::from_ref(&*label).cast::<UIView>())
                    || std::ptr::eq(hit, std::ptr::from_ref(&*child_label).cast::<UIView>())
            }),
            "hit testing must reach the restored subtree"
        );
    };

    // Three nested cycles: child claim first (the deepest claim), then
    // the enclosing content claim — each against the same cached
    // renderers, each proven by its own completed, successful fence.
    for _cycle in 0..3 {
        let (flag, complete) = fence_flag();
        child_capture.capture(&target, complete);
        await_fence(&flag, "child");
        // The child frame carried the child's own colors and glyphs.
        let texels = readback(&target);
        assert!(
            count_pixels(&texels, [0, 110, 220, 255], [60, 200, 255, 255]) > 1_000,
            "the child claim must render the child's color"
        );
        assert!(
            count_pixels(&texels, [220, 220, 220, 255], [255, 255, 255, 255]) > 20,
            "the child claim must render real label glyphs"
        );

        let (flag, complete) = fence_flag();
        content_capture.capture(&target, complete);
        await_fence(&flag, "content");
        let texels = readback(&target);
        assert!(
            count_pixels(&texels, [200, 0, 0, 255], [255, 60, 60, 255]) > 5_000,
            "the content claim must render the content's color"
        );
        assert!(
            count_pixels(&texels, [0, 110, 220, 255], [60, 200, 255, 255]) > 1_000,
            "the content claim must render the nested child's color"
        );
        assert!(
            count_pixels(&texels, [220, 220, 220, 255], [255, 255, 255, 255]) > 20,
            "the content claim must render real label glyphs"
        );

        check();
    }

    // Releasing the cached renderers must not invalidate the layers
    // they claimed: on the next real main-queue turn both trees answer,
    // still attached, and tear down normally.
    content_capture.shutdown();
    child_capture.shutdown();
    drop(content_capture);
    drop(child_capture);
    crate::harness::pump_main_turn();
    check();
    assert!(
        content.layer().superlayer().is_some(),
        "the backing layer lost its parent when the renderer dropped"
    );
    assert!(
        child.layer().superlayer().is_some(),
        "the nested backing layer lost its parent when the renderer dropped"
    );

    content.removeFromSuperview();
    sibling.removeFromSuperview();
    parent.removeFromSuperview();
}

/// Capturing a view with no superview is supported — the claim restores
/// nothing, the frame still renders real content, and teardown after
/// the cached renderer's release stays clean.
fn a_detached_capture_renders_and_teardown_stays_clean() {
    use std::rc::Rc;

    use cocoa_ui::capture::ViewCapture;
    use cocoa_ui::objc2_foundation::NSString;
    use cocoa_ui::objc2_ui_kit::{UIColor, UILabel};

    let mtm = marker();
    // SAFETY: `initWithFrame:` is `UIView`'s plain initializer; `mtm` is
    // the real main thread — same for `UILabel` below.
    // SAFETY: `initWithFrame:` is `UIView`'s plain initializer; `mtm` is
    let content: Retained<UIView> = unsafe {
        msg_send![
            UIView::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(200.0, 200.0))
        ]
    };
    content.setBackgroundColor(Some(&UIColor::orangeColor()));
    // SAFETY: `initWithFrame:` is `UILabel`'s plain initializer; `mtm` is
    // a test-owned allocation on the main thread.
    let label: Retained<UILabel> = unsafe {
        msg_send![
            UILabel::alloc(mtm),
            initWithFrame: CGRect::new(CGPoint::new(4.0, 30.0), CGSize::new(150.0, 24.0))
        ]
    };
    label.setText(Some(&NSString::from_str("detached")));
    // SAFETY: a test-owned label on the main thread; white is a valid color.
    unsafe { label.setTextColor(Some(&UIColor::whiteColor())) };
    content.addSubview(&label);
    assert!(content.superview().is_none());

    let target = capture_target();
    let capture = Rc::new(ViewCapture::new(mtm, content.clone(), |_| None));
    capture.set_on_redraw(|| {});
    let (flag, complete) = crate::harness::fence_flag();
    capture.capture(&target, complete);
    assert!(
        content.superview().is_none(),
        "a detached capture must not invent a parent"
    );
    crate::harness::await_fence(&flag, "detached");

    let texels = crate::harness::readback(&target);
    assert!(
        crate::harness::count_pixels(&texels, [0, 110, 220, 255], [60, 200, 255, 255]) > 5_000,
        "a detached capture must render the view's own color"
    );
    assert!(
        crate::harness::count_pixels(&texels, [220, 220, 220, 255], [255, 255, 255, 255]) > 20,
        "a detached capture must render real label glyphs"
    );

    capture.shutdown();
    drop(capture);
    crate::harness::pump_main_turn();
    let _layer = content.layer(); // crashes on an invalidated layer
    label.removeFromSuperview();
}
