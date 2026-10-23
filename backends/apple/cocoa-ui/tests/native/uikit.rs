//! `UIKit` cases — real `UIKit` objects built through the public API on the
//! real main thread.

use std::cell::Cell;
use std::rc::Rc;

use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::{AnyObject, Bool, Sel};
use cocoa_ui::objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, msg_send, sel};
use cocoa_ui::objc2_core_foundation::{CGPoint, CGRect, CGSize};
use cocoa_ui::objc2_ui_kit::{UIGestureRecognizerState, UIPanGestureRecognizer, UIView, UIWindow};
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
/// the `Continuous` scroll type, a wheel notch through `Discrete` — and
/// each must land as a `SurfaceEvent::Scroll` with the pan's incremental
/// translation negated into the W3C content-motion sign — always a
/// logical-point `Pixel` displacement, whatever the mask — and `finished`
/// per gesture kind. A finger drag is
/// pointer input, not a scroll: both recognizers take zero touches.
#[allow(clippy::too_many_lines)] // The wiring check plus the five scroll probes it verifies.
fn indirect_scroll_input_arrives_as_surface_scroll_events() {
    use std::cell::RefCell;

    use cocoa_ui::input::{ScrollUnit, SurfaceEvent};
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
        let SurfaceEvent::Scroll {
            position,
            delta_x,
            delta_y,
            unit,
            finished,
        } = event
        else {
            panic!("expected a scroll event, got {event:?}");
        };
        assert_eq!((position.x, position.y), (40.0, 24.0));
        // A pan's translation counts pointer motion — positive right and
        // down; `delta_x`/`delta_y` count content motion — positive left
        // and up.
        assert_eq!((*delta_x, *delta_y), (-12.0, 30.0));
        assert_eq!(*unit, ScrollUnit::Pixel);
        assert!(
            !finished,
            "a mid-gesture callback marked the scroll finished"
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
        let SurfaceEvent::Scroll {
            delta_x,
            unit,
            finished,
            ..
        } = event
        else {
            panic!("expected a scroll event, got {event:?}");
        };
        assert_eq!(*delta_x, -4.0);
        assert_eq!(*unit, ScrollUnit::Pixel);
        assert!(*finished, "a glide's end did not mark the scroll finished");
    }

    // Cancelled and Failed terminate a glide the same way Ended does.
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
        let SurfaceEvent::Scroll {
            delta_x,
            delta_y,
            unit,
            finished,
            ..
        } = event
        else {
            panic!("expected a scroll event, got {event:?}");
        };
        assert_eq!((*delta_x, *delta_y), (-2.0, 0.0));
        assert_eq!(*unit, ScrollUnit::Pixel);
        assert!(
            *finished,
            "a {terminal:?} callback did not mark the scroll finished"
        );
    }

    // A wheel notch is complete on its own: the same logical-point
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
