//! `AppKit` cases — real `AppKit` objects built through the public API on
//! the real main thread.

use std::cell::Cell;
use std::rc::Rc;

use cocoa_ui::appkit::input_view::InputView;
use cocoa_ui::appkit::{HostView, Label, Window, WindowLevel, WindowStyle};
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::Bool;
use cocoa_ui::objc2::{msg_send, sel};
use cocoa_ui::objc2_app_kit::NSView;
use cocoa_ui::objc2_foundation::{NSArray, NSNotFound, NSRange, NSRect, NSString};
use cocoa_ui::{Point, Rect, Size};
use libtest_mimic::Trial;

use crate::harness::marker;

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
            "appkit::visibility",
            a_windowed_view_is_presentable_until_clipped_or_hidden
        ),
        case!(
            "appkit::visibility",
            ancestor_emissions_reach_a_descendants_subscribed_wake
        ),
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

/// Whether two rects share any area — the same positive-intersection test
/// `visibility::presentable` composes.
fn intersects(
    a: cocoa_ui::objc2_core_foundation::CGRect,
    b: cocoa_ui::objc2_core_foundation::CGRect,
) -> bool {
    let x = a.origin.x.max(b.origin.x);
    let y = a.origin.y.max(b.origin.y);
    (a.origin.x + a.size.width).min(b.origin.x + b.size.width) - x > 0.0
        && (a.origin.y + a.size.height).min(b.origin.y + b.size.height) - y > 0.0
}

/// `visibility::presentable` is the single gate the GPU and filtered
/// frame clocks consume: a windowed view answers `true` only while the
/// window presents, no ancestor hides or zeroes it, and the scroll clip
/// leaves some of it inside — and it flips back on reveal, reparent and
/// scroll, with no frame-size change involved. The clip assertions reuse
/// `observe_scroll_viewport` to prove the same scroll move wakes watchers.
fn a_windowed_view_is_presentable_until_clipped_or_hidden() {
    let mtm = marker();
    let window = Window::new(mtm, Rect::new(0.0, 0.0, 400.0, 300.0), WindowStyle::TITLED);
    let host = HostView::new(mtm, window.content_rect());
    window.set_content_view(&host);

    let scroll = cocoa_ui::appkit::ScrollView::new(mtm, true, false);
    cocoa_ui::view::set_frame(&scroll, Rect::new(0.0, 0.0, 200.0, 200.0));
    host.add_subview(&scroll);
    scroll.set_document_extent(Size::new(200.0, 1000.0));
    let child = HostView::new(mtm, Rect::new(0.0, 800.0, 100.0, 50.0));
    let document = scroll
        .document_view()
        .expect("the scroll view carries a document");
    cocoa_ui::view::add_subview(&document, &child);

    // A windowless view — and a view in a hidden window — cannot present:
    // `window_presentable` gates first.
    assert!(!cocoa_ui::visibility::presentable(&child));
    window.order_front();
    host.layout_if_needed();
    scroll.layout_if_needed();
    assert!(
        scroll.viewport_size().width > 0.0 && scroll.viewport_size().height > 0.0,
        "clip view never got a viewport — got {:?}",
        scroll.viewport_size()
    );
    // A bare test process never receives `occlusionState == Visible` from
    // the window server (that needs a bundled application's GUI session),
    // so `presentable` stays `false` here even ordered front — the same
    // deferral an occluded real window takes. The geometry half the gate
    // composes — native `visibleRect` under the real clip chain — is what
    // the assertions below exercise.
    assert!(!cocoa_ui::visibility::presentable(&child));

    // Fully clipped below the 200pt viewport — the #1539 defect case.
    // `visibleRect` is the clip's bounds mapped into the child's space and
    // does NOT intersect the child's own bounds, so the honest test is the
    // intersection — the same one `presentable` composes.
    let visible = child.visibleRect();
    assert!(
        !intersects(visible, child.bounds()),
        "fully clipped child should have no own-bounds area inside the clip: {visible:?}",
    );

    // The same scroll move wakes a viewport observation — the bridge the
    // components' `scroll_watch` reuses — with no frame-size change.
    let scroll_fires = Rc::new(Cell::new(0));
    let _observation = cocoa_ui::scroll::observe_scroll_viewport(&scroll.clone().into_super(), {
        let scroll_fires = Rc::clone(&scroll_fires);
        move || scroll_fires.set(scroll_fires.get() + 1)
    });
    scroll.scroll_to(cocoa_ui::Point::new(0.0, 750.0));
    scroll.layout_if_needed();
    let visible = child.visibleRect();
    assert!(
        intersects(visible, child.bounds()),
        "scrolled-in child has no own-bounds area inside the clip: {visible:?}",
    );
    assert!(
        scroll_fires.get() >= 1,
        "clip-view bounds change never fired"
    );

    // Hidden then detached ancestors empty the visible area again.
    cocoa_ui::view::set_hidden(&scroll, true);
    assert!(!cocoa_ui::visibility::presentable(&child));
    cocoa_ui::view::set_hidden(&scroll, false);
    cocoa_ui::view::remove_from_superview(&child);
    assert!(!cocoa_ui::visibility::presentable(&child));

    window.close();
}

