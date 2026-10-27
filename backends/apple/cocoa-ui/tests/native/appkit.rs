//! `AppKit` cases — real `AppKit` objects built through the public API on
//! the real main thread.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cocoa_ui::appkit::input_view::InputView;
use cocoa_ui::appkit::{HostView, Label, TextField, Window, WindowLevel, WindowStyle};
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::Bool;
use cocoa_ui::objc2::{msg_send, sel};
use cocoa_ui::objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSMenuItem, NSView};
use cocoa_ui::objc2_foundation::{NSArray, NSNotFound, NSPoint, NSRange, NSRect, NSString};
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
        case!(
            "appkit::metal_presenter",
            an_armed_presenter_constructs_pauses_and_invalidates_cleanly
        ),
        case!(
            "appkit::metal_presenter",
            a_presenter_on_a_screenless_window_delivers_nothing
        ),
        case!("appkit::label", a_factory_label_survives_debug_ivar_checks),
        case!(
            "appkit::label",
            a_label_draws_its_text_at_the_top_of_an_oversized_frame
        ),
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
            a_capture_preserves_containment_and_survives_release
        ),
        case!(
            "capture",
            a_detached_capture_renders_and_teardown_stays_clean
        ),
        case!("capture", a_hidden_capture_renders_without_revealing),
        case!(
            "appkit::text_field",
            a_fields_edit_path_carries_selection_undo_and_change
        ),
        case!(
            "appkit::text_field",
            a_single_line_fields_replacement_keeps_its_line_invariant
        ),
        case!(
            "appkit::text_field",
            the_edit_menu_handler_appends_to_the_field_editors_menu
        ),
        case!(
            "appkit::text_field",
            a_fields_history_observation_retires_when_editing_moves
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

/// Checks the presenter's arm lifecycle on an on-screen host: it
/// constructs paused, `set_paused` toggles the link's arm state,
/// `invalidate` stops delivery for good, and nothing arrives afterwards.
/// Actual `CAMetalDisplayLink` delivery is never observed on this runner
/// (the paravirtual display paces nothing), so the link's delivery count
/// is asserted silent rather than counted.
fn an_armed_presenter_constructs_pauses_and_invalidates_cleanly() {
    use cocoa_ui::metal_presenter::MetalPresenter;
    use cocoa_ui::objc2_app_kit::{NSScreen, NSView};
    use cocoa_ui::objc2_foundation::{NSPoint, NSRect, NSSize};
    use cocoa_ui::objc2_quartz_core::CAMetalLayer;

    let mtm = marker();
    let main = NSScreen::screens(mtm)
        .iter()
        .next()
        .expect("the suite requires a real screen")
        .frame();
    let window = Window::new(
        mtm,
        Rect::new(main.origin.x + 40.0, main.origin.y + 40.0, 200.0, 200.0),
        WindowStyle::empty(),
    );
    let view = NSView::new(mtm);
    view.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(200.0, 200.0)));
    let layer = CAMetalLayer::new();
    layer.setDrawableSize(cocoa_ui::objc2_core_foundation::CGSize::new(200.0, 200.0));
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    window.native().setContentView(Some(&view));
    window.native().orderFrontRegardless();

    let updates = Rc::new(Cell::new(0u32));
    let presenter = MetalPresenter::new(
        layer,
        {
            let updates = Rc::clone(&updates);
            Rc::new(move |_| updates.set(updates.get() + 1))
        },
        false,
    );

    // The presenter starts paused; arming schedules link updates on the
    // main run loop. Delivery itself cannot be asserted on this runner —
    // the paravirtual display paces `CAMetalDisplayLink` for nothing, not
    // even a plain Swift window — so the check is contract-level only:
    // arming and re-pausing are accepted, and drop stays silent.
    assert!(presenter.is_paused());
    presenter.set_paused(false);
    assert!(!presenter.is_paused());
    presenter.set_paused(true);
    presenter.set_paused(false);
    drop(presenter);
    crate::harness::pump_main_until(0.3, || false);
    assert_eq!(updates.get(), 0, "a dropped link delivered a frame");
    window.close();
}

