//! `AppKit` cases — real `AppKit` objects built through the public API on
//! the real main thread.

use std::cell::Cell;
use std::rc::Rc;

use cocoa_ui::appkit::input_view::InputView;
use cocoa_ui::appkit::{HostView, Label, Window, WindowLevel, WindowStyle};
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::Bool;
use cocoa_ui::objc2::{msg_send, sel};
use cocoa_ui::objc2_foundation::{NSArray, NSNotFound, NSRange, NSString};
use cocoa_ui::{Rect, Size};
use libtest_mimic::Trial;

use crate::harness::{capture_target, marker, nonzero_texels};

/// The suite's `AppKit` cases, named after the module and test they moved
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
            "appkit::colors",
            extended_linear_display_p3_preserves_space_and_hdr_channels
        ),
        case!(
            "appkit::host_view",
            a_child_invalidation_inside_layout_does_not_reenter_the_pass
        ),
        case!(
            "appkit::input_view",
            the_text_input_client_selectors_register_under_appkits_names
        ),
        case!(
            "appkit::input_view",
            a_fresh_input_view_reports_no_marked_text
        ),
        case!(
            "appkit::input_view",
            valid_attributes_for_marked_text_returns_attribute_names
        ),
        case!("appkit::label", a_factory_label_survives_debug_ivar_checks),
        case!(
            "appkit::window",
            a_window_starts_hidden_with_the_requested_style
        ),
        case!("appkit::window", closing_a_window_fires_on_close),
        case!(
            "appkit::window",
            the_wrapper_forwards_title_level_size_and_content
        ),
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
    use cocoa_ui::appkit::colors;
    use cocoa_ui::objc2_core_graphics::{
        CGColor, CGColorSpace, kCGColorSpaceExtendedLinearDisplayP3,
    };

    let color = colors::extended_linear_display_p3(1.5, 0.25, 0.5, 0.8);
    let cg = color.CGColor();
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

/// Regression test for the layout-invalidation loop: a label whose width
/// changes inside the parent's layout pass invalidates upward; the old
/// ancestor walk re-entered `layout` synchronously and looped at 85% CPU.
/// The fixed path only marks ancestors dirty, so the handler's call count
/// stays bounded across a few passes.
fn a_child_invalidation_inside_layout_does_not_reenter_the_pass() {
    let mtm = marker();
    let host = HostView::new(mtm, Rect::new(0.0, 0.0, 400.0, 300.0));
    let label = Label::new(mtm);
    label.set_text("content");
    host.add_subview(&label);

    let calls = Rc::new(Cell::new(0));
    host.set_layout_handler({
        let calls = Rc::clone(&calls);
        move |view| {
            calls.set(calls.get() + 1);
            let remaining = view.bounds().size.width - label.intrinsicContentSize().width;
            if calls.get() == 1 && remaining > 40.0 {
                // Mutate a child mid-pass: the label's intrinsic size
                // invalidates upward. A synchronous ancestor walk would
                // recurse into this handler before it returns.
                label.setFrameSize(cocoa_ui::objc2_foundation::NSSize::new(
                    label.frame().size.width + 20.0,
                    label.frame().size.height,
                ));
            }
        }
    });

    host.set_needs_layout();
    host.layout_if_needed();
    // The second pass is itself the flush the mid-pass invalidation waits
    // on: `layoutSubtreeIfNeeded` is the platform's synchronous layout API —
    // no run-loop wait stands in for it.
    host.layout_if_needed();

    assert!(
        calls.get() <= 4,
        "layout re-entered {} times — invalidation must mark, not recurse",
        calls.get(),
    );
    assert!(calls.get() >= 1);
}