/// The typed-owned wake: `VisibilityWatch` registers the one closure on
/// every observable ancestor, `refresh` detaches the old chain's tokens
/// before binding the new one — so a reparent's previous ancestors can
/// never reach the handler — and dropping the watch stops delivery
/// entirely. A plain `NSView` ancestor with no emitter stays
/// unobserved: mutating its notification flags is unsound under shared
/// leaves, so a host reports its changes through `updateVisibility`
/// instead — the watch must neither subscribe to it nor touch its
/// posting flags.
fn ancestor_emissions_reach_a_descendants_subscribed_wake() {
    let mtm = marker();
    let window = Window::new(mtm, Rect::new(0.0, 0.0, 400.0, 300.0), WindowStyle::TITLED);
    let host = HostView::new(mtm, window.content_rect());
    window.set_content_view(&host);
    window.order_front();

    let child = HostView::new(mtm, Rect::new(0.0, 0.0, 100.0, 50.0));
    host.add_subview(&child);

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
    // wakes on the child; the refresh that follows detaches the old
    // chain's tokens — an emission on a former ancestor can no longer
    // reach the handler — and binds the new one. `other` stays a
    // detached sibling so `host` genuinely leaves the chain.
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
        "an ancestor the watch detached still delivered a wake"
    );

    // A foreign (non-`CocoaUi`) ancestor publishes nothing the watch can
    // subscribe to — per-watch flag mutation was unsound — so its frame
    // change must NOT reach the wake and its posting flags must stay
    // untouched; the host reports those changes through the explicit
    // `updateVisibility` contract.
    // SAFETY: `initWithFrame:` on a fresh `NSView` allocation on the main
    // thread.
    let plain: Retained<NSView> = unsafe {
        msg_send![mtm.alloc::<NSView>(), initWithFrame: NSRect::new(Point::new(0.0, 0.0).into(), Size::new(200.0, 200.0).into())]
    };
    host.add_subview(&plain);
    cocoa_ui::view::remove_from_superview(&child);
    cocoa_ui::view::add_subview(&plain, &child);
    let posted_frame = plain.postsFrameChangedNotifications();
    let posted_bounds = plain.postsBoundsChangedNotifications();
    watch.refresh(&child);
    let before = fires.get();
    plain.setFrameSize(Size::new(180.0, 200.0).into());
    assert_eq!(
        fires.get() - before,
        0,
        "a foreign ancestor's frame change must not wake a descendant — hosts call updateVisibility"
    );
    // `AppKit` itself enables posting on windowed views — the watch's
    // contract is only that it leaves the flags exactly as it found
    // them.
    assert_eq!(
        (
            plain.postsFrameChangedNotifications(),
            plain.postsBoundsChangedNotifications()
        ),
        (posted_frame, posted_bounds),
        "the watch mutated a foreign ancestor's posting flags"
    );
    // The host-side contract: emitting on a mounted `CocoaUi` root — the
    // call `waterui_apple_update_visibility` performs — still reaches
    // the wake through the foreign layer.
    let before = fires.get();
    host.visibility_emitter().emit();
    assert_eq!(fires.get() - before, 1);

    drop(watch);
    let before = fires.get();
    plain.setFrameSize(Size::new(200.0, 200.0).into());
    other.visibility_emitter().emit();
    assert_eq!(
        fires.get() - before,
        0,
        "a dropped watch kept receiving wakes"
    );

    window.close();
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