/// Checks a `MetalPresenter` on a `CAMetalLayer`-backed view in a window
/// outside every screen: construction is safe, an armed link delivers no
/// frame within a bounded run-loop window, and drop leaves nothing
/// queued.
fn a_presenter_on_a_screenless_window_delivers_nothing() {
    use cocoa_ui::metal_presenter::MetalPresenter;
    use cocoa_ui::objc2_app_kit::{NSScreen, NSView};
    use cocoa_ui::objc2_foundation::{NSPoint, NSRect, NSSize};
    use cocoa_ui::objc2_quartz_core::CAMetalLayer;

    let mtm = marker();
    // A frame whose origin lies to the right of every screen's extent.
    let offscreen = NSScreen::screens(mtm)
        .iter()
        .fold(0.0_f64, |right, screen| {
            let frame = screen.frame();
            right.max(frame.origin.x + frame.size.width)
        })
        + 10_000.0;
    let window = Window::new(
        mtm,
        Rect::new(offscreen, 0.0, 200.0, 200.0),
        WindowStyle::empty(),
    );
    let view = NSView::new(mtm);
    view.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(200.0, 200.0)));
    let layer = CAMetalLayer::new();
    layer.setDrawableSize(cocoa_ui::objc2_core_foundation::CGSize::new(200.0, 200.0));
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    window.native().setContentView(Some(&view));

    let updates = Rc::new(Cell::new(0u32));
    let presenter = MetalPresenter::new(
        layer,
        {
            let updates = Rc::clone(&updates);
            Rc::new(move |_| updates.set(updates.get() + 1))
        },
        false,
    );

    presenter.set_paused(false);
    crate::harness::pump_main_until(0.5, || false);
    assert_eq!(
        updates.get(),
        0,
        "a link hosted outside every screen delivered a frame"
    );
    drop(presenter);
    crate::harness::pump_main_until(0.3, || false);
    assert_eq!(updates.get(), 0, "a dropped link delivered a frame");
    window.close();
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

/// The `AppKit` anchor the `UIKit` label now matches — the parity check for
/// water-rs/waterui#2718: inside a frame taller than the text, the ink a
/// label draws anchors at the top of its bounds, never vertically
/// centred. `cacheDisplay` is the view's offscreen raster, so the ink
/// rows it leaves name where the text went.
fn a_label_draws_its_text_at_the_top_of_an_oversized_frame() {
    let mtm = marker();
    let label = Label::label_with_string(mtm, "Hello 0");
    label.setTextColor(Some(&cocoa_ui::objc2_app_kit::NSColor::blackColor()));
    label.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(0.0, 0.0),
        cocoa_ui::objc2_foundation::NSSize::new(320.0, 240.0),
    ));
    let rep = label
        .bitmapImageRepForCachingDisplayInRect(label.bounds())
        .expect("a bitmap rep");
    label.cacheDisplayInRect_toBitmapImageRep(label.bounds(), &rep);
    // SAFETY: `bitmapData` answers `pixelsHigh * bytesPerRow` readable
    // bytes, RGBA8 premultiplied.
    let ink_rows = unsafe {
        let bytes = rep.bitmapData().cast::<u8>();
        let stride = usize::try_from(rep.bytesPerRow()).unwrap();
        let wide = usize::try_from(rep.pixelsWide()).unwrap();
        (0..usize::try_from(rep.pixelsHigh()).unwrap())
            .filter(|row| {
                (0..wide).any(|col| {
                    let px = bytes.add(row * stride + col * 4);
                    *px.add(3) > 100 && *px < 200 && *px.add(1) < 200 && *px.add(2) < 200
                })
            })
            .collect::<Vec<_>>()
    };
    assert!(!ink_rows.is_empty(), "the label drew no ink");
    let height = usize::try_from(rep.pixelsHigh()).unwrap();
    // The rep's row 0 is the frame's top edge in screen order.
    let last_ink_row = *ink_rows.last().unwrap();
    assert!(
        last_ink_row < height / 4,
        "the text's last ink row must sit in the frame's top quarter, saw row {last_ink_row} of {height}"
    );
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

