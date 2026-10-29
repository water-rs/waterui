//! `UIKit` cases — real `UIKit` objects built through the public API on the
//! real main thread.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::{AnyObject, Bool, Sel};
use cocoa_ui::objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, msg_send, sel};
use cocoa_ui::objc2_core_foundation::{CGPoint, CGRect, CGSize};
use cocoa_ui::objc2_foundation::{NSRange, NSString};
use cocoa_ui::objc2_ui_kit::{
    UIGestureRecognizerState, UIKeyInput, UIPanGestureRecognizer, UITextInput, UITextRange,
    UITouch, UITouchType, UIView, UIWindow,
};
use cocoa_ui::uikit::input_view::{InputView, TextRange};
use cocoa_ui::uikit::text_input::{
    Autocapitalization, Autocorrection, KeyboardType, ReturnKey, TextInputClient, TextInputTraits,
};
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
            "uikit::label",
            a_label_draws_its_text_at_the_top_of_an_oversized_frame
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
            "uikit::input_view",
            indirect_scroll_input_arrives_as_surface_scroll_events
        ),
        case!(
            "uikit::input_view",
            marked_text_composes_against_the_client_document
        ),
        case!(
            "uikit::input_view",
            insert_text_replaces_the_marked_range_else_the_selection
        ),
        case!(
            "uikit::input_view",
            delete_backward_removes_one_grapheme_or_the_selection
        ),
        case!(
            "uikit::input_view",
            selection_and_geometry_queries_answer_from_the_client
        ),
        case!(
            "uikit::input_view",
            delete_backward_is_the_clients_when_it_overrides_it
        ),
        case!("uikit::input_view", traits_come_from_the_client),
        case!(
            "uikit::input_view",
            an_input_view_without_a_client_is_an_empty_document
        ),
        case!(
            "uikit::input_view",
            a_cancelled_touch_is_reported_as_cancelled_not_released
        ),
        case!(
            "uikit::input_view",
            layout_and_safe_area_handlers_follow_the_view
        ),
        case!("display_link", a_paused_frame_clock_stays_armed_but_silent),
        case!(
            "uikit::menu_panel",
            an_unpresented_panel_tracks_its_rows_pages_and_focus
        ),
        case!("view", display_immediately_targets_the_layer_not_the_view),
        case!(
            "capture",
            a_capture_preserves_containment_and_survives_release
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

/// Regression for water-rs/waterui#2718: `UILabel` vertically centres
/// the rect it draws text in, but a leaf's frame is its layout answer —
/// the ink anchors at the top the way `NSTextFieldCell` draws an
/// oversized frame. The case rasterizes a tall label and asserts the
/// text rows sit at the frame's top.
fn a_label_draws_its_text_at_the_top_of_an_oversized_frame() {
    use cocoa_ui::objc2::AllocAnyThread;

    let mtm = marker();
    let label = Label::new(mtm);
    let attributed = cocoa_ui::objc2_foundation::NSAttributedString::initWithString(
        cocoa_ui::objc2_foundation::NSAttributedString::alloc(),
        &cocoa_ui::objc2_foundation::NSString::from_str("Hello 0"),
    );
    label.set_attributed_text(&attributed);
    let frame = Rect::new(0.0, 0.0, 320.0, 240.0);
    label.setFrame(frame.into());
    // The geometry contract `drawTextInRect:` receives: the rect text is
    // drawn in must name the top of the bounds, at the text's own height.
    let text_rect = label.textRectForBounds_limitedToNumberOfLines(frame.into(), 0);
    let intrinsic = label.intrinsicContentSize();
    assert!(
        text_rect.origin.y <= 1.0,
        "the text rect anchors at the bounds' top, saw y={}",
        text_rect.origin.y
    );
    assert!(
        text_rect.size.height < 240.0 * 0.5,
        "the text rect is the text's height, saw {}",
        text_rect.size.height
    );
    // The rasterized check: render the layer into an RGBA8 bitmap and
    // find the rows holding ink — they must sit in the frame's top
    // quarter, not centred.
    let (width, height) = (320usize, 240usize);
    let mut pixels = vec![0u8; width * height * 4];
    let context =
        cocoa_ui::bitmap::bitmap_context(&mut pixels, width, height).expect("a bitmap context");
    cocoa_ui::bitmap::with_uikit_context(&context, || {
        cocoa_ui::bitmap::begin_layer_flip(&context, frame.size.height);
        cocoa_ui::bitmap::render_layer(&label.layer(), &context);
        cocoa_ui::bitmap::end_layer_flip(&context);
    });
    let ink_rows: Vec<usize> = (0..height)
        .filter(|row| {
            (0..width).any(|col| {
                let px = row * width * 4 + col * 4;
                pixels[px + 3] > 100
                    && pixels[px] < 200
                    && pixels[px + 1] < 200
                    && pixels[px + 2] < 200
            })
        })
        .collect();
    assert!(
        !ink_rows.is_empty(),
        "the label drew no ink — intrinsic {intrinsic:?}"
    );
    let last_ink_row = *ink_rows.last().unwrap();
    assert!(
        last_ink_row < height / 4,
        "the text's last ink row must sit in the frame's top quarter, saw row {last_ink_row} of {height}"
    );
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
        sel!(touchesCancelled:withEvent:),
        sel!(pressesBegan:withEvent:),
        sel!(pressesCancelled:withEvent:),
        sel!(keyboardType),
        sel!(returnKeyType),
        sel!(isSecureTextEntry),
        sel!(autocorrectionType),
        sel!(autocapitalizationType),
        sel!(textContentType),
        sel!(selectionRectsForRange:),
        sel!(setInputDelegate:),
        sel!(safeAreaInsetsDidChange),
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

/// A document in UTF-16 units, the model a [`TextInputClient`] edits.
#[derive(Debug)]
struct TestClient {
    text: RefCell<Vec<u16>>,
    selection: RefCell<Range<usize>>,
    marked: RefCell<Option<Range<usize>>>,
    traits: Cell<TextInputTraits>,
    editing: Cell<bool>,
    returns: Cell<u32>,
}

impl TestClient {
    fn new(text: &str) -> Rc<Self> {
        let text: Vec<u16> = text.encode_utf16().collect();
        let end = text.len();
        Rc::new(Self {
            text: RefCell::new(text),
            selection: RefCell::new(end..end),
            marked: RefCell::new(None),
            traits: Cell::new(TextInputTraits::default()),
            editing: Cell::new(true),
            returns: Cell::new(0),
        })
    }

    fn document(&self) -> String {
        String::from_utf16(&self.text.borrow()).expect("the document is valid UTF-16")
    }

    fn select(&self, range: Range<usize>) {
        self.selection.replace(range);
    }
}

impl TextInputClient for TestClient {
    fn is_editing(&self) -> bool {
        self.editing.get()
    }

    fn traits(&self) -> TextInputTraits {
        self.traits.get()
    }

    fn len(&self) -> usize {
        self.text.borrow().len()
    }

    fn text(&self, range: Range<usize>) -> String {
        String::from_utf16_lossy(&self.text.borrow()[range])
    }

    fn selection(&self) -> Range<usize> {
        self.selection.borrow().clone()
    }

    fn set_selection(&self, range: Range<usize>) {
        self.selection.replace(range);
    }

    fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.borrow().clone()
    }

    fn set_marked_text(&self, text: &str, selected: Range<usize>) {
        let range = self.marked_range().unwrap_or_else(|| self.selection());
        let units: Vec<u16> = text.encode_utf16().collect();
        let start = range.start;
        let end = start + units.len();
        self.text.borrow_mut().splice(range, units);
        self.marked.replace(Some(start..end));
        self.selection
            .replace(start + selected.start..start + selected.end);
    }

    fn unmark_text(&self) {
        self.marked.replace(None);
    }

    fn replace(&self, range: Range<usize>, text: &str) {
        let units: Vec<u16> = text.encode_utf16().collect();
        let end = range.start + units.len();
        self.text.borrow_mut().splice(range, units);
        self.marked.replace(None);
        self.selection.replace(end..end);
    }

    fn first_rect(&self, range: Range<usize>) -> Rect {
        Rect::new(cells(range.start), 0.0, 10.0, 20.0)
    }

    fn caret_rect(&self, offset: usize) -> Rect {
        Rect::new(cells(offset), 0.0, 1.0, 20.0)
    }

    fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        vec![
            Rect::new(cells(range.start), 0.0, 10.0, 20.0),
            Rect::new(0.0, 20.0, cells(range.end), 20.0),
        ]
    }

    fn closest_offset(&self, point: kurbo::Point) -> usize {
        (0..=self.len())
            .min_by(|a, b| {
                (cells(*a) - point.x)
                    .abs()
                    .total_cmp(&(cells(*b) - point.x).abs())
            })
            .expect("the range is never empty")
    }

    fn perform_return(&self) {
        self.returns.set(self.returns.get() + 1);
    }
}