/// The `CARenderer` root claim is scoped to the frame it encodes: two
/// cached `ViewCapture`s bound to nested views run child→parent claim
/// cycles, and after each the views report the same superview, sibling
/// order, superlayer, frame, and hidden flag as before — and they keep
/// answering them after both cached renderers are released on a later
/// main-queue turn. This is the nested-claim shape the production
/// ownership fix exists for.
#[allow(clippy::too_many_lines)] // The nested fixture plus the containment contract it checks.
fn a_capture_claim_restores_containment_and_survives_release() {
    use std::rc::Rc;

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
        .setBackgroundColor(Some(&NSColor::blueColor().CGColor()));
    // A non-identity transform is the preservation case. AppKit owns the
    // backing layer's transform and re-syncs it from `frameRotation`
    // during layout, so the fixture uses the view-level rotation the
    // platform persists rather than a raw `CATransform3D` AppKit would
    // revert on the next layout pass — the claim's restore must coexist
    // with that, not fight it.
    content.setFrameRotation(30.0);
    let label = Label::new(mtm);
    label.set_text("capture-claim");
    label.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(4.0, 8.0),
        NSSize::new(160.0, 24.0),
    ));
    content.addSubview(&label);
    // The nested claim: a second capturable subtree inside `content`.
    let child = NSView::new(mtm);
    child.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(20.0, 60.0),
        NSSize::new(120.0, 120.0),
    ));
    child.setWantsLayer(true);
    child
        .layer()
        .expect("a wanted layer exists")
        .setBackgroundColor(Some(&NSColor::orangeColor().CGColor()));
    let child_label = Label::new(mtm);
    child_label.set_text("nested");
    child_label.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(4.0, 8.0),
        NSSize::new(100.0, 24.0),
    ));
    child.addSubview(&child_label);
    content.addSubview(&child);
    parent.addSubview(&content);
    let sibling = NSView::new(mtm);
    sibling.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(8.0, 280.0),
        NSSize::new(40.0, 40.0),
    ));
    parent.addSubview(&sibling);
    // Let AppKit's layout pass wire the view hierarchy into the layer
    // hierarchy before the snapshots the claim will be held to.
    crate::harness::pump_main_turn();

    let frame = content.frame();
    let layer_transform = content.layer().expect("a wanted layer exists").transform();
    let child_frame = child.frame();

    let target = crate::harness::capture_target();
    let content_capture = Rc::new(ViewCapture::new(mtm, content.clone(), |_| None));
    content_capture.set_on_redraw(|| {});
    let child_capture = Rc::new(ViewCapture::new(mtm, child.clone(), |_| None));
    child_capture.set_on_redraw(|| {});

    // Everything the claim owes the tree: superview and ordered
    // siblings, the model layer's own superlayer, geometry, and the
    // full transform — checked after every cycle and again after both
    // cached renderers are released.
    let check = || {
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
        let child_parent =
            cocoa_ui::view::superview(&child).expect("the nested claim left the child detached");
        assert!(std::ptr::eq(&raw const *child_parent, &raw const *content));
        let content_order = content.subviews();
        assert_eq!(content_order.count(), 2);
        assert!(std::ptr::eq(
            &raw const *content_order.objectAtIndex(0),
            std::ptr::from_ref(&*label).cast()
        ));
        assert!(std::ptr::eq(
            &raw const *content_order.objectAtIndex(1),
            &raw const *child
        ));
        let child_order = child.subviews();
        assert_eq!(child_order.count(), 1);
        assert!(std::ptr::eq(
            &raw const *child_order.objectAtIndex(0),
            std::ptr::from_ref(&*child_label).cast()
        ));
        assert_eq!(content.frame(), frame);
        assert_eq!(child.frame(), child_frame);
        assert!(
            (content.frameRotation() - 30.0).abs() < 1e-6,
            "the claim must preserve the view's rotation"
        );
        let actual_transform = content.layer().expect("a wanted layer exists").transform();
        assert!(
            actual_transform.equal_to_transform(layer_transform),
            "the claim must preserve the layer's full transform: {actual_transform:?} vs {layer_transform:?}"
        );
        assert!(
            content
                .layer()
                .is_some_and(|layer| layer.superlayer().is_some()),
            "the model layer must keep its superlayer"
        );
        assert!(
            child
                .layer()
                .is_some_and(|layer| layer.superlayer().is_some()),
            "the nested model layer must keep its superlayer"
        );
        assert!(!content.isHidden());
    };

    // Three nested cycles: child claim first, then the enclosing content
    // claim — each against the same cached renderers, each proven by
    // its own completed, successful fence.
    for _cycle in 0..3 {
        let (flag, complete) = crate::harness::fence_flag();
        child_capture.capture(&target, complete);
        crate::harness::await_fence(&flag, "child");
        let (flag, complete) = crate::harness::fence_flag();
        content_capture.capture(&target, complete);
        crate::harness::await_fence(&flag, "content");
        check();
    }

    // A layer claimed out of a window's render context encodes an empty
    // frame on a host without an app compositor, so pixel fidelity is
    // proven by the detached arm below; this arm proves the capture
    // completed and the whole containment contract survived it.

    // Releasing the cached renderers must not invalidate the layers
    // they claimed: on the next real main-queue turn both trees answer,
    // still attached, and tear down normally.
    content_capture.shutdown();
    child_capture.shutdown();
    drop(content_capture);
    drop(child_capture);
    crate::harness::pump_main_turn();
    check();

    content.removeFromSuperview();
    sibling.removeFromSuperview();
    parent.removeFromSuperview();
    window.close();
}