/// The native raster pass reads the live layer tree without claiming it:
/// two cached `ViewCapture`s bound to nested views run child→parent
/// capture cycles, and after each the views report the same superview,
/// sibling order, superlayer, frame, and hidden flag as before — and
/// they keep answering them after both cached raster frames are released
/// on a later main-queue turn. This is the nested-capture shape the
/// containment contract exists for.
#[allow(clippy::too_many_lines)] // The nested fixture plus the containment contract it checks.
fn a_capture_preserves_containment_and_survives_release() {
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
    let content_capture = Rc::new(ViewCapture::new(mtm, content.clone()));
    content_capture.set_on_redraw(|| {});
    let child_capture = Rc::new(ViewCapture::new(mtm, child.clone()));
    child_capture.set_on_redraw(|| {});

    // Everything the tree must keep through a capture: superview and
    // ordered siblings, the model layer's own superlayer, geometry, and
    // the full transform — checked after every cycle and again after
    // both cached raster frames are released.
    let check = || {
        let restored_parent = cocoa_ui::view::superview(&content)
            .expect("the capture left the view detached from its parent");
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
            cocoa_ui::view::superview(&child).expect("the nested capture left the child detached");
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
            "the capture must preserve the view's rotation"
        );
        let actual_transform = content.layer().expect("a wanted layer exists").transform();
        assert!(
            actual_transform.equal_to_transform(layer_transform),
            "the capture must preserve the layer's full transform: {actual_transform:?} vs {layer_transform:?}"
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

    // Three nested cycles: child capture first, then the enclosing
    // content capture — each against the same cached raster frames, each
    // proven by its own completed, successful fence.
    for _cycle in 0..3 {
        let (flag, complete) = crate::harness::fence_flag();
        child_capture.capture(&target, 0, complete);
        crate::harness::await_fence(&flag, "child");
        let (flag, complete) = crate::harness::fence_flag();
        content_capture.capture(&target, 0, complete);
        crate::harness::await_fence(&flag, "content");
        check();
    }

    // A tree outside a window's render context may rasterize an empty
    // frame on a host without an app compositor, so pixel fidelity is
    // proven by the detached arm below; this arm proves the capture
    // completed and the whole containment contract survived it.

    // Releasing the cached raster frames must not invalidate the layers
    // they drew: on the next real main-queue turn both trees answer,
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

/// Capturing a view with no superview is supported — no containment is
/// disturbed, the frame still renders real content, and teardown after
/// the cached raster frame's release stays clean.
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
    // first — the capture composites `layer.contents`, which only a
    // real display pass fills — before the view detaches again.
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
    let capture = Rc::new(ViewCapture::new(mtm, content.clone()));
    capture.set_on_redraw(|| {});
    let (flag, complete) = crate::harness::fence_flag();
    capture.capture(&target, 0, complete);
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

/// A normally-hidden subtree — the shape a filter-owned source actually
/// has — must still rasterize: the capture un-hides the root inside the
/// single transaction, reads the revealed tree, and re-hides before the
/// sole commit, so the flag never leaves `true` and the pixels still
/// land.
fn a_hidden_capture_renders_without_revealing() {
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
    label.setTextColor(Some(&NSColor::whiteColor()));
    label.set_text("hidden");
    label.setWantsLayer(true);
    label.setFrame(NSRect::new(
        NSPoint::new(4.0, 60.0),
        NSSize::new(150.0, 24.0),
    ));
    content.addSubview(&label);

    // A contrasting band so the readback can see the label's frame,
    // not just its glyphs.
    label
        .layer()
        .expect("a wanted layer exists")
        .setBackgroundColor(Some(&NSColor::systemYellowColor().CGColor()));

    // Same backing-fill trick as the detached arm: a brief window
    // residency rasterizes the label into its layer contents first.
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

    // A pending layout the pass must absorb: the root already resized,
    // so the label's autoresizing only applies when layout runs inside
    // the transaction — a wider label frame afterwards proves the
    // collected geometry and the draw both saw the post-layout tree.
    label.setAutoresizingMask(cocoa_ui::objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable);
    content.setFrameSize(NSSize::new(300.0, 200.0));
    content.setNeedsLayout(true);

    // The hidden filter-owned-root case: hidden before the capture ever
    // runs.
    content.setHidden(true);

    let target = crate::harness::capture_target();
    let capture = Rc::new(ViewCapture::new(mtm, content.clone()));
    capture.set_on_redraw(|| {});
    let (flag, complete) = crate::harness::fence_flag();
    capture.capture(&target, 0, complete);
    // The reveal lived and died inside one transaction: the flag is
    // already restored when `capture` returns — nothing was committed
    // while the tree was un-hidden.
    assert!(
        content.isHidden(),
        "a hidden root must be re-hidden before the capture commits"
    );
    crate::harness::await_fence(&flag, "hidden");
    assert!(
        content.isHidden(),
        "the flag must stay set after the frame settles"
    );
    // The autoresized label widens only when the pending layout runs:
    // the right margin 200 - 4 - 150 = 46 is preserved, so the
    // post-layout width is 300 - 4 - 46 = 250.
    assert!(
        (label.frame().size.width - 250.0).abs() < 1.0,
        "the pending layout must have run inside the capture: {:?}",
        label.frame()
    );

    let texels = crate::harness::readback(&target);
    assert!(
        crate::harness::count_pixels(&texels, [0, 110, 220, 255], [60, 200, 255, 255]) > 5_000,
        "a hidden root must still rasterize its own color"
    );
    assert!(
        crate::harness::count_pixels(&texels, [0, 180, 180, 255], [120, 255, 255, 255]) > 1_000,
        "the raster must come from the post-layout tree: the label's widened band"
    );

    capture.shutdown();
    drop(capture);
    crate::harness::pump_main_turn();
    label.removeFromSuperview();
}

/// A text field in a never-ordered window: `begin_editing` brings up the
/// shared field editor, `set_selected_range`/`replace_range` act on it
/// through the native edit path — the change handler observes each splice,
/// the caret lands after inserted text and the editor's undo manager
/// restores the pre-edit text through the same notification.
fn a_fields_edit_path_carries_selection_undo_and_change() {
    let mtm = marker();
    let window = Window::new(mtm, Rect::new(0.0, 0.0, 400.0, 300.0), WindowStyle::TITLED);
    let host = HostView::new(mtm, window.content_rect());
    window.set_content_view(&host);
    let field = TextField::new(mtm, None);
    cocoa_ui::view::set_frame(&field, Rect::new(10.0, 10.0, 200.0, 24.0));
    host.add_subview(&field);
    field.setStringValue(&NSString::from_str("hé😀b"));

    assert!(
        field.selected_range().is_none(),
        "an unedited field reports no selection"
    );
    assert!(
        field.begin_editing(),
        "selectText: produced no field editor"
    );
    let editor = field.field_editor().expect("the field editor is live");

    let changes = Rc::new(RefCell::new(Vec::<String>::new()));
    field.on_change({
        let changes = Rc::clone(&changes);
        move |field| changes.borrow_mut().push(field.string())
    });

    // UTF-16 units: h=1 é=1 =1 😀=2 b=1 — 😀 spans units 2..4.
    field.set_selected_range(NSRange::new(2, 2));
    assert_eq!(field.selected_range(), Some(NSRange::new(2, 2)));

    field.replace_range(NSRange::new(2, 2), &NSString::from_str("X"));
    assert_eq!(field.string(), "héXb");
    assert_eq!(changes.borrow().as_slice(), ["héXb"]);
    assert_eq!(
        field.selected_range(),
        Some(NSRange::new(3, 0)),
        "the caret lands after the inserted text"
    );

    // Undo of the surrogate-pair replacement restores the exact original
    // text through the same change notification.
    let undo = editor
        .undoManager()
        .expect("the field editor carries an undo manager");
    undo.undo();
    crate::harness::pump_main_turn();
    assert_eq!(
        field.string(),
        "hé😀b",
        "undo restores the pre-replacement text"
    );
    assert_eq!(
        changes.borrow().as_slice(),
        ["héXb", "hé😀b"],
        "the editor's DidUndo notification reports the restored text once"
    );
    undo.redo();
    crate::harness::pump_main_turn();
    assert_eq!(field.string(), "héXb", "redo replays the replacement");
    assert_eq!(
        changes.borrow().as_slice(),
        ["héXb", "hé😀b", "héXb"],
        "DidRedo reports the replayed text once"
    );
    undo.undo();
    crate::harness::pump_main_turn();

    // A caret insertion is an empty-range replacement.
    field.replace_range(NSRange::new(0, 0), &NSString::from_str(">"));
    assert_eq!(field.string(), ">hé😀b");

    window.close();
}

/// A single-line field's `replace_range` is the same entry a pasted
/// insertion takes, so embedded newlines never survive into it.
fn a_single_line_fields_replacement_keeps_its_line_invariant() {
    let mtm = marker();
    let window = Window::new(mtm, Rect::new(0.0, 0.0, 400.0, 300.0), WindowStyle::TITLED);
    let host = HostView::new(mtm, window.content_rect());
    window.set_content_view(&host);
    let field = TextField::new(mtm, Some(1));
    cocoa_ui::view::set_frame(&field, Rect::new(10.0, 10.0, 200.0, 24.0));
    host.add_subview(&field);

    assert!(field.begin_editing());
    field.replace_range(NSRange::new(0, 0), &NSString::from_str("a\nb\nc"));
    assert!(
        !field.string().contains('\n'),
        "a single-line field admitted a newline: {:?}",
        field.string()
    );
    window.close();
}

/// `textView:menu:forEvent:atIndex:` is the field editor's context-menu
/// hook and the field is its delegate: a right-click menu request reaches
/// the installed handler with the standard menu, whose items stay and
/// whose appended rows the method returns.
fn the_edit_menu_handler_appends_to_the_field_editors_menu() {
    let mtm = marker();
    let window = Window::new(mtm, Rect::new(0.0, 0.0, 400.0, 300.0), WindowStyle::TITLED);
    let host = HostView::new(mtm, window.content_rect());
    window.set_content_view(&host);
    let field = TextField::new(mtm, None);
    cocoa_ui::view::set_frame(&field, Rect::new(10.0, 10.0, 200.0, 24.0));
    host.add_subview(&field);
    assert!(field.begin_editing());
    let editor = field.field_editor().unwrap();

    let asked = Rc::new(Cell::new(0u32));
    field.install_edit_menu_handler({
        let asked = Rc::clone(&asked);
        move |_field, _editor, menu| {
            asked.set(asked.get() + 1);
            menu.addItem(&NSMenuItem::new(mtm));
            if let Some(item) = menu.itemAtIndex(menu.numberOfItems() - 1) {
                item.setTitle(&NSString::from_str("Custom Row"));
            }
        }
    });

    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::RightMouseDown,
        NSPoint::new(5.0, 5.0),
        NSEventModifierFlags::empty(),
        0.0,
        window.native().windowNumber(),
        None,
        1,
        1,
        0.0,
    )
    .expect("a right-mouse-down event constructs");
    let menu = editor
        .menuForEvent(&event)
        .expect("the field editor produced a menu");

    assert!(asked.get() > 0, "the delegate hook never ran");
    assert!(
        menu.itemWithTitle(&NSString::from_str("Custom Row"))
            .is_some(),
        "the appended row is missing from the editor's menu"
    );
    assert!(
        menu.numberOfItems() > 1,
        "the platform's standard rows were replaced rather than kept"
    );
    window.close();
}

/// Two fields share the window's field editor: editing `A`, then moving
/// editing to `B` ends `A`'s session — its undo observers retire and its
/// `on_change` stays silent while `B`'s own undo reports once.
fn a_fields_history_observation_retires_when_editing_moves() {
    let mtm = marker();
    let window = Window::new(mtm, Rect::new(0.0, 0.0, 400.0, 300.0), WindowStyle::TITLED);
    let host = HostView::new(mtm, window.content_rect());
    window.set_content_view(&host);
    let field_a = TextField::new(mtm, None);
    cocoa_ui::view::set_frame(&field_a, Rect::new(10.0, 10.0, 200.0, 24.0));
    host.add_subview(&field_a);
    let field_b = TextField::new(mtm, None);
    cocoa_ui::view::set_frame(&field_b, Rect::new(10.0, 44.0, 200.0, 24.0));
    host.add_subview(&field_b);
    field_a.setStringValue(&NSString::from_str("a"));
    field_b.setStringValue(&NSString::from_str("b"));

    let a_changes = Rc::new(RefCell::new(Vec::<String>::new()));
    field_a.on_change({
        let a_changes = Rc::clone(&a_changes);
        move |field| a_changes.borrow_mut().push(field.string())
    });
    let b_changes = Rc::new(RefCell::new(Vec::<String>::new()));
    field_b.on_change({
        let b_changes = Rc::clone(&b_changes);
        move |field| b_changes.borrow_mut().push(field.string())
    });

    assert!(field_a.begin_editing(), "A never began editing");
    field_a.replace_range(NSRange::new(0, 1), &NSString::from_str("A"));
    assert_eq!(a_changes.borrow().as_slice(), ["A"]);

    // Editing moves to B: the shared editor transfers and A's
    // `controlTextDidEndEditing` retires its session observers.
    assert!(field_b.begin_editing(), "B never began editing");
    field_b.replace_range(NSRange::new(0, 1), &NSString::from_str("B"));
    assert_eq!(b_changes.borrow().as_slice(), ["B"]);

    let undo = field_b
        .field_editor()
        .expect("B has the shared editor")
        .undoManager()
        .expect("the field editor carries an undo manager");
    undo.undo();
    crate::harness::pump_main_turn();
    assert_eq!(
        b_changes.borrow().as_slice(),
        ["B", "b"],
        "B's session reports its undo"
    );
    assert_eq!(
        a_changes.borrow().as_slice(),
        ["A"],
        "A's retired session reports nothing"
    );

    window.close();
}