/// An `InputView` over a [`TestClient`] holding `text`, caret at the end.
fn view_over(text: &str) -> (Retained<InputView>, Rc<TestClient>) {
    let view = InputView::new(marker());
    let client = TestClient::new(text);
    view.set_text_input_client(client.clone());
    (view, client)
}

/// The offsets of a `UITextRange` this view handed out.
fn offsets(range: &UITextRange) -> (usize, usize) {
    range
        .downcast_ref::<TextRange>()
        .expect("the view hands out its own ranges")
        .offsets()
}

/// A count of UTF-16 units as the 10-point-wide cells [`TestClient`] lays
/// its geometry out in.
fn cells(units: usize) -> f64 {
    f64::from(u32::try_from(units).expect("test documents are small")) * 10.0
}

/// An input method composes in place: `setMarkedText:` marks and selects
/// inside the client's document, `markedTextRange` reports the mark with
/// UTF-16 offsets (a surrogate pair counts two), an empty mark abandons the
/// composition, and `unmarkText` commits it.
fn marked_text_composes_against_the_client_document() {
    let (view, client) = view_over("ab");
    assert!(view.hasText(), "an editing client with text has text");

    view.setMarkedText_selectedRange(Some(&NSString::from_str("ni\u{1F600}")), NSRange::new(2, 0));
    assert_eq!(client.document(), "abni\u{1F600}");
    assert_eq!(client.marked_range(), Some(2..6));
    assert_eq!(
        view.markedTextRange().map(|range| offsets(&range)),
        Some((2, 6))
    );
    assert_eq!(
        client.selection(),
        4..4,
        "the selection is relative to the mark"
    );

    // A second update replaces the mark, not the document before it.
    view.setMarkedText_selectedRange(Some(&NSString::from_str("\u{4F60}")), NSRange::new(1, 0));
    assert_eq!(client.document(), "ab\u{4F60}");
    assert_eq!(
        view.markedTextRange().map(|range| offsets(&range)),
        Some((2, 3))
    );

    view.unmarkText();
    assert_eq!(
        client.document(),
        "ab\u{4F60}",
        "unmarking commits the text"
    );
    assert_eq!(view.markedTextRange(), None);

    // An empty mark abandons a composition.
    view.setMarkedText_selectedRange(Some(&NSString::from_str("x")), NSRange::new(1, 0));
    assert_eq!(client.document(), "ab\u{4F60}x");
    view.setMarkedText_selectedRange(None, NSRange::new(0, 0));
    assert_eq!(client.document(), "ab\u{4F60}");
    assert_eq!(view.markedTextRange(), None);
}