/// Capturing a view with no superview is supported — the claim restores
/// nothing, the frame still renders real content, and teardown after
/// the cached renderer's release stays clean.
fn a_detached_capture_renders_and_teardown_stays_clean() {
    use std::rc::Rc;

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
        .setBackgroundColor(Some(&NSColor::orangeColor().CGColor()));
    let label = Label::new(mtm);
    // A contrasting glyph color so the readback can tell real label
    // rendering from a flat filled frame.
    label.setTextColor(Some(&NSColor::whiteColor()));
    label.set_text("detached");
    label.setWantsLayer(true);
    label.setFrame(NSRect::new(
        NSPoint::new(4.0, 60.0),
        NSSize::new(150.0, 24.0),
    ));
    content.addSubview(&label);

    // A parentless view gets no window update cycle, so a brief window
    // residency rasterizes the label's text into its backing layer
    // first — the renderer composites `layer.contents`, which only a
    // real display pass fills — before the claim detaches it again.
    {
        let window = Window::new(
            mtm,
            Rect::new(0.0, 0.0, 200.0, 200.0),
            WindowStyle::all() - WindowStyle::FULL_SCREEN,
        );
        let host = NSView::new(mtm);
        host.setFrameSize(NSSize::new(200.0, 200.0));
        host.setWantsLayer(true);
        window.native().setContentView(Some(&host));
        host.addSubview(&content);
        window.native().orderFrontRegardless();
        crate::harness::pump_main_turn();
        content.removeFromSuperview();
        window.close();
    }

    assert!(cocoa_ui::view::superview(&content).is_none());

    let target = crate::harness::capture_target();
    let capture = Rc::new(ViewCapture::new(mtm, content.clone(), |_| None));
    capture.set_on_redraw(|| {});
    let (flag, complete) = crate::harness::fence_flag();
    capture.capture(&target, complete);
    assert!(
        cocoa_ui::view::superview(&content).is_none(),
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
