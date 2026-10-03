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
            "appkit::window",
            a_window_starts_hidden_with_the_requested_style
        ),
        case!("appkit::window", closing_a_window_fires_on_close),
        case!(
            "appkit::window",
            the_wrapper_forwards_title_level_size_and_content
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