/// `insertText:` replaces the marked range when there is one, else the
/// selection; the return key's newline is the editor's action, not text.
fn insert_text_replaces_the_marked_range_else_the_selection() {
    let (view, client) = view_over("hello world");
    client.select(0..5);
    view.insertText(&NSString::from_str("bye"));
    assert_eq!(client.document(), "bye world");
    assert_eq!(client.selection(), 3..3, "the caret follows the insertion");

    view.setMarkedText_selectedRange(Some(&NSString::from_str("wo")), NSRange::new(2, 0));
    assert_eq!(client.document(), "byewo world");
    view.insertText(&NSString::from_str("W"));
    assert_eq!(
        client.document(),
        "byeW world",
        "the committed text replaced the mark, not the selection"
    );
    assert_eq!(client.marked_range(), None);

    view.insertText(&NSString::from_str("\n"));
    assert_eq!(client.returns.get(), 1);
    assert_eq!(
        client.document(),
        "byeW world",
        "a newline is the editor's call"
    );
}

/// `deleteBackward` is an edit of the document: it removes the selection
/// when there is one, else the whole grapheme cluster before the caret — a
/// surrogate pair or a base with a combining mark is one character — and
/// does nothing at the start.
fn delete_backward_removes_one_grapheme_or_the_selection() {
    let (view, client) = view_over("a\u{1F600}");
    view.deleteBackward();
    assert_eq!(client.document(), "a", "a surrogate pair deletes whole");
    assert_eq!(client.selection(), 1..1);

    client.replace(1..1, "e\u{301}");
    view.deleteBackward();
    assert_eq!(client.document(), "a", "a combining sequence deletes whole");

    client.replace(1..1, "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
    view.deleteBackward();
    assert_eq!(client.document(), "a", "a ZWJ sequence deletes whole");

    client.replace(1..1, "bcd");
    client.select(1..3);
    view.deleteBackward();
    assert_eq!(client.document(), "ad", "a selection deletes as a range");

    client.select(0..0);
    view.deleteBackward();
    assert_eq!(
        client.document(),
        "ad",
        "nothing precedes the document start"
    );
}

/// A client for event-based content: no document, so the delete key is its
/// own override, and it must be reached even with nothing to delete.
#[derive(Debug, Default)]
struct EventClient {
    deletes: Cell<u32>,
    inserted: RefCell<Vec<String>>,
}

impl TextInputClient for EventClient {
    fn is_editing(&self) -> bool {
        false
    }

    fn traits(&self) -> TextInputTraits {
        TextInputTraits::default()
    }

    fn len(&self) -> usize {
        0
    }

    fn text(&self, _range: Range<usize>) -> String {
        String::new()
    }

    fn selection(&self) -> Range<usize> {
        0..0
    }

    fn set_selection(&self, _range: Range<usize>) {}

    fn marked_range(&self) -> Option<Range<usize>> {
        None
    }

    fn set_marked_text(&self, _text: &str, _selected: Range<usize>) {}

    fn unmark_text(&self) {}

    fn replace(&self, _range: Range<usize>, text: &str) {
        self.inserted.borrow_mut().push(text.to_owned());
    }

    fn delete_backward(&self) {
        self.deletes.set(self.deletes.get() + 1);
    }

    fn first_rect(&self, _range: Range<usize>) -> Rect {
        Rect::ZERO
    }

    fn caret_rect(&self, _offset: usize) -> Rect {
        Rect::ZERO
    }

    fn selection_rects(&self, _range: Range<usize>) -> Vec<Rect> {
        Vec::new()
    }

    fn closest_offset(&self, _point: kurbo::Point) -> usize {
        0
    }

    fn perform_return(&self) {}
}

/// `deleteBackward` is the client's own `delete_backward`: content with no
/// document gets the delete key even at an empty document start, and
/// `insertText:` outside a composition reaches it as a plain replacement.
fn delete_backward_is_the_clients_when_it_overrides_it() {
    let view = InputView::new(marker());
    let client = Rc::new(EventClient::default());
    view.set_text_input_client(client.clone());
    view.deleteBackward();
    view.deleteBackward();
    assert_eq!(client.deletes.get(), 2);
    view.insertText(&NSString::from_str("x"));
    assert_eq!(*client.inserted.borrow(), ["x"]);
}

/// Ranges, positions and geometry are the client's: `UITextInput`'s
/// selection, text, position arithmetic and rectangles all read the
/// document, and a selection `UIKit` sets lands on the client.
fn selection_and_geometry_queries_answer_from_the_client() {
    let (view, client) = view_over("hello");
    client.select(1..3);
    assert_eq!(
        view.selectedTextRange().map(|range| offsets(&range)),
        Some((1, 3))
    );

    let range = view
        .textRangeFromPosition_toPosition(&view.beginningOfDocument(), &view.endOfDocument())
        .expect("both positions are the view's own");
    assert_eq!(offsets(&range), (0, 5));
    assert_eq!(
        view.textInRange(&range).map(|text| text.to_string()),
        Some(String::from("hello"))
    );

    view.setSelectedTextRange(Some(&range));
    assert_eq!(client.selection(), 0..5);

    let end = view.endOfDocument();
    assert!(
        view.positionFromPosition_offset(&end, 1).is_none(),
        "a position past the document end does not exist"
    );
    let moved = view
        .positionFromPosition_offset(&end, -2)
        .expect("two units back stays inside the document");
    assert_eq!(view.offsetFromPosition_toPosition(&moved, &end), 2);

    let rect = view.caretRectForPosition(&moved);
    assert_eq!((rect.origin.x, rect.size.height), (30.0, 20.0));
    let first = view.firstRectForRange(&range);
    assert_eq!((first.origin.x, first.size.width), (0.0, 10.0));
    let rects = view.selectionRectsForRange(&range);
    assert_eq!(rects.len(), 2);
    assert!(rects.objectAtIndex(0).containsStart());
    assert!(!rects.objectAtIndex(0).containsEnd());
    assert!(rects.objectAtIndex(1).containsEnd());

    let closest = view
        .closestPositionToPoint(CGPoint::new(31.0, 5.0))
        .expect("the client answers every point");
    assert_eq!(
        view.offsetFromPosition_toPosition(&view.beginningOfDocument(), &closest),
        3
    );
}

/// `UITextInputTraits` are the client's, read when `UIKit` asks: the keyboard
/// type, return key, secure entry, autocorrection, capitalization and
/// content type, with `nil` for free text.
fn traits_come_from_the_client() {
    use cocoa_ui::ContentType;
    use cocoa_ui::objc2_ui_kit::{
        UIKeyboardType, UIReturnKeyType, UITextAutocapitalizationType, UITextAutocorrectionType,
        UITextContentTypePassword, UITextInputTraits,
    };

    let (view, client) = view_over("");
    assert_eq!(view.keyboardType(), UIKeyboardType::Default);
    let free_text: Option<Retained<NSString>> =
        // SAFETY: `textContentType` returns an optional string.
        unsafe { msg_send![&*view, textContentType] };
    assert!(free_text.is_none(), "free text has no content type");

    client.traits.set(TextInputTraits {
        keyboard_type: KeyboardType::EmailAddress,
        return_key: ReturnKey::Send,
        secure: true,
        autocorrection: Autocorrection::Disabled,
        autocapitalization: Autocapitalization::None,
        content_type: Some(ContentType::Password),
    });
    assert_eq!(view.keyboardType(), UIKeyboardType::EmailAddress);
    assert_eq!(view.returnKeyType(), UIReturnKeyType::Send);
    assert!(view.isSecureTextEntry());
    assert_eq!(view.autocorrectionType(), UITextAutocorrectionType::No);
    assert_eq!(
        view.autocapitalizationType(),
        UITextAutocapitalizationType::None
    );
    let content: Option<Retained<NSString>> =
        // SAFETY: `textContentType` returns an optional string.
        unsafe { msg_send![&*view, textContentType] };
    assert_eq!(
        content.map(|content| content.to_string()),
        // SAFETY: a system constant.
        Some(unsafe { UITextContentTypePassword }.to_string())
    );

    // Not editing: no text, no text-system routing.
    client.editing.set(false);
    client.replace(0..0, "x");
    assert!(!view.hasText());
}

/// Without a client the view is an empty document that edits nothing.
fn an_input_view_without_a_client_is_an_empty_document() {
    let view = InputView::new(marker());
    assert!(!view.hasText());
    view.insertText(&NSString::from_str("ignored"));
    view.deleteBackward();
    assert_eq!(
        view.selectedTextRange().map(|range| offsets(&range)),
        Some((0, 0))
    );
    assert_eq!(view.markedTextRange(), None);
    let whole = view
        .textRangeFromPosition_toPosition(&view.beginningOfDocument(), &view.endOfDocument())
        .expect("both positions are the view's own");
    assert_eq!(offsets(&whole), (0, 0));
}

/// The ivars of a [`SeededTouch`].
#[derive(Debug)]
struct SeededTouchIvars {
    location: Cell<CGPoint>,
}

cocoa_ui::objc2::define_class!(
    // SAFETY: `UITouch` permits subclassing for input the process itself
    // generates; the overrides answer seeded values.
    #[unsafe(super(UITouch))]
    #[name = "CocoaUiNativeTestSeededTouch"]
    #[thread_kind = cocoa_ui::objc2::MainThreadOnly]
    #[ivars = SeededTouchIvars]
    /// A direct `UITouch` at a seeded location — a headless process cannot
    /// make `UIKit` produce one.
    struct SeededTouch;

    impl SeededTouch {
        // SAFETY: `locationInView:`'s signature.
        #[unsafe(method(locationInView:))]
        fn location_in_view(&self, _view: Option<&UIView>) -> CGPoint {
            self.ivars().location.get()
        }

        // SAFETY: `type`'s signature.
        #[unsafe(method(type))]
        fn touch_type(&self) -> UITouchType {
            UITouchType::Direct
        }
    }
);

impl SeededTouch {
    fn new(mtm: MainThreadMarker, location: CGPoint) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SeededTouchIvars {
            location: Cell::new(location),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Each touch reports for itself, and a system cancellation is a
/// `Cancelled` phase — never a release, which would commit whatever the
/// touch had started.
fn a_cancelled_touch_is_reported_as_cancelled_not_released() {
    use cocoa_ui::input::{PointerButtons, SurfaceEvent, TouchKind, TouchPhase};
    use cocoa_ui::objc2_foundation::NSSet;
    use cocoa_ui::objc2_ui_kit::UIEvent;

    let mtm = marker();
    let view = InputView::new(mtm);
    let events = Rc::new(RefCell::new(Vec::new()));
    view.set_event_handler({
        let events = Rc::clone(&events);
        move |event| events.borrow_mut().push(event)
    });
    let touch = SeededTouch::new(mtm, CGPoint::new(12.0, 34.0));
    let touches = NSSet::from_retained_slice(&[touch.into_super()]);
    let no_event = Option::<&UIEvent>::None;
    // SAFETY: the three touch callbacks take a set of `UITouch`es and an
    // optional event.
    unsafe {
        let _: () = msg_send![&*view, touchesBegan: &*touches, withEvent: no_event];
        let _: () = msg_send![&*view, touchesMoved: &*touches, withEvent: no_event];
        let _: () = msg_send![&*view, touchesCancelled: &*touches, withEvent: no_event];
    }

    let events = events.borrow();
    let touch_events: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SurfaceEvent::Touch {
                id,
                phase,
                position,
                kind,
                buttons,
            } => Some((*id, *phase, *position, *kind, *buttons)),
            _ => None,
        })
        .collect();
    assert_eq!(touch_events.len(), 3, "one event per callback: {events:?}");
    let id = touch_events[0].0;
    for (touch_id, _, position, kind, buttons) in &touch_events {
        assert_eq!(*touch_id, id, "a touch keeps its identity for its life");
        assert_eq!((position.x, position.y), (12.0, 34.0));
        assert_eq!(*kind, TouchKind::Direct);
        assert_eq!(*buttons, PointerButtons::empty());
    }
    assert_eq!(
        touch_events.iter().map(|touch| touch.1).collect::<Vec<_>>(),
        [TouchPhase::Began, TouchPhase::Moved, TouchPhase::Cancelled]
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, SurfaceEvent::PointerButton { pressed: false, .. })),
        "a cancellation must not look like a release"
    );
}