/// Regression test for the selector-override defect: `#[unsafe(method(..))]`
/// names compile regardless of spelling, and a method declared under a
/// `snake_case` name was never installed under the `AppKit` selector —
/// `NSTextInputContext::initWithClient:` then threw on the missing
/// `NSTextInputClient` methods. Assert the `ObjC` selectors `AppKit` calls
/// actually resolve on the class.
fn the_text_input_client_selectors_register_under_appkits_names() {
    let mtm = marker();
    // `new` itself performs `initWithClient:` — the call that threw when
    // the selectors were missing.
    let view = InputView::new(mtm);
    for selector in [
        sel!(insertText:replacementRange:),
        sel!(setMarkedText:selectedRange:replacementRange:),
        sel!(unmarkText),
        sel!(selectedRange),
        sel!(markedRange),
        sel!(hasMarkedText),
        sel!(attributedSubstringForProposedRange:actualRange:),
        sel!(validAttributesForMarkedText),
        sel!(firstRectForCharacterRange:actualRange:),
        sel!(characterIndexForPoint:),
        sel!(isFlipped),
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

/// `markedRange`/`selectedRange` answer UTF-16 `NSRange`s: an empty document
/// reports `NSNotFound`.
fn a_fresh_input_view_reports_no_marked_text() {
    let mtm = marker();
    let view = InputView::new(mtm);
    // SAFETY: `markedRange` is the view's own getter.
    let marked: NSRange = unsafe { msg_send![&*view, markedRange] };
    assert_eq!(marked.location, NSNotFound.cast_unsigned());
    assert_eq!(marked.length, 0);
    // SAFETY: `hasMarkedText` is the view's own getter.
    let has_marked: Bool = unsafe { msg_send![&*view, hasMarkedText] };
    assert!(!has_marked.as_bool());
}

/// `validAttributesForMarkedText` must answer an `NSArray` of attribute
/// names — `NSTextInputContext` reads it during composition setup.
fn valid_attributes_for_marked_text_returns_attribute_names() {
    let mtm = marker();
    let view = InputView::new(mtm);
    let attributes: Retained<NSArray<NSString>> =
        // SAFETY: `validAttributesForMarkedText` is the view's own getter.
        unsafe { msg_send![&*view, validAttributesForMarkedText] };
    assert!(attributes.count() >= 3);
}

/// The `labelWithString:` factory allocs through `self` without running
/// `set_ivars`: `setFrameSize:` — delivered during the factory's own
/// layout — must not trip the debug initialized-ivars check, and every
/// later `ivars()` access must see the marked flag.
fn a_factory_label_survives_debug_ivar_checks() {
    let mtm = marker();
    let label = Label::label_with_string(mtm, "hello");
    label.set_text("world");
    assert!(label.source_text().is_some());
    label.set_line_limit(1);
}

/// The case the old harness excluded: `-[NSWindow initWithContentRect:]`
/// throws `NSInternalInconsistency` off `pthread_main`, so a real `NSWindow`
/// could only exist once the suite moved to the true main thread. A fresh
/// window is hidden, keeps the style it was handed, and reports the content
/// area it was asked for. `FULL_SCREEN` is `AppKit`'s own state bit, not a
/// part to request, so the style goes in without it.
fn a_window_starts_hidden_with_the_requested_style() {
    let mtm = marker();
    let style = WindowStyle::all() - WindowStyle::FULL_SCREEN;
    let window = Window::new(mtm, Rect::new(100.0, 100.0, 480.0, 320.0), style);
    assert!(!window.is_visible());
    assert!(!window.is_fullscreen());
    assert_eq!(window.style_mask(), style);
    assert_eq!(window.content_rect().size, Size::new(480.0, 320.0));
    window.close();
}

/// `close()` must route through `windowWillClose:` — the notification the
/// kit's delegate observes — so the installed `on_close` handler runs.
fn closing_a_window_fires_on_close() {
    let mtm = marker();
    let window = Window::new(mtm, Rect::ZERO, WindowStyle::TITLED | WindowStyle::CLOSABLE);
    let closed = Rc::new(Cell::new(false));
    window.on_close({
        let closed = Rc::clone(&closed);
        move || closed.set(true)
    });
    window.close();
    assert!(
        closed.get(),
        "windowWillClose: never reached the on_close handler"
    );
}

/// Wrapper forwarding: property setters must land on the native `NSWindow`,
/// notifications must reach the `on_*` handlers, and a `HostView` installed
/// as content must report the window it lives in.
fn the_wrapper_forwards_title_level_size_and_content() {
    let mtm = marker();
    let window = Window::new(
        mtm,
        Rect::new(0.0, 0.0, 400.0, 300.0),
        WindowStyle::all() - WindowStyle::FULL_SCREEN,
    );

    window.set_title("native suite");
    assert_eq!(window.native().title().to_string(), "native suite");

    window.set_level(WindowLevel::Floating);
    assert_eq!(window.level(), Some(WindowLevel::Floating));
    window.set_level(WindowLevel::Normal);
    assert_eq!(window.level(), Some(WindowLevel::Normal));

    window.set_content_min_size(Size::new(200.0, 150.0));
    assert_eq!(window.content_min_size(), Size::new(200.0, 150.0));

    let resized = Rc::new(Cell::new(false));
    window.on_resize({
        let resized = Rc::clone(&resized);
        move || resized.set(true)
    });
    window.set_content_rect(Rect::new(0.0, 0.0, 500.0, 350.0), false);
    assert!(
        resized.get(),
        "windowDidResize: never reached the on_resize handler"
    );
    assert_eq!(window.content_rect().size, Size::new(500.0, 350.0));

    let host = HostView::new(mtm, window.content_rect());
    window.set_content_view(&host);
    assert!(cocoa_ui::view::has_window(&host));
    let reported = cocoa_ui::view::window(&host).expect("a hosted view reports its window");
    assert!(std::ptr::eq(&raw const *reported, window.native()));

    window.close();
}

/// The `CARenderer` root claim is scoped to the frame it encodes: after
/// `ViewCapture::capture` the layer-backed view reports the same
/// superview, sibling order, frame, and hidden flag as before — and it
/// keeps answering them after the cached renderer is released on a
/// later main-queue turn.
fn a_capture_claim_restores_containment_and_survives_release() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use cocoa_ui::capture::ViewCapture;
    use cocoa_ui::objc2_app_kit::{NSColor, NSView};
    use cocoa_ui::objc2_foundation::NSSize;

    let mtm = marker();
    // AppKit only materializes the layer hierarchy under a window: a
    // detached view tree leaves `layer.superlayer` unwired, so the
    // containment-restore contract needs a real host window.
    let window = Window::new(
        mtm,
        Rect::new(0.0, 0.0, 400.0, 400.0),
        WindowStyle::all() - WindowStyle::FULL_SCREEN,
    );
    let parent = NSView::new(mtm);
    parent.setFrameSize(NSSize::new(400.0, 400.0));
    parent.setWantsLayer(true);
    window.native().setContentView(Some(&parent));
    window.native().orderFrontRegardless();
    crate::harness::pump_main_turn();

    let content = NSView::new(mtm);
    content.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(8.0, 40.0),
        NSSize::new(200.0, 200.0),
    ));
    content.setWantsLayer(true);
    content
        .layer()
        .expect("a wanted layer exists")
        .setBackgroundColor(Some(&NSColor::systemBlueColor().CGColor()));
    // A non-identity layer transform is the preservation case: the
    // claim's restore must hand back the full `CATransform3D`, not an
    // affine view-level approximation of it.
    let mut perspective =
        cocoa_ui::objc2_quartz_core::CATransform3D::new_rotation(0.3, 0.0, 1.0, 0.0);
    perspective.m34 = -1.0 / 600.0;
    content
        .layer()
        .expect("a wanted layer exists")
        .setTransform(perspective);
    let label = Label::new(mtm);
    label.set_text("capture-claim");
    content.addSubview(&label);
    parent.addSubview(&content);
    let sibling = NSView::new(mtm);
    sibling.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(8.0, 280.0),
        NSSize::new(40.0, 40.0),
    ));
    parent.addSubview(&sibling);

    let frame = content.frame();
    let layer_transform = content.layer().expect("a wanted layer exists").transform();

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
    // claim must already be gone — superview, order, and geometry answer
    // their recorded values synchronously.
    let restored_parent = cocoa_ui::view::superview(&content)
        .expect("the claim left the view detached from its parent");
    assert!(std::ptr::eq(
        &raw const *restored_parent,
        &raw const *parent
    ));
    let order = parent.subviews();
    assert_eq!(order.count(), 2);
    assert!(std::ptr::eq(
        &raw const *order.objectAtIndex(0),
        &raw const *content
    ));
    assert!(std::ptr::eq(
        &raw const *order.objectAtIndex(1),
        &raw const *sibling
    ));
    assert_eq!(content.frame(), frame);
    assert!(!content.isHidden());
    assert!(
        content
            .layer()
            .is_some_and(|layer| layer.transform().equal_to_transform(layer_transform)),
        "the claim must preserve the layer's full transform"
    );
    assert!(
        content
            .layer()
            .is_some_and(|layer| layer.superlayer().is_some()),
        "the backing layer must be re-attached to its superlayer"
    );

    assert!(
        crate::harness::pump_main_until(5.0, || done.load(Ordering::Relaxed)),
        "the capture fence never completed"
    );
    // A layer claimed out of a window's render context encodes an empty
    // frame on a host without an app compositor, so pixel fidelity is
    // proven by the detached arm below; this arm proves the capture
    // completed and the whole containment contract survived it.
    let _ = nonzero_texels(&target);

    // Releasing the cached renderer must not invalidate the layer it
    // claimed: on the next main-queue turn the same layer answers, still
    // attached, and the tree tears down normally.
    capture.shutdown();
    drop(capture);
    crate::harness::pump_main_turn();
    let still_attached =
        cocoa_ui::view::superview(&content).expect("renderer release re-severed the view");
    assert!(std::ptr::eq(&raw const *still_attached, &raw const *parent));
    assert!(
        content
            .layer()
            .is_some_and(|layer| layer.superlayer().is_some()),
        "the backing layer lost its parent when the renderer dropped"
    );
    assert_eq!(content.frame(), frame);
    assert!(
        content
            .layer()
            .is_some_and(|layer| layer.transform().equal_to_transform(layer_transform)),
        "renderer release must not disturb the restored transform"
    );

    content.removeFromSuperview();
    sibling.removeFromSuperview();
    parent.removeFromSuperview();
    window.close();
}

/// Capturing a view with no superview is supported — the claim restores
/// nothing, the frame still renders real content, and teardown after
/// the cached renderer's release stays clean.
fn a_detached_capture_renders_and_teardown_stays_clean() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use cocoa_ui::capture::ViewCapture;
    use cocoa_ui::objc2_app_kit::{NSColor, NSView};
    use cocoa_ui::objc2_foundation::{NSPoint, NSRect, NSSize};

    let mtm = marker();
    let content = NSView::new(mtm);
    content.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(200.0, 200.0),
    ));
    content.setWantsLayer(true);
    content
        .layer()
        .expect("a wanted layer exists")
        .setBackgroundColor(Some(&NSColor::systemOrangeColor().CGColor()));
    let label = Label::new(mtm);
    label.set_text("detached");
    content.addSubview(&label);
    assert!(cocoa_ui::view::superview(&content).is_none());

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
        cocoa_ui::view::superview(&content).is_none(),
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