/// The layout handler runs after a layout pass of the view, and the
/// safe-area handler carries the view's insets.
fn layout_and_safe_area_handlers_follow_the_view() {
    let mtm = marker();
    let view = InputView::new(mtm);
    let layouts = Rc::new(Cell::new(0));
    view.set_layout_handler({
        let layouts = Rc::clone(&layouts);
        move || layouts.set(layouts.get() + 1)
    });
    let insets = Rc::new(Cell::new(None));
    view.set_safe_area_handler({
        let insets = Rc::clone(&insets);
        move |value| insets.set(Some(value))
    });

    view.setFrame(CGRect::new(CGPoint::ZERO, CGSize::new(100.0, 50.0)));
    view.setNeedsLayout();
    view.layoutIfNeeded();
    assert!(layouts.get() >= 1, "a laid-out view reports its layout");

    // SAFETY: `safeAreaInsetsDidChange` is the plain `UIView` callback.
    unsafe {
        let _: () = msg_send![&*view, safeAreaInsetsDidChange];
    }
    assert_eq!(insets.get(), Some(view.safe_area_insets()));
}

/// A paused frame clock keeps its link and its request but ticks nothing;
/// the setting is the clock's, so a re-selection keeps it.
fn a_paused_frame_clock_stays_armed_but_silent() {
    use cocoa_ui::display_link::FrameClock;

    let mtm = marker();
    let host = HostView::new(mtm, Rect::new(0.0, 0.0, 100.0, 100.0));
    let _window = attach(mtm, &host);
    let clock = FrameClock::new(mtm, || {});
    assert!(!clock.is_paused());
    clock.set_paused(true);
    clock.start(&host);
    assert!(clock.is_paused(), "starting does not clear the pause");
    clock.reselect(&host);
    assert!(clock.is_paused(), "re-selecting does not clear the pause");
    clock.set_paused(false);
    assert!(!clock.is_paused());
    clock.stop();
}

/// The ivars of a [`SeededPan`].
#[derive(Debug)]
struct SeededPanIvars {
    /// What `translationInView:` reports — `UIKit` only answers a real
    /// recognizer's translation while it tracks input, so the seeded
    /// subclass answers from this cell.
    translation: Cell<CGPoint>,
    /// What `locationInView:` reports.
    location: Cell<CGPoint>,
    /// What `state` reports.
    state: Cell<UIGestureRecognizerState>,
}

cocoa_ui::objc2::define_class!(
    // SAFETY: `UIPanGestureRecognizer` permits subclassing for input the
    // process itself generates; every override below answers a seeded
    // value from ivars instead of touching `UIKit` internals.
    #[unsafe(super(UIPanGestureRecognizer))]
    #[name = "CocoaUiNativeTestSeededPan"]
    #[thread_kind = cocoa_ui::objc2::MainThreadOnly]
    #[ivars = SeededPanIvars]
    /// A `UIPanGestureRecognizer` whose translation, location and state
    /// come from cells — a headless process cannot make `UIKit` report
    /// scroll input to a recognizer, and a stock recognizer answers zero
    /// translation while untracking.
    struct SeededPan;

    impl SeededPan {
        // SAFETY: `translationInView:`'s signature; the view argument is
        // ignored — the seeded value is already in the probe view's frame.
        #[unsafe(method(translationInView:))]
        fn translation_in_view(&self, _view: Option<&UIView>) -> CGPoint {
            self.ivars().translation.get()
        }

        // SAFETY: `setTranslation:inView:`'s signature; the scroll path
        // resets the delta by writing zero here.
        #[unsafe(method(setTranslation:inView:))]
        fn set_translation_in_view(&self, translation: CGPoint, _view: Option<&UIView>) {
            self.ivars().translation.set(translation);
        }

        // SAFETY: `locationInView:`'s signature.
        #[unsafe(method(locationInView:))]
        fn location_in_view(&self, _view: Option<&UIView>) -> CGPoint {
            self.ivars().location.get()
        }

        // SAFETY: `state`'s signature.
        #[unsafe(method(state))]
        fn state(&self) -> UIGestureRecognizerState {
            self.ivars().state.get()
        }
    }
);

impl SeededPan {
    /// A pan recognizer seeded with `location`, zero translation and the
    /// `Changed` state.
    fn new(mtm: MainThreadMarker, location: CGPoint) -> Retained<Self> {
        let ivars = SeededPanIvars {
            translation: Cell::new(CGPoint::ZERO),
            location: Cell::new(location),
            state: Cell::new(UIGestureRecognizerState::Changed),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `initWithTarget:action:` accepts a null target/action
        // pair; nothing dispatches to it.
        unsafe {
            msg_send![
                super(this),
                initWithTarget: std::ptr::null::<AnyObject>(),
                action: Option::<Sel>::None
            ]
        }
    }
}

/// GPU-surface scroll input on `UIKit` (#2068): indirect scrolling
/// reaches a view as `UIPanGestureRecognizer`s — a trackpad glide through
/// the `Continuous` scroll type, a wheel notch through `Discrete`. A glide
/// lands as a phased `SurfaceEvent::TrackpadPan` and a notch as a
/// `SurfaceEvent::Scroll`, each with the pan's incremental translation
/// negated into the W3C content-motion sign — always a logical-point
/// displacement, whatever the mask. A finger drag is pointer input, not a
/// scroll: both recognizers take zero touches.
#[allow(clippy::too_many_lines)] // The wiring check plus the five scroll probes it verifies.
fn indirect_scroll_input_arrives_as_surface_scroll_events() {
    use cocoa_ui::input::{GesturePhase, ScrollUnit, SurfaceEvent};
    use cocoa_ui::objc2_ui_kit::{UIPanGestureRecognizer, UIScrollTypeMask};

    let mtm = marker();
    let view = InputView::new(mtm);
    let events = Rc::new(RefCell::new(Vec::new()));
    view.set_event_handler({
        let events = Rc::clone(&events);
        move |event| events.borrow_mut().push(event)
    });

    // The wiring: one pan recognizer per indirect scroll type, no
    // touches, no touch cancellation — a drag stays pointer input.
    let recognizers = view.gestureRecognizers().unwrap_or_default();
    let pans: Vec<Retained<UIPanGestureRecognizer>> = recognizers
        .into_iter()
        .filter_map(|recognizer| recognizer.downcast::<UIPanGestureRecognizer>().ok())
        .collect();
    let pan_for = |mask| {
        pans.iter()
            .find(|pan| pan.allowedScrollTypesMask() == mask)
            .unwrap_or_else(|| panic!("no pan recognizer admits {mask:?}"))
    };
    for pan in [
        pan_for(UIScrollTypeMask::Continuous),
        pan_for(UIScrollTypeMask::Discrete),
    ] {
        assert_eq!(
            pan.maximumNumberOfTouches(),
            0,
            "a scroll recognizer took touches — a finger drag would scroll"
        );
        assert!(!pan.cancelsTouchesInView());
    }

    // A seeded recognizer carries the translation the action reads;
    // driving the probe is the same path `scrollContinuous:` takes when
    // `UIKit` reports a glide.
    let gesture = SeededPan::new(mtm, CGPoint::new(40.0, 24.0));
    let this: &UIView = &view;
    gesture.setTranslation_inView(CGPoint::new(12.0, -30.0), Some(this));
    view.scroll_probe_for_test(&gesture, UIScrollTypeMask::Continuous);

    // The action consumes the incremental translation: the delta a
    // recognizer reports is since its own last callback.
    assert_eq!(gesture.translationInView(Some(this)), CGPoint::ZERO);
    {
        let events = events.borrow();
        let [event] = events.as_slice() else {
            panic!("expected one scroll event, got {}", events.len());
        };
        let SurfaceEvent::TrackpadPan {
            position,
            delta_x,
            delta_y,
            phase,
        } = event
        else {
            panic!("expected a trackpad pan event, got {event:?}");
        };
        assert_eq!((position.x, position.y), (40.0, 24.0));
        // A pan's translation counts pointer motion — positive right and
        // down; `delta_x`/`delta_y` count content motion — positive left
        // and up.
        assert_eq!((*delta_x, *delta_y), (-12.0, 30.0));
        assert_eq!(
            *phase,
            GesturePhase::Changed,
            "a mid-gesture callback was not a Changed phase"
        );
    }

    // The gesture's terminal callback carries `finished`.
    gesture.setTranslation_inView(CGPoint::new(4.0, 0.0), Some(this));
    gesture.ivars().state.set(UIGestureRecognizerState::Ended);
    let before = events.borrow().len();
    view.scroll_probe_for_test(&gesture, UIScrollTypeMask::Continuous);
    assert_eq!(
        gesture.translationInView(Some(this)),
        CGPoint::ZERO,
        "the terminal callback did not consume its translation"
    );
    {
        let events = events.borrow();
        assert_eq!(
            events.len(),
            before + 1,
            "the Ended callback did not append exactly one event"
        );
        let [.., event] = events.as_slice() else {
            panic!("expected a second scroll event, got {}", events.len());
        };
        let SurfaceEvent::TrackpadPan { delta_x, phase, .. } = event else {
            panic!("expected a trackpad pan event, got {event:?}");
        };
        assert_eq!(*delta_x, -4.0);
        assert_eq!(*phase, GesturePhase::Ended);
    }

    // Cancelled and Failed both end a glide as a cancellation.
    for terminal in [
        UIGestureRecognizerState::Cancelled,
        UIGestureRecognizerState::Failed,
    ] {
        gesture.ivars().state.set(terminal);
        gesture.setTranslation_inView(CGPoint::new(2.0, 0.0), Some(this));
        let before = events.borrow().len();
        view.scroll_probe_for_test(&gesture, UIScrollTypeMask::Continuous);
        assert_eq!(
            gesture.translationInView(Some(this)),
            CGPoint::ZERO,
            "a {terminal:?} callback did not consume its translation"
        );
        let events = events.borrow();
        assert_eq!(
            events.len(),
            before + 1,
            "a {terminal:?} callback did not append exactly one event"
        );
        let [.., event] = events.as_slice() else {
            panic!("expected another scroll event, got {}", events.len());
        };
        let SurfaceEvent::TrackpadPan {
            delta_x,
            delta_y,
            phase,
            ..
        } = event
        else {
            panic!("expected a trackpad pan event, got {event:?}");
        };
        assert_eq!((*delta_x, *delta_y), (-2.0, 0.0));
        assert_eq!(
            *phase,
            GesturePhase::Cancelled,
            "a {terminal:?} callback did not cancel the glide"
        );
    }

    // A wheel notch is complete on its own: a logical-point
    // `Pixel` displacement, `finished` on every callback — even
    // mid-`Changed`.
    gesture.ivars().state.set(UIGestureRecognizerState::Changed);
    gesture.setTranslation_inView(CGPoint::new(0.0, 5.0), Some(this));
    let before = events.borrow().len();
    view.scroll_probe_for_test(&gesture, UIScrollTypeMask::Discrete);
    assert_eq!(
        gesture.translationInView(Some(this)),
        CGPoint::ZERO,
        "the discrete callback did not consume its translation"
    );
    {
        let events = events.borrow();
        assert_eq!(
            events.len(),
            before + 1,
            "the discrete callback did not append exactly one event"
        );
        let [.., event] = events.as_slice() else {
            panic!("expected a fifth scroll event, got {}", events.len());
        };
        let SurfaceEvent::Scroll {
            delta_y,
            unit,
            finished,
            ..
        } = event
        else {
            panic!("expected a scroll event, got {event:?}");
        };
        assert_eq!(*delta_y, -5.0);
        assert_eq!(*unit, ScrollUnit::Pixel);
        assert!(*finished, "a wheel notch is a complete scroll on its own");
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

/// The native raster pass reads the live layer tree without claiming
/// it: two cached `ViewCapture`s bound to nested views run child→parent
/// capture cycles, and after each the views report the same parent,
/// sibling order, bounds, center, transform, and hidden flag as before
/// — and they keep answering them after both cached raster frames are
/// released on a later main-queue turn. This is the nested-capture
/// shape the containment contract exists for.
#[allow(clippy::too_many_lines)] // The nested fixture plus the containment contract it checks.
fn a_capture_preserves_containment_and_survives_release() {
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
    // The nested capture: a second capturable subtree inside `content`,
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
    let content_capture = Rc::new(ViewCapture::new(mtm, content.clone()));
    content_capture.set_on_redraw(|| {});
    let child_capture = Rc::new(ViewCapture::new(mtm, child.clone()));
    child_capture.set_on_redraw(|| {});

    // Everything the tree must keep through a capture, checked after
    // every cycle and again after both cached raster frames are
    // released.
    let check = || {
        let restored_parent = content
            .superview()
            .expect("the capture left the view detached from its parent");
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
            .expect("the nested capture left the child detached");
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
            "the capture must preserve the layer's full transform, perspective included"
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

    // Three nested cycles: child capture first (the deepest one), then
    // the enclosing content capture — each against the same cached
    // raster frames, each proven by its own completed, successful fence.
    for _cycle in 0..3 {
        let (flag, complete) = fence_flag();
        child_capture.capture(&target, 0, complete);
        await_fence(&flag, "child");
        // The child frame carried the child's own colors and glyphs.
        let texels = readback(&target);
        assert!(
            count_pixels(&texels, [0, 110, 220, 255], [60, 200, 255, 255]) > 1_000,
            "the child capture must render the child's color"
        );
        assert!(
            count_pixels(&texels, [220, 220, 220, 255], [255, 255, 255, 255]) > 20,
            "the child capture must render real label glyphs"
        );

        let (flag, complete) = fence_flag();
        content_capture.capture(&target, 0, complete);
        await_fence(&flag, "content");
        let texels = readback(&target);
        assert!(
            count_pixels(&texels, [200, 0, 0, 255], [255, 60, 60, 255]) > 5_000,
            "the content capture must render the content's color"
        );
        assert!(
            count_pixels(&texels, [0, 110, 220, 255], [60, 200, 255, 255]) > 1_000,
            "the content capture must render the nested child's color"
        );
        assert!(
            count_pixels(&texels, [220, 220, 220, 255], [255, 255, 255, 255]) > 20,
            "the content capture must render real label glyphs"
        );

        check();
    }

    // Releasing the cached raster frames must not invalidate the layers
    // they drew: on the next real main-queue turn both trees answer,
    // still attached, and tear down normally.
    content_capture.shutdown();
    child_capture.shutdown();
    drop(content_capture);
    drop(child_capture);
    crate::harness::pump_main_turn();
    check();
    assert!(
        content.layer().superlayer().is_some(),
        "the backing layer lost its parent when the raster frame dropped"
    );
    assert!(
        child.layer().superlayer().is_some(),
        "the nested backing layer lost its parent when the raster frame dropped"
    );

    content.removeFromSuperview();
    sibling.removeFromSuperview();
    parent.removeFromSuperview();
}

/// Capturing a view with no superview is supported — no containment is
/// disturbed, the frame still renders real content, and teardown after
/// the cached raster frame's release stays clean.
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
    let capture = Rc::new(ViewCapture::new(mtm, content.clone()));
    capture.set_on_redraw(|| {});
    let (flag, complete) = crate::harness::fence_flag();
    capture.capture(&target, 0, complete);
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
