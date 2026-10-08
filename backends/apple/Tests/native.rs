//! Native tests for the Rust `AppKit`/`UIKit` backend — real platform
//! objects on their actual main thread. Metal presentation trials use a real
//! `AppKit` application event loop and window on the test machine.
//!
//! These cases create `NSView`/`NSWindow`/`UIView` objects, which
//! `MainThreadMarker`-protected APIs only allow on the process's actual
//! main thread. The stock harness runs cases on worker threads, so this
//! target is `harness = false`: [`libtest_mimic`] gives it the libtest CLI
//! that nextest enumerates (`--list`, `--exact`, one process per case), and
//! `test_threads = 1` makes it run every case on `main`, where
//! [`MainThreadMarker::new`] answers `Some`.
//!
//! Everything here goes through the backend's public typed surfaces —
//! `dispatch::install`/`dispatch::render`, `windows::bind_root_window`,
//! `contract::NativeLeaf` — plus the `cocoa-ui` kit API, so the suite
//! exercises exactly what a host embedding the backend could.

// The suite only exists on the crate's supported targets.
#![cfg(any(target_os = "macos", target_os = "ios"))]

use cocoa_ui::{MainThreadMarker, PlatformView, Retained};
use libtest_mimic::{Arguments, Trial};

mod migration;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, Label};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HostView, Label};

fn main() {
    let mut args = Arguments::from_args();
    // `AppKit`/`UIKit` objects may only be built on the real main thread;
    // `run` executes sequentially in the calling thread at one thread.
    args.test_threads = Some(1);
    // Process-global startup (executors, tracing dispatcher) installs
    // once here on the same true main thread the trials run on — the
    // gpu-surface fixture mounts through it.
    #[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
    waterui_apple::native_test_support::gpu_surface::initialize_process();
    #[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
    if !args.list {
        use cocoa_ui::appkit::{ActivationPolicy, Application, ApplicationHandlers};
        let app = Application::shared(mtm());
        let _policy_accepted = app.set_activation_policy(ActivationPolicy::Regular);
        app.run(ApplicationHandlers::new().did_finish_launching(move |mtm| {
            // Run within the AppKit application lifecycle. Trials dispatch
            // native application events; a GCD block around the whole suite
            // would prevent reentrant main-queue completion delivery.
            let _ = mtm;
            libtest_mimic::run(&args, trials()).exit();
        }));
        unreachable!("the native trial runner exits the process");
    }
    libtest_mimic::run(&args, trials()).exit();
}

fn trials() -> Vec<Trial> {
    let mut tests = base_trials();
    #[cfg(all(target_os = "macos", feature = "native-test"))]
    {
        tests.extend([
            Trial::test("window::manager_installs_into_the_environment", || {
                window::manager_installs_into_the_environment(mtm());
                Ok(())
            }),
            Trial::test("window::bind_root_window_wires_a_live_window", || {
                window::bind_root_window_wires_a_live_window(mtm());
                Ok(())
            }),
            Trial::test(
                "menus::the_window_menu_opens_with_the_standard_close_item",
                || {
                    menus::the_window_menu_opens_with_the_standard_close_item();
                    Ok(())
                },
            ),
            Trial::test(
                "menus::a_pull_down_validates_its_callback_rows_from_their_command",
                || {
                    menus::a_pull_down_validates_its_callback_rows_from_their_command();
                    Ok(())
                },
            ),
        ]);
        #[cfg(feature = "gpu_surface")]
        {
            tests.extend(gpu_surface::trials());
            tests.extend(filtered::trials());
        }
    }
    #[cfg(target_os = "ios")]
    {
        tests.extend(tabs::trials());
        tests.extend(navigation::trials());
        tests.extend(controller_bounds::trials());
        tests.extend(safe_area::trials());
    }
    tests.extend(migration::trials());
    tests.extend(owner_lifetimes::trials());
    tests.extend(scroll::trials());
    tests.extend(list_scroll::trials());
    tests
}

fn base_trials() -> Vec<Trial> {
    vec![
        Trial::test("leaf::mount_attaches_and_unmount_detaches", || {
            leaf::mount_attaches_and_unmount_detaches();
            Ok(())
        }),
        Trial::test("leaf::dropping_mounted_detaches_the_view", || {
            leaf::dropping_mounted_detaches_the_view();
            Ok(())
        }),
        Trial::test("leaf::bind_applies_now_and_on_every_change", || {
            leaf::bind_applies_now_and_on_every_change();
            Ok(())
        }),
        Trial::test("leaf::mounting_installs_the_intrinsic_measure", || {
            leaf::mounting_installs_the_intrinsic_measure();
            Ok(())
        }),
        Trial::test("leaf::a_layout_pass_applies_the_handler_frame", || {
            leaf::a_layout_pass_applies_the_handler_frame();
            Ok(())
        }),
        Trial::test("resolve::unit_view_maps_to_a_hidden_empty_host", || {
            resolve::unit_view_maps_to_a_hidden_empty_host();
            Ok(())
        }),
        Trial::test("resolve::a_string_maps_to_the_text_leaf", || {
            resolve::a_string_maps_to_the_text_leaf();
            Ok(())
        }),
        Trial::test("resolve::spacer_maps_to_a_stretching_host", || {
            resolve::spacer_maps_to_a_stretching_host();
            Ok(())
        }),
        Trial::test("resolve::opacity_metadata_wraps_the_child", || {
            resolve::opacity_metadata_wraps_the_child();
            Ok(())
        }),
        Trial::test("resolve::an_unclaimed_metadata_view_panics", || {
            resolve::an_unclaimed_metadata_view_panics();
            Ok(())
        }),
        Trial::test("resolve::ignorable_metadata_renders_its_content", || {
            resolve::ignorable_metadata_renders_its_content();
            Ok(())
        }),
        Trial::test("resolve::a_native_with_fallback_resolves_to_it", || {
            resolve::a_native_with_fallback_resolves_to_it();
            Ok(())
        }),
        Trial::test(
            "resolve::unclaimed_wrappers_panic_while_claimed_render",
            || {
                resolve::unclaimed_wrappers_panic_while_claimed_render();
                Ok(())
            },
        ),
        Trial::test(
            "resolve::control_leaves_answer_their_intrinsic_height",
            || {
                resolve::control_leaves_answer_their_intrinsic_height();
                Ok(())
            },
        ),
        Trial::test(
            "picture::a_laid_out_picture_rasterizes_at_its_bounds",
            || {
                picture::a_laid_out_picture_rasterizes_at_its_bounds();
                Ok(())
            },
        ),
    ]
}

/// The marker the whole suite builds objects under — the real one, on the
/// thread `main` runs on.
fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("the custom harness runs cases on the process's main thread")
}

/// The window `attach` hands back, per kit — the scroll suites share it.
#[cfg(target_os = "macos")]
type AttachedWindow = cocoa_ui::appkit::Window;
#[cfg(target_os = "ios")]
type AttachedWindow = Retained<cocoa_ui::objc2_ui_kit::UIWindow>;

/// Attaches `content` to a harness window and orders it in — the frame
/// clock ticks only for a view on a screen, so both scroll suites mount
/// this way.
fn mount_and_order_front(content: &PlatformView) -> AttachedWindow {
    let window = leaf::attach(mtm(), content);
    #[cfg(target_os = "macos")]
    window.order_front();
    #[cfg(target_os = "ios")]
    window.makeKeyAndVisible();
    window
}

/// A landed scroll writes the computed target to the view's model
/// verbatim, so where it ended compares exactly — `f64::to_bits`
/// asserts the same equality `assert_eq!` did, without
/// `clippy::float_cmp`.
fn assert_offset_eq(actual: f64, expected: f64, msg: &str) {
    assert_eq!(actual.to_bits(), expected.to_bits(), "{msg}");
}

/// Pumps the main run loop until `in_flight` reports the animation done,
/// collecting the offset sampled each pass — a flight's trajectory and
/// its completion signal in one wait.
///
/// The pump itself is the completion signal: a flight that never ends
/// fails the case instead of hanging it. Springs overshoot, so the done
/// condition is the animation's own report, never proximity to the
/// target.
fn pump_flight(in_flight: impl Fn() -> bool, offset: impl Fn() -> f64) -> Vec<f64> {
    let samples = std::cell::RefCell::new(Vec::new());
    let landed = waterui_apple::native_test_support::pump_main_until(
        waterui_apple::native_test_support::MAIN_QUEUE_DEADLINE,
        || {
            samples.borrow_mut().push(offset());
            !in_flight()
        },
    );
    let samples = samples.into_inner();
    assert!(
        landed,
        "the scroll animation must complete; offsets seen: {samples:?}"
    );
    samples
}

/// The distinct offsets a flight moved through, quantized to a tenth of
/// a point so sub-pixel easing steps count — a jump reports one value,
/// an animation at least three (start, intermediates, end).
fn distinct_offsets(samples: &[f64]) -> usize {
    let mut ys: Vec<u64> = samples
        .iter()
        .map(|y| (y * 10.0).round().to_bits())
        .collect();
    ys.sort_unstable();
    ys.dedup();
    ys.len()
}

/// The animation cases the scroll suites run: `default` is the
/// platform's own smooth scroll, the explicit curves ride the frame
/// clock.
///
/// The `UIKit` `Default` arm lives in the `native_app` harness: this one
/// mounts a window with no scene and no `UIApplication`, where the
/// platform's animated scroll never advances the offset, while
/// `native_app` runs its cases inside a launched application. The
/// explicit curves write the offset themselves every frame clock tick, so
/// they observe here on both kits.
fn animation_cases() -> Vec<(&'static str, waterui::animation::Animation)> {
    use std::time::Duration;

    use waterui::animation::Animation;

    #[cfg_attr(
        not(target_os = "macos"),
        expect(unused_mut, reason = "AppKit adds an entry")
    )]
    let mut cases = vec![
        (
            "bezier",
            Animation::Bezier {
                duration: Duration::from_millis(500),
                x1: 0.25,
                y1: 0.1,
                x2: 0.25,
                y2: 1.0,
            },
        ),
        (
            "spring",
            Animation::Spring {
                stiffness: 300.0,
                damping: 30.0,
            },
        ),
    ];
    #[cfg(target_os = "macos")]
    cases.insert(0, ("default", Animation::Default));
    cases
}

/// One `Trial` per named case — the table shape the scroll suites emit.
fn trial_each(named: Vec<(String, Box<dyn FnOnce() + Send>)>) -> Vec<Trial> {
    named
        .into_iter()
        .map(|(name, case)| {
            Trial::test(name, move || {
                case();
                Ok(())
            })
        })
        .collect()
}

/// Real `AppKit` input for the scroll takeover cases — events built the
/// way the window server delivers them and dispatched through `AppKit`'s
/// own entry points, never a hand-posted notification.
#[cfg(target_os = "macos")]
mod appkit_input {
    use std::cell::Cell;
    use std::rc::Rc;

    use cocoa_ui::display_link::FrameClock;
    use cocoa_ui::notification::{NotificationName, NotificationObserver, observe_object};
    use cocoa_ui::objc2_app_kit::{
        NSEvent, NSEventModifierFlags, NSEventType, NSScrollView,
        NSScrollViewDidLiveScrollNotification, NSView, NSWindow,
    };
    use cocoa_ui::objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use cocoa_ui::{MainThreadMarker, Retained};
    use objc2::runtime::AnyObject;
    use objc2::{MainThreadOnly, define_class, msg_send};
    use objc2_core_graphics::{CGEvent, CGScrollEventUnit};
    use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};

    /// A function key: its virtual key code and the `NS…FunctionKey`
    /// character `AppKit` reports for it.
    #[derive(Clone, Copy, Debug)]
    pub struct FunctionKey {
        key_code: u16,
        character: u32,
    }

    /// `kVK_PageDown` / `NSPageDownFunctionKey`.
    pub const PAGE_DOWN: FunctionKey = FunctionKey {
        key_code: 0x79,
        character: 0xF72D,
    };

    /// `kVK_End` / `NSEndFunctionKey`.
    pub const END: FunctionKey = FunctionKey {
        key_code: 0x77,
        character: 0xF72B,
    };

    /// Presses `key` in `window`: a key-down `window.sendEvent` routes to
    /// the first responder, the way the window server's event does.
    pub fn press(window: &NSWindow, key: FunctionKey) {
        let characters = NSString::from_str(
            &char::from_u32(key.character)
                .expect("function-key characters are valid code points")
                .to_string(),
        );
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::ZERO,
            NSEventModifierFlags::Function,
            0.0,
            window.windowNumber(),
            None,
            &characters,
            &characters,
            false,
            key.key_code,
        )
        .expect("AppKit builds a key-down event");
        window.sendEvent(&event);
    }

    /// Scrolls `scroll_view` by `pixels` with a legacy mouse wheel: a
    /// phase-less `CGEventCreateScrollWheelEvent2` event wrapped by
    /// `+[NSEvent eventWithCGEvent:]` and sent through the scroll view's
    /// own `scrollWheel:`. Negative `pixels` scroll toward the end.
    pub fn wheel(scroll_view: &NSScrollView, pixels: i32) {
        let cg_event =
            CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 1, pixels, 0, 0)
                .expect("CoreGraphics builds a scroll-wheel event");
        let event = NSEvent::eventWithCGEvent(&cg_event).expect("AppKit wraps the wheel event");
        assert!(
            event.phase().0 == 0 && event.momentumPhase().0 == 0,
            "a legacy wheel event carries no gesture or momentum phase"
        );
        scroll_view.scrollWheel(&event);
    }

    /// Counts the `NSScrollViewDidLiveScrollNotification`s `AppKit` posts
    /// for `scroll_view` while the observer is retained.
    pub fn count_live_scrolls(
        mtm: MainThreadMarker,
        scroll_view: &NSScrollView,
    ) -> (NotificationObserver, Rc<Cell<usize>>) {
        let count = Rc::new(Cell::new(0));
        // SAFETY: the name is a system notification constant.
        let name = NotificationName::framework(unsafe { NSScrollViewDidLiveScrollNotification });
        let observer = observe_object(mtm, &name, scroll_view, {
            let count = Rc::clone(&count);
            move || count.set(count.get() + 1)
        });
        (observer, count)
    }

    /// Whether `view`'s class keeps `AppKit`'s responsive scrolling —
    /// `+isCompatibleWithResponsiveScrolling`, which answers `NO` for a
    /// class that overrides an incompatible method such as `scrollWheel:`.
    pub fn responsive_scrolling_compatible(view: &AnyObject) -> bool {
        // SAFETY: every `NSView` class answers the class property.
        unsafe { msg_send![view.class(), isCompatibleWithResponsiveScrolling] }
    }

    define_class!(
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[name = "WaterUiNativeTestFocusTarget"]
        /// A focusable view: the key responder a document needs before
        /// the keyboard can scroll it.
        pub struct FocusTarget;

        impl FocusTarget {
            #[unsafe(method(acceptsFirstResponder))]
            fn accepts_first_responder(&self) -> bool {
                true
            }
        }
    );

    define_class!(
        #[unsafe(super(NSScrollView))]
        #[thread_kind = MainThreadOnly]
        #[name = "WaterUiNativeTestWheelScrollView"]
        /// The regression the responsive-scrolling check guards against: a
        /// scroll view overriding `scrollWheel:`.
        pub struct WheelScrollView;

        impl WheelScrollView {
            #[unsafe(method(scrollWheel:))]
            fn scroll_wheel(&self, event: &NSEvent) {
                // SAFETY: forwards the event to `NSScrollView`'s own handler.
                let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
            }
        }
    );

    impl WheelScrollView {
        /// An empty wheel-overriding scroll view.
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            // SAFETY: standard `NSScrollView` init on a main-thread class.
            unsafe { msg_send![Self::alloc(mtm), initWithFrame: NSRect::ZERO] }
        }
    }

    /// Runs the run loop until `view`'s window presents a frame — a
    /// display-link tick on the view — so the first layout, tiling, text
    /// rendering and window-server commit happen before a case starts
    /// timing anything.
    pub fn present_first_frame(mtm: MainThreadMarker, view: &NSView) {
        let ticked = Rc::new(Cell::new(false));
        let clock = FrameClock::new(mtm, {
            let ticked = Rc::clone(&ticked);
            move || ticked.set(true)
        });
        clock.start(view);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || ticked.get()),
            "the mounted window never presented a frame"
        );
        clock.stop();
    }

    /// Mid-flight, `AppKit` corrects the clip for a document growing by
    /// 500pt (`grow_document`), then for `window` growing and shrinking —
    /// none of which is user scrolling, so the flight to `target` must
    /// keep running through each. Each change comes after the flight has
    /// progressed another quarter of `target`; returns once it lands.
    pub fn assert_flight_survives_corrections(
        window: &NSWindow,
        target: f64,
        in_flight: impl Fn() -> bool,
        offset: impl Fn() -> f64,
        grow_document: impl FnOnce(),
    ) {
        let content = window.contentView().expect("the window has a content view");
        let size = content.frame().size;
        let advance = |step: u32, what: &str| {
            let reached = target * f64::from(step) / 4.0;
            assert!(
                pump_main_until(MAIN_QUEUE_DEADLINE, || offset() >= reached || !in_flight()),
                "the flight must progress toward {target}"
            );
            assert!(
                in_flight(),
                "the flight must still be running before {what}: offset {}",
                offset()
            );
        };
        let settle = |what: &str| {
            content.layoutSubtreeIfNeeded();
            assert!(
                in_flight(),
                "{what} must not cancel the flight: offset {}",
                offset()
            );
        };
        advance(1, "the document growing");
        grow_document();
        settle("the document growing");
        advance(2, "a larger window");
        window.setContentSize(NSSize::new(size.width + 120.0, size.height + 160.0));
        settle("a larger window");
        advance(3, "a smaller window");
        window.setContentSize(NSSize::new(size.width - 120.0, size.height - 160.0));
        settle("a smaller window");
        let samples = super::pump_flight(in_flight, offset);
        assert!(
            samples.len() > 1,
            "the flight must still be running after the last correction: {samples:?}"
        );
    }

    impl FocusTarget {
        /// A small focusable view.
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            // SAFETY: standard `NSView` init on a main-thread class.
            unsafe {
                msg_send![
                    Self::alloc(mtm),
                    initWithFrame: NSRect::new(NSPoint::ZERO, NSSize::new(10.0, 10.0))
                ]
            }
        }
    }
}

/// `NativeLeaf` mount/watch/bind against real views.
mod leaf {
    use std::cell::Cell;
    use std::rc::Rc;

    use waterui::reactive::binding;
    use waterui_apple::contract::NativeLeaf;
    use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

    use super::{HostView, Label, MainThreadMarker, PlatformView, mtm};

    /// A fixed-size leaf: the smallest `SubView` the mount path needs.
    pub struct TestSubView;

    impl SubView for TestSubView {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(Size::new(40.0, 20.0))
        }

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    /// Mounting adds the leaf's view to the parent's subview list, and the
    /// returned `Mounted`'s `unmount` detaches it again and hands the leaf
    /// back for reuse: mounted again, the handlers its component installed
    /// before the first mount still run.
    pub fn mount_attaches_and_unmount_detaches() {
        let mtm = mtm();
        let parent = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let _window = attach(mtm, &parent);
        let child = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let laid_out = Rc::new(Cell::new(false));
        child.set_layout_handler({
            let laid_out = Rc::clone(&laid_out);
            move |_| laid_out.set(true)
        });
        let leaf = NativeLeaf::new(&*child, TestSubView);
        let mounted = leaf.mount(&parent);
        assert_eq!(cocoa_ui::view::subviews(&parent).len(), 1);
        let leaf = mounted.unmount();
        assert!(cocoa_ui::view::superview(leaf.view()).is_none());
        assert_eq!(cocoa_ui::view::subviews(&parent).len(), 0);

        let _mounted = leaf.mount(&parent);
        laid_out.set(false);
        child.set_needs_layout();
        child.layout_if_needed();
        assert!(laid_out.get(), "a moved leaf keeps its layout handler");
    }

    /// Dropping a `Mounted` — how a container releases a replaced child —
    /// detaches the view from its superview before releasing the leaf.
    pub fn dropping_mounted_detaches_the_view() {
        let mtm = mtm();
        let parent = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let child = Label::new(mtm);
        let child_view: &PlatformView = &child;
        let mounted = NativeLeaf::new(child_view, TestSubView).mount(&parent);
        let view = cocoa_ui::view::retain_base(mounted.view());
        drop(mounted);
        assert!(cocoa_ui::view::superview(&view).is_none());
    }

    /// `bind` applies the current value immediately and every later write —
    /// the path every reactive property takes into its platform object.
    pub fn bind_applies_now_and_on_every_change() {
        let mtm = mtm();
        let host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mut leaf = NativeLeaf::new(&*host, TestSubView);
        let target = cocoa_ui::view::retain_base(leaf.view());
        let flag = binding(false);
        leaf.bind(&flag, move |value| {
            cocoa_ui::view::set_hidden(&target, value);
        });
        assert!(!cocoa_ui::view::is_hidden(&host));
        flag.set(true);
        assert!(cocoa_ui::view::is_hidden(&host));
    }

    /// A `HostView` leaf mirrors its layout face onto the view's intrinsic
    /// measure only once mounted — before it, the view answers exactly what
    /// an unattached kit host answers.
    pub fn mounting_installs_the_intrinsic_measure() {
        let mtm = mtm();
        let parent = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let unattached = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let child = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let leaf = NativeLeaf::new(&*child, TestSubView);
        assert_eq!(
            cocoa_ui::view::fitting_size(leaf.view()),
            cocoa_ui::view::fitting_size(&unattached)
        );
        let _mounted = leaf.mount(&parent);
        let fitting = cocoa_ui::view::fitting_size(&child);
        assert_eq!(fitting, cocoa_ui::geometry::Size::new(40.0, 20.0));
    }

    /// A host inside a real (never shown) window runs its layout pass, and
    /// the handler's frames land on the children — the bridge every
    /// container leans on.
    pub fn a_layout_pass_applies_the_handler_frame() {
        let mtm = mtm();
        let host = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let child = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mounted = NativeLeaf::new(&*child, TestSubView).mount(&host);
        let child_view = cocoa_ui::view::retain_base(mounted.view());
        host.set_layout_handler(move |host| {
            let host_view: &PlatformView = host;
            cocoa_ui::view::set_frame(&child_view, cocoa_ui::view::bounds(host_view));
        });
        let _window = attach(mtm, &host);
        host.set_needs_layout();
        host.layout_if_needed();
        // `set_content_view` resizes the host to the window's content
        // area, so the expected child frame is the host's real bounds —
        // the handler must land that frame verbatim.
        let expected = cocoa_ui::view::bounds(&host);
        let frame = cocoa_ui::view::frame(mounted.view());
        assert_eq!(frame.size, expected.size);
    }

    /// Puts `content` inside a real window that is never ordered in — the
    /// smallest environment in which the frameworks still run their full
    /// layout path.
    #[cfg(target_os = "macos")]
    pub fn attach(mtm: MainThreadMarker, content: &PlatformView) -> cocoa_ui::appkit::Window {
        let window = cocoa_ui::appkit::Window::new(
            mtm,
            cocoa_ui::Rect::new(0.0, 0.0, 640.0, 480.0),
            cocoa_ui::appkit::WindowStyle::TITLED | cocoa_ui::appkit::WindowStyle::CLOSABLE,
        );
        window.set_content_view(content);
        window
    }

    /// `UIKit` does not need a scene for `layoutSubviews` to run; the
    /// window exists so `window`-dependent paths see a real one.
    #[cfg(target_os = "ios")]
    pub fn attach(
        mtm: MainThreadMarker,
        content: &PlatformView,
    ) -> cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIWindow> {
        use cocoa_ui::objc2_ui_kit::UIWindow;
        use objc2::{MainThreadOnly, msg_send};

        // SAFETY: `initWithFrame:` is `UIWindow`'s plain initializer and
        // `mtm` proves the main-thread confinement the harness provides.
        let window: cocoa_ui::Retained<UIWindow> = unsafe {
            msg_send![
                UIWindow::alloc(mtm),
                initWithFrame: objc2_core_foundation::CGRect::new(
                    objc2_core_foundation::CGPoint::new(0.0, 0.0),
                    objc2_core_foundation::CGSize::new(390.0, 844.0),
                )
            ]
        };
        // Content mounts directly — the window's keyboard region needs
        // no host ancestor. `addSubview` alone leaves the content at its
        // zero frame, so surfaces that read their viewport (lazy
        // containers especially) would see an empty window — size it the
        // way `set_content_view` does on macOS.
        window.addSubview(content);
        cocoa_ui::view::set_frame(content, cocoa_ui::view::bounds(&window));
        window
    }
}

/// View → leaf mapping through `dispatch::render` — the typed entry point
/// a host reaches. A view nobody claims panics (there is no foreign caller
/// to hand it back to), so a spurious empty render could not masquerade as
/// a pass.
mod resolve {
    use waterui::Str;
    use waterui::ViewExt as _;
    use waterui::component::form::picker::{PickerItem, picker};
    use waterui::component::form::secure::{Secure, SecureField};
    use waterui::component::slider::slider;
    use waterui::component::text_field::TextField;
    use waterui::filter::Opacity;
    use waterui::layout::Spacer;
    use waterui::reactive::binding;
    use waterui_apple::contract::NativeLeaf;
    use waterui_backend_core::{AnyView, View};
    use waterui_core::layout::{ProposalSize, Size, StretchAxis};
    use waterui_core::metadata::MetadataKey;
    use waterui_core::{IgnorableMetadata, Metadata, Native, NativeView};

    use super::{HostView, Label, Retained, mtm};

    /// A `Metadata` key no handler is registered for — an honest miss,
    /// never fabricated.
    struct Unregistered;

    impl MetadataKey for Unregistered {}

    /// A `NativeView` no handler is registered for.
    struct UnclaimedNative;

    impl NativeView for UnclaimedNative {}

    /// The minimum environment a real render needs — shared with the
    /// `native_app` harness.
    pub use waterui_apple::native_test_support::render_environment as env;

    /// Renders `view` through the typed dispatch entry point, main thread,
    /// fresh env. Panics when nothing claims the view — the typed
    /// contract's answer to a miss.
    pub fn render(view: impl View) -> NativeLeaf {
        let _mtm = mtm();
        waterui_apple::dispatch::render(AnyView::new(view), &env())
    }

    /// Renders `view`, reporting whether `render` panicked instead of
    /// producing a leaf — for the cases asserting the miss path itself.
    fn render_or_panic(view: impl View) -> Option<NativeLeaf> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(view))).ok()
    }

    /// `()` lands as a hidden `HostView` that measures zero and answers
    /// `is_empty` — the leaf a stack ignores.
    pub fn unit_view_maps_to_a_hidden_empty_host() {
        let leaf = render(());
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert!(cocoa_ui::view::is_hidden(&view));
        assert!(leaf.layout().is_empty());
        assert_eq!(
            leaf.layout()
                .measure(ProposalSize::new(Some(100.0), Some(100.0)))
                .size,
            Size::new(0.0, 0.0)
        );
    }

    /// A `&'static str` expands `Str` → `Native<Str>` → the text leaf: a
    /// `HostView` wrapper holding a real kit `Label` with the string's
    /// attributed text on it — and the layout face still answers measure.
    pub fn a_string_maps_to_the_text_leaf() {
        let leaf = render("hello waterui");
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert!(!cocoa_ui::view::is_hidden(&view));
        let subviews = cocoa_ui::view::subviews(&view);
        assert_eq!(subviews.len(), 1);
        let label = subviews[0]
            .downcast_ref::<Label>()
            .expect("the text leaf mounts a kit label");
        let text = label
            .source_text()
            .expect("the label carries attributed text");
        assert_eq!(text.string().to_string(), "hello waterui");
        let measured = leaf.layout().measure(ProposalSize::UNSPECIFIED);
        assert!(measured.size.width > 0.0);
        assert!(measured.size.height > 0.0);
    }

    /// `Native<Spacer>` maps to a transparent host whose layout face
    /// stretches on the enclosing stack's main axis.
    pub fn spacer_maps_to_a_stretching_host() {
        let leaf = render(Spacer::new(12.0));
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert!(!cocoa_ui::view::is_hidden(&view));
        assert_eq!(leaf.layout().stretch_axis(), StretchAxis::MainAxis);
        assert_eq!(leaf.layout().priority(), i32::MIN);
    }

    /// `Metadata<Opacity>` is claimed by its handler: the wrapper is a
    /// `HostView` at the declared alpha with the content mounted inside.
    pub fn opacity_metadata_wraps_the_child() {
        let leaf = render(Metadata::new((), Opacity::new(0.5)));
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert_eq!(cocoa_ui::view::alpha(&view).to_bits(), 0.5f64.to_bits());
        let subviews = cocoa_ui::view::subviews(&view);
        assert_eq!(subviews.len(), 1);
        let primary = cocoa_ui::view::primary_content(&view)
            .expect("the wrapper forwards its primary content");
        assert_eq!(Retained::as_ptr(&primary), Retained::as_ptr(&subviews[0]));
    }

    /// A `Metadata` nobody claims panics in `body()` — on the typed
    /// contract nothing catches it, so the panic propagates out of
    /// `render` itself.
    pub fn an_unclaimed_metadata_view_panics() {
        assert!(render_or_panic(Metadata::new((), Unregistered)).is_none());
    }

    /// `IgnorableMetadata` is transparent: its `body()` returns the
    /// content, so an unregistered key renders through to the content's
    /// leaf.
    pub fn ignorable_metadata_renders_its_content() {
        let leaf = render(IgnorableMetadata::new((), Unregistered));
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert!(cocoa_ui::view::is_hidden(&view));
    }

    /// `Native::with_fallback` is the honest port of "not claimed here":
    /// the dispatcher expands to the embedded fallback and resolves it —
    /// here, to the spacer host — inside the one `render` call.
    pub fn a_native_with_fallback_resolves_to_it() {
        let leaf = render(Native::new(UnclaimedNative).with_fallback(Spacer::new(8.0)));
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert_eq!(leaf.layout().stretch_axis(), StretchAxis::MainAxis);
    }

    /// The typed unclaimed-view contract: a view whose `body()` panics —
    /// `Metadata`/`Native` wrappers nobody claims — propagates the panic
    /// out of `render`; composable views resolve.
    pub fn unclaimed_wrappers_panic_while_claimed_render() {
        assert!(render_or_panic(Metadata::new((), Unregistered)).is_none());
        assert!(render_or_panic(Native::new(UnclaimedNative)).is_none());
        render(());
        render(Spacer::new(8.0));
        render(IgnorableMetadata::new((), Unregistered));
    }

    /// §6's control contract: a finite height offer is advice, not an
    /// allocation. The slider, the default-style picker and both text
    /// fields answer their intrinsic height to it — a `VStack` above them
    /// cannot starve a trailing `ScrollView` by handing out space that
    /// only exists because the control claimed it.
    pub fn control_leaves_answer_their_intrinsic_height() {
        let volume = binding(0.5_f64);
        let selection = binding("Alpha");
        let text_value = binding(Str::from(""));
        let secret = binding(Secure::new(String::new()));
        let items: Vec<PickerItem<&'static str>> = vec![
            waterui::text!("Alpha").tag("Alpha"),
            waterui::text!("Beta").tag("Beta"),
            waterui::text!("Gamma").tag("Gamma"),
        ];
        let leaves = [
            ("slider", render(slider("Volume", &volume))),
            ("picker", render(picker("Letter", items, &selection))),
            ("text field", render(TextField::new("Name", &text_value))),
            (
                "secure field",
                render(SecureField::new("Password", &secret)),
            ),
        ];
        for (name, leaf) in leaves {
            let offered = leaf
                .layout()
                .measure(ProposalSize::new(Some(300.0), Some(500.0)))
                .size;
            let unspecified = leaf
                .layout()
                .measure(ProposalSize::new(Some(300.0), None))
                .size;
            assert_eq!(
                offered.height.to_bits(),
                unspecified.height.to_bits(),
                "{name} answers a finite height offer with its intrinsic height"
            );
            assert!(
                offered.height < 500.0,
                "{name} must not grow into the offered height"
            );
        }
    }
}

/// `Native<Picture>` rasterization: the bitmap is sized to the laid-out
/// bounds times the backing scale, and a relayout re-rasterizes — a picture
/// stretched past its declared size shows pixels rasterized for that size,
/// not an upscale of the declared-size bitmap.
mod picture {
    use std::cell::Cell;
    use std::rc::Rc;

    #[cfg(target_os = "macos")]
    use cocoa_ui::appkit::ImageView;
    #[cfg(target_os = "ios")]
    use cocoa_ui::uikit::ImageView;
    use kurbo::Shape;
    use waterui::graphics::color::WorkingColor;
    use waterui::graphics::draw::Draw;
    use waterui::graphics::picture::Picture;
    use waterui::reactive::constant;
    use waterui_core::layout::Size;

    use super::{HostView, PlatformView, mtm};

    /// The pixel dimensions of the bitmap the leaf's image view shows.
    #[cfg(target_os = "macos")]
    fn image_pixels(view: &PlatformView) -> (usize, usize) {
        let view = view
            .downcast_ref::<ImageView>()
            .expect("the picture leaf mounts the kit image view");
        let image = view.image().expect("the picture leaf paints an image");
        let rep = image
            .representations()
            .firstObject()
            .expect("the raster image carries one rep");
        (
            usize::try_from(rep.pixelsWide()).expect("a bitmap has nonnegative pixels"),
            usize::try_from(rep.pixelsHigh()).expect("a bitmap has nonnegative pixels"),
        )
    }

    /// The pixel dimensions of the bitmap the leaf's image view shows.
    #[cfg(target_os = "ios")]
    fn image_pixels(view: &PlatformView) -> (usize, usize) {
        let view = view
            .downcast_ref::<ImageView>()
            .expect("the picture leaf mounts the kit image view");
        let image = view.image().expect("the picture leaf paints an image");
        // SAFETY: `CGImage` is a plain property read on the main thread.
        let cg = unsafe { image.CGImage() }.expect("the raster image is a CGImage");
        (
            cocoa_ui::objc2_core_graphics::CGImage::width(Some(&cg)),
            cocoa_ui::objc2_core_graphics::CGImage::height(Some(&cg)),
        )
    }

    /// A `Picture` laid out at twice its declared size rasterizes at twice
    /// the pixel size: the leaf re-rasterizes when its bounds change rather
    /// than upscaling the bitmap painted for the declared size (#1565).
    pub fn a_laid_out_picture_rasterizes_at_its_bounds() {
        let mtm = mtm();
        let declared = Size::new(100.0, 50.0);
        let picture = Picture::new(
            declared,
            constant(Picture::record(|scene| {
                scene.fill(
                    kurbo::Rect::new(0.0, 0.0, 100.0, 50.0).to_path(0.1),
                    WorkingColor::BLACK,
                );
            })),
        );
        let leaf = crate::resolve::render(picture);

        let host = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 400.0, 300.0));
        let mounted = leaf.mount(&host);
        let child = cocoa_ui::view::retain_base(mounted.view());
        let child_size = Rc::new(Cell::new(declared));
        host.set_layout_handler({
            let child_size = Rc::clone(&child_size);
            move |_host| {
                let size = child_size.get();
                cocoa_ui::view::set_frame(
                    &child,
                    cocoa_ui::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
                );
            }
        });

        let _window = crate::leaf::attach(mtm, &host);
        // Two flushes: the parent's pass frames the child, the second runs
        // the child's own layout hook on its new bounds.
        host.set_needs_layout();
        host.layout_if_needed();
        host.layout_if_needed();
        let pixels_at_declared = image_pixels(mounted.view());
        assert!(
            pixels_at_declared.0 > 0 && pixels_at_declared.1 > 0,
            "the declared-size bitmap exists"
        );

        child_size.set(Size::new(declared.width * 2.0, declared.height * 2.0));
        host.set_needs_layout();
        host.layout_if_needed();
        host.layout_if_needed();

        assert_eq!(
            image_pixels(mounted.view()),
            (pixels_at_declared.0 * 2, pixels_at_declared.1 * 2),
            "laid out at twice its declared size, the rasterized bitmap must double"
        );
    }
}

/// `TabsLayout` chrome the iOS backend owns — the `UITabAccessory` bottom
/// slot and `tabBarMinimizeBehavior` — against a real `UITabBarController`.
/// All cases run on the true main thread under the harness and reach the
/// controller the way a host does: through the view hierarchy's responder
/// chain.
#[cfg(target_os = "ios")]
mod tabs {
    use cocoa_ui::objc2_ui_kit::{
        NSDirectionalRectEdge, UIScrollView, UITabBarController, UITabBarMinimizeBehavior,
    };
    use cocoa_ui::uikit::view_controller::owning_controller;
    use cocoa_ui::uikit::{Label, NavContentController};
    use cocoa_ui::{PlatformView, Retained, view};
    use waterui::navigation::{NavigationStack, NavigationView, Tab, TabBarMinimizeBehavior, Tabs};
    use waterui::prelude::{label, scroll, text, vstack};
    use waterui::reactive::binding;
    use waterui_apple::contract::NativeLeaf;

    use super::resolve;

    /// The `tabs::` trials, kept beside their fixtures so the top-level
    /// registry stays a one-line extension per platform.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test("tabs::pane_scroll_view_associates_for_collapse", || {
                pane_scroll_view_associates_for_collapse();
                Ok(())
            }),
            libtest_mimic::Trial::test("tabs::panes_without_surfaces_answer_none", || {
                panes_without_surfaces_answer_none();
                Ok(())
            }),
            libtest_mimic::Trial::test("tabs::pane_scroll_view_tracks_nav_pushes", || {
                pane_scroll_view_tracks_nav_pushes();
                Ok(())
            }),
            libtest_mimic::Trial::test("tabs::pane_scroll_view_tracks_dynamic_replacement", || {
                pane_scroll_view_tracks_dynamic_replacement();
                Ok(())
            }),
            libtest_mimic::Trial::test("tabs::bottom_accessory_mounts_into_the_controller", || {
                bottom_accessory_mounts_into_the_controller();
                Ok(())
            }),
            libtest_mimic::Trial::test(
                "tabs::accessory_layout_centers_the_measured_answer",
                || {
                    accessory_layout_centers_the_measured_answer();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "tabs::binding_updates_preserve_the_accessory_mount",
                || {
                    binding_updates_preserve_the_accessory_mount();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "tabs::tab_selection_does_not_rebuild_the_accessory",
                || {
                    tab_selection_does_not_rebuild_the_accessory();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "tabs::dropping_the_leaf_releases_accessory_watchers",
                || {
                    dropping_the_leaf_releases_accessory_watchers();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "tabs::each_minimize_behavior_maps_to_the_uikit_property",
                || {
                    each_minimize_behavior_maps_to_the_uikit_property();
                    Ok(())
                },
            ),
        ]
    }

    /// Tab identity for the fixtures.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Pane {
        One,
        Two,
    }

    /// The rendered leaf's first subview is the controller's root view; a
    /// `UIViewController` sits on the responder chain right after its own
    /// view — the same reach `adopt_controllers` relies on.
    fn tab_bar_controller(leaf: &NativeLeaf) -> Retained<UITabBarController> {
        let controller_view = view::subviews(leaf.view())
            .into_iter()
            .next()
            .expect("the tabs host carries the controller's view");
        owning_controller(&controller_view)
            .and_then(|responder| responder.downcast::<UITabBarController>().ok())
            .expect("the tabs leaf mounts a UITabBarController")
    }

    /// First `Label`'s text in the subtree, depth-first.
    fn label_text(view: &PlatformView) -> Option<String> {
        for sub in view::subviews(view) {
            if let Some(label) = sub.downcast_ref::<Label>()
                && let Some(attributed) = label.source_text()
            {
                return Some(attributed.string().to_string());
            }
            if let Some(found) = label_text(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// First `Label` in the subtree, retained for reads after the leaf
    /// that mounted it is gone.
    fn first_label(view: &PlatformView) -> Option<Retained<PlatformView>> {
        for sub in view::subviews(view) {
            if sub.downcast_ref::<Label>().is_some() {
                return Some(sub);
            }
            if let Some(found) = first_label(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// Two plain tabs with `accessory` in the bottom slot.
    fn tabs(
        accessory: impl waterui_backend_core::View,
    ) -> (waterui::reactive::Binding<Pane>, NativeLeaf) {
        let pane = binding(Pane::One);
        let leaf = resolve::render(
            Tabs::new(
                &pane,
                vec![
                    Tab::container(Pane::One, label("One"), || text("pane one")),
                    Tab::container(Pane::Two, label("Two"), || text("pane two")),
                ],
            )
            .bottom_accessory(accessory),
        );
        (pane, leaf)
    }

    /// The shared `bottom_accessory` view lands in the controller's own
    /// accessory slot: `UIKit` reports a `UITabAccessory` whose content
    /// view carries the rendered text at a natural height.
    pub fn bottom_accessory_mounts_into_the_controller() {
        let (_pane, leaf) = tabs(text("Now Playing"));
        let accessory = tab_bar_controller(&leaf)
            .bottomAccessory()
            .expect("the shared accessory view installs a UITabAccessory");
        let content = accessory.contentView();
        assert_eq!(label_text(&content).as_deref(), Some("Now Playing"));
        assert!(view::fitting_size(&content).height > 0.0);
    }

    /// A `Binding` inside the accessory updates the mounted text in place
    /// — the same `UITabAccessory` keeps serving the capsule.
    pub fn binding_updates_preserve_the_accessory_mount() {
        let track = binding(String::from("first"));
        let (_pane, leaf) = tabs(text!("{track}"));
        let controller = tab_bar_controller(&leaf);
        let accessory = controller.bottomAccessory().expect("installed");
        let content = accessory.contentView();
        // `text!` wraps interpolations in bidi isolates — match the payload.
        assert!(label_text(&content).is_some_and(|text| text.contains("first")));
        track.set(String::from("second"));
        assert!(label_text(&content).is_some_and(|text| text.contains("second")));
        let after = controller.bottomAccessory().expect("still installed");
        assert_eq!(Retained::as_ptr(&after), Retained::as_ptr(&accessory));
    }

    /// Selecting the other tab switches panes through the existing
    /// controller — the accessory mount is untouched.
    pub fn tab_selection_does_not_rebuild_the_accessory() {
        let (pane, leaf) = tabs(text("Now Playing"));
        let controller = tab_bar_controller(&leaf);
        let accessory = controller.bottomAccessory().expect("installed");
        pane.set(Pane::Two);
        assert_eq!(controller.selectedIndex(), 1);
        let after = controller.bottomAccessory().expect("still installed");
        assert_eq!(Retained::as_ptr(&after), Retained::as_ptr(&accessory));
    }

    /// Dropping the leaf uninstalls the capsule and releases the mounted
    /// subtree's watchers — the controller itself is owned by containment,
    /// not by the leaf, so a retained handle stays valid but must report
    /// no accessory, and no later write may reach the old label.
    pub fn dropping_the_leaf_releases_accessory_watchers() {
        let track = binding(String::from("first"));
        let (label, controller) = {
            let (_pane, leaf) = tabs(text!("{track}"));
            let controller = tab_bar_controller(&leaf);
            let label = {
                let content = controller
                    .bottomAccessory()
                    .expect("installed")
                    .contentView();
                track.set(String::from("second"));
                first_label(&content).expect("the accessory mounts a label")
            };
            drop(leaf);
            (label, controller)
        };
        assert!(controller.bottomAccessory().is_none());
        track.set(String::from("third"));
        let text = label
            .downcast_ref::<Label>()
            .and_then(Label::source_text)
            .expect("the retained label");
        let text = text.string().to_string();
        assert!(text.contains("second") && !text.contains("third"));
    }

    /// Every shared minimize-behavior variant maps onto the `UIKit`
    /// property one-to-one — no translation layer of our own.
    pub fn each_minimize_behavior_maps_to_the_uikit_property() {
        for (shared, native) in [
            (
                TabBarMinimizeBehavior::Automatic,
                UITabBarMinimizeBehavior::Automatic,
            ),
            (
                TabBarMinimizeBehavior::Never,
                UITabBarMinimizeBehavior::Never,
            ),
            (
                TabBarMinimizeBehavior::OnScrollDown,
                UITabBarMinimizeBehavior::OnScrollDown,
            ),
            (
                TabBarMinimizeBehavior::OnScrollUp,
                UITabBarMinimizeBehavior::OnScrollUp,
            ),
        ] {
            let pane = binding(Pane::One);
            let leaf = resolve::render(
                Tabs::new(
                    &pane,
                    vec![Tab::container(Pane::One, label("One"), || text("one"))],
                )
                .minimize_behavior(shared),
            );
            assert_eq!(tab_bar_controller(&leaf).tabBarMinimizeBehavior(), native);
        }
    }

    /// Every `UIScrollView` in the subtree, depth-first — each fixture
    /// pane mounts exactly one, so the association can only be that
    /// surface.
    fn scroll_views_in(view: &PlatformView) -> Vec<Retained<UIScrollView>> {
        let mut found = Vec::new();
        for sub in view::subviews(view) {
            if let Some(scroll) = sub.downcast_ref::<UIScrollView>() {
                found.push(scroll.into());
            }
            found.extend(scroll_views_in(&sub));
        }
        found
    }

    /// `UIKit` only auto-associates a scroll view that is the
    /// controller's own root, so the leaf associates each pane's declared
    /// scroll surface for the bottom edge: a scroll mounted as the root,
    /// or one the pane's container declares as a candidate.
    pub fn pane_scroll_view_associates_for_collapse() {
        let pane = binding(Pane::One);
        let leaf = resolve::render(Tabs::new(
            &pane,
            vec![
                Tab::container(Pane::One, label("One"), || {
                    scroll(vstack((text("one"), text("two"), text("three"))))
                }),
                Tab::container(Pane::Two, label("Two"), || {
                    vstack((text("aside"), scroll(text("nested"))))
                }),
            ],
        ));
        let controllers = tab_bar_controller(&leaf)
            .viewControllers()
            .expect("tabs installed");
        for (index, controller) in controllers.iter().enumerate() {
            let root = controller.view().expect("the pane's root view");
            let scrolls = scroll_views_in(&root);
            assert_eq!(
                scrolls.len(),
                1,
                "pane {index} mounts exactly one scroll surface"
            );
            let associated = controller
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .expect("the pane's scroll view associates for the bottom edge");
            assert_eq!(Retained::as_ptr(&associated), Retained::as_ptr(&scrolls[0]));
        }
    }

    /// A scroll-free pane answers exactly `None`, and a scrolled pane
    /// associates nothing for any edge but the bottom one — the tab bar
    /// only tracks a surface it actually gets.
    pub fn panes_without_surfaces_answer_none() {
        let pane = binding(Pane::One);
        let leaf = resolve::render(Tabs::new(
            &pane,
            vec![
                Tab::container(Pane::One, label("One"), || scroll(text("scrollable"))),
                Tab::container(Pane::Two, label("Two"), || text("static")),
            ],
        ));
        let controllers = tab_bar_controller(&leaf)
            .viewControllers()
            .expect("tabs installed");
        let scrolled = controllers.objectAtIndex(0);
        assert!(
            scrolled
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .is_some(),
            "the scrolled pane associates for the bottom edge"
        );
        for edge in [
            NSDirectionalRectEdge::Top,
            NSDirectionalRectEdge::Leading,
            NSDirectionalRectEdge::Trailing,
        ] {
            assert!(
                scrolled.contentScrollViewForEdge(edge).is_none(),
                "edge {edge:?} associates nothing"
            );
        }
        assert!(
            controllers
                .objectAtIndex(1)
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .is_none(),
            "a scroll-free pane associates nothing"
        );
    }

    /// A `Dynamic` pane replacement re-answers through the same declared
    /// chain: swap in fresh scroll content and the next query reports
    /// the new surface; swap to scroll-free content and the answer is
    /// `None` exactly. The replaced scroll deallocates once the pools
    /// drain — nothing in the tab controller retains it.
    pub fn pane_scroll_view_tracks_dynamic_replacement() {
        use cocoa_ui::objc2::rc::{Weak, autoreleasepool};
        use waterui::component::Dynamic;
        use waterui_backend_core::AnyView;

        let scrolled = binding(true);
        let content = scrolled.clone();
        let pane = binding(Pane::One);
        // Render inside a bounded pool: the leaf's own `Retained`/`Rc`
        // ownership survives, while every autoreleased temporary the
        // mount produced drains now — the weak read later must not meet
        // a render-time retainer in the harness's outer pool.
        let leaf = autoreleasepool(|_| {
            resolve::render(Tabs::new(
                &pane,
                vec![Tab::container(Pane::One, label("One"), move || {
                    Dynamic::watch(content.clone(), |scrolled| {
                        if scrolled {
                            AnyView::new(scroll(text("surface")))
                        } else {
                            AnyView::new(text("plain"))
                        }
                    })
                })],
            ))
        });
        let controller = autoreleasepool(|_| {
            tab_bar_controller(&leaf)
                .viewControllers()
                .expect("tabs installed")
                .objectAtIndex(0)
        });
        // Every association/query temporary drains before the swap: the
        // weak read afterwards must not meet an autoreleased retainer.
        let weak_old = autoreleasepool(|_| {
            let old_scrolls = scroll_views_in(&controller.view().expect("the pane's root view"));
            assert_eq!(old_scrolls.len(), 1, "the fixture mounts one scroll");
            let associated = controller
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .expect("the scroll associates for the bottom edge");
            assert_eq!(
                Retained::as_ptr(&associated),
                Retained::as_ptr(&old_scrolls[0])
            );
            Weak::from_retained(&old_scrolls[0])
        });

        // Replacing the pane's content inside a bounded pool keeps UIKit
        // autorelease temporaries from holding the old surface past the
        // swap — the weak read happens after the drain.
        autoreleasepool(|_| {
            scrolled.set(false);
        });
        assert!(
            autoreleasepool(|_| controller
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .is_none()),
            "a scroll-free replacement answers None"
        );
        assert!(
            weak_old.load().is_none(),
            "the replaced scroll is deallocated — nothing retained it"
        );

        // Swapping scroll content back in mounts a fresh surface the next
        // query reports — not the released one.
        autoreleasepool(|_| {
            scrolled.set(true);
            let new_scrolls = scroll_views_in(&controller.view().expect("the pane's root view"));
            assert_eq!(new_scrolls.len(), 1, "the replacement mounts one scroll");
            let associated = controller
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .expect("the new scroll associates for the bottom edge");
            assert_eq!(
                Retained::as_ptr(&associated),
                Retained::as_ptr(&new_scrolls[0])
            );
        });
    }

    /// The association answers the *current* surface on every `UIKit`
    /// query, not a pointer captured at install: a native push reports
    /// the pushed page's scroll and a pop reports the root's again.
    /// Each known scroll reference comes straight off the page
    /// controller's root — the fixture never re-walks the declared
    /// chain.
    pub fn pane_scroll_view_tracks_nav_pushes() {
        /// The `UINavigationController` owning a view in the subtree,
        /// depth-first — either the view's own controller is the nav
        /// controller, or the page controller answers one.
        fn nav_controller_in(
            view: &PlatformView,
        ) -> Option<Retained<cocoa_ui::objc2_ui_kit::UINavigationController>> {
            if let Some(controller) = owning_controller(view) {
                if let Ok(nav) = controller.clone().downcast() {
                    return Some(nav);
                }
                if let Some(nav) = controller.navigationController() {
                    return Some(nav);
                }
            }
            for sub in view::subviews(view) {
                if let Some(found) = nav_controller_in(&sub) {
                    return Some(found);
                }
            }
            None
        }

        let mtm = super::mtm();
        let pane = binding(Pane::One);
        let leaf = resolve::render(Tabs::new(
            &pane,
            vec![Tab::container(Pane::One, label("One"), || {
                NavigationStack::new(NavigationView::new("Root", scroll(text("root"))))
            })],
        ));
        let controller = tab_bar_controller(&leaf)
            .viewControllers()
            .expect("tabs installed")
            .objectAtIndex(0);
        let root = controller.view().expect("the pane's root view");
        let nav = nav_controller_in(&root).expect("the pane mounts a stack");
        let root_scrolls = scroll_views_in(
            &nav.topViewController()
                .expect("the root page")
                .view()
                .expect("the root page's view"),
        );
        assert_eq!(root_scrolls.len(), 1, "the root page mounts one scroll");
        let root_scroll = &root_scrolls[0];
        let associated = controller
            .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
            .expect("the root scroll associates for the bottom edge");
        assert_eq!(Retained::as_ptr(&associated), Retained::as_ptr(root_scroll));

        // A native push installs a page whose root view *is* its scroll
        // surface — the same shape `UITableView` pages take — and the
        // next query must answer it rather than the root's.
        let pushed_scroll = UIScrollView::new(mtm);
        let pushed = NavContentController::new(mtm, &pushed_scroll);
        nav.pushViewController_animated(&pushed, false);
        let associated = controller
            .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
            .expect("the pushed scroll associates for the bottom edge");
        assert_eq!(
            Retained::as_ptr(&associated),
            Retained::as_ptr(&pushed_scroll)
        );

        // A native pop returns the association to the root's surface.
        nav.popViewControllerAnimated(false);
        let associated = controller
            .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
            .expect("the root scroll associates again");
        assert_eq!(Retained::as_ptr(&associated), Retained::as_ptr(root_scroll));
    }

    /// The accessory's child answers the measure itself — the host
    /// centers that answer inside bounds and never clamps it: wider
    /// bounds center it, tighter bounds let it overflow centered. A
    /// stretching child fills the bounds it measured back.
    pub fn accessory_layout_centers_the_measured_answer() {
        fn child_frame(
            content: &PlatformView,
            size: cocoa_ui::geometry::Size,
        ) -> cocoa_ui::geometry::Rect {
            view::set_frame(
                content,
                cocoa_ui::geometry::Rect::new(0.0, 0.0, size.width, size.height),
            );
            content.setNeedsLayout();
            content.layoutIfNeeded();
            view::frame(&view::subviews(content)[0])
        }

        let (_pane, leaf) = tabs(text("Now Playing"));
        let accessory = tab_bar_controller(&leaf)
            .bottomAccessory()
            .expect("installed");
        let content = accessory.contentView();
        let intrinsic = view::fitting_size(&content);
        assert!(intrinsic.width > 0.0 && intrinsic.height > 0.0);

        // Wider bounds center the answer inside the host — it is not
        // force-filled. Tighter bounds keep it unclamped: the answer
        // overflows, still centered, so both a clamp and a fill would
        // fail the same assertions.
        let wide = cocoa_ui::geometry::Size::new(intrinsic.width * 2.0, intrinsic.height * 2.0);
        let wide_frame = child_frame(&content, wide);
        assert!(wide_frame.size.width < wide.width);
        assert!((wide_frame.origin.x - (wide.width - wide_frame.size.width) / 2.0).abs() < 0.01);
        assert!((wide_frame.origin.y - (wide.height - wide_frame.size.height) / 2.0).abs() < 0.01);

        let tight = cocoa_ui::geometry::Size::new(intrinsic.width / 2.0, intrinsic.height / 2.0);
        let tight_frame = child_frame(&content, tight);
        assert!(
            tight_frame.size.width > tight.width || tight_frame.size.height > tight.height,
            "the measured answer overflows the offered bounds instead of clamping"
        );
        assert!((tight_frame.origin.x - (tight.width - tight_frame.size.width) / 2.0).abs() < 0.01);
        assert!(
            (tight_frame.origin.y - (tight.height - tight_frame.size.height) / 2.0).abs() < 0.01
        );

        // A stretching child measures back to the bounds it was offered
        // and fills the host.
        let (_pane, leaf) = tabs(scroll(text("stretched")));
        let accessory = tab_bar_controller(&leaf)
            .bottomAccessory()
            .expect("installed");
        let content = accessory.contentView();
        let frame = child_frame(&content, wide);
        assert_eq!(frame.size, wide);
    }
}

/// Window lifecycle on a real, never-shown `NSWindow` — only reachable
/// because the harness runs on the true main thread, which is the only
/// place `-[NSWindow init]` is legal. The assertion bodies live in the
/// crate's `native-test` feature, which owns the private reach
/// into `windows` and `embedding`.
#[cfg(all(target_os = "macos", feature = "native-test"))]
mod window {
    pub use waterui_apple::native_test_support::{
        bind_root_window_wires_a_live_window, manager_installs_into_the_environment,
    };
}

/// The standard menu bar's content, read back off the installed `NSMenu`s.
#[cfg(all(target_os = "macos", feature = "native-test"))]
mod menus {
    use std::rc::Rc;

    use cocoa_ui::appkit::{Command, Menu, MenuButton, MenuItem};
    use cocoa_ui::objc2_app_kit::{NSApplication, NSEventModifierFlags};
    use waterui::component::menu::{CloseWindowPlacement, Shortcut};
    use waterui_apple::native_test_support::menus::window_menu_rows;

    use crate::mtm;

    /// `build_default`'s Window menu starts with the standard Close item —
    /// ⌘W, untargeted `performClose:`, disabled while no key window takes
    /// it — unless a declared menu carries Close, when the Window menu has
    /// none.
    pub fn the_window_menu_opens_with_the_standard_close_item() {
        let rows = window_menu_rows(
            mtm(),
            CloseWindowPlacement::WindowMenu,
            Some(Shortcut::new('w').command()).as_ref(),
        );
        let first = rows.first().expect("the Window menu has rows");
        assert_eq!(first.title, "Close");
        assert_eq!(first.key_equivalent, "w");
        assert_eq!(first.action.as_deref(), Some("performClose:"));
        assert!(first.modifiers.contains(NSEventModifierFlags::Command));
        assert!(!first.enabled, "no key window takes `performClose:`");

        let declared = window_menu_rows(mtm(), CloseWindowPlacement::Declared, None);
        assert!(!declared.is_empty(), "the Window menu keeps its other rows");
        assert!(
            declared
                .iter()
                .all(|row| row.title != "Close" && row.action.as_deref() != Some("performClose:")),
            "a declared Close is never repeated in the Window menu: {declared:?}"
        );
    }

    /// A mounted pull-down validates its rows each time it opens, and a
    /// callback row answers from its command's enabled state — set on the
    /// item before or after its action, or carried by the [`Command`].
    pub fn a_pull_down_validates_its_callback_rows_from_their_command() {
        let mtm = mtm();
        // Validation asks the application for each row's target, as it
        // does in a launched app.
        let _app = NSApplication::sharedApplication(mtm);
        let button = MenuButton::new(mtm);
        let menu = Menu::new(mtm, "");
        menu.add_item(MenuItem::new(mtm, "", None, ""));
        menu.add_item(
            MenuItem::new(mtm, "on", None, "")
                .with_enabled(true)
                .with_action(|| {}),
        );
        menu.add_item(
            MenuItem::new(mtm, "off", None, "")
                .with_enabled(false)
                .with_action(|| {}),
        );
        menu.add_item(
            MenuItem::new(mtm, "off after", None, "")
                .with_action(|| {})
                .with_enabled(false),
        );
        let off_command = Command {
            label: "off command".to_owned(),
            enabled: false,
            ..Command::default()
        };
        menu.add_item(MenuItem::command(mtm, &off_command, Rc::new(|| {})));
        button.set_menu(&menu);

        let native = menu.menu();
        assert!(
            native.autoenablesItems(),
            "the pull-down validates its rows"
        );
        native.update();
        let enabled: Vec<bool> = (1..native.numberOfItems())
            .map(|index| {
                native
                    .itemAtIndex(index)
                    .expect("a row at every index")
                    .isEnabled()
            })
            .collect();
        assert_eq!(enabled, [true, false, false, false]);
    }
}

/// GPU-surface ownership regression coverage (#1725): a real mounted
/// `SceneView` — production `build_surface`, `SurfaceState`,
/// `SceneRenderer`/`SceneEngine` — driven through the capturable
/// sequence, the publication wait and the readiness contract. Event
/// orderings the GPU's own timing cannot pin down are delivered by
/// calling the production settle bodies directly in the order under
/// test — reported as deterministic seam calls, not physical cadence or
/// real link deliveries.
#[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
mod gpu_surface {
    use std::rc::Rc;

    use libtest_mimic::Trial;
    use waterui_apple::native_test_support::ErrorLog;
    use waterui_apple::native_test_support::gpu_surface::MountedSceneSurface;

    use super::mtm;

    /// The registered trials — the settle-ordering and completion
    /// coverage for the mounted-surface ownership contract.
    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "gpu_surface::failed_render_settles_readiness_once",
                failed_render_settles_readiness_once,
            ),
            Trial::test(
                "gpu_surface::external_capture_submission_completes_ok",
                external_capture_submission_completes_ok,
            ),
            Trial::test(
                "gpu_surface::failure_inside_a_capture_scope_still_balances",
                failure_inside_a_capture_scope_still_balances,
            ),
            Trial::test(
                "gpu_surface::publication_park_inside_a_capture_scope_stays_balanced",
                publication_park_inside_a_capture_scope_stays_balanced,
            ),
            Trial::test(
                "gpu_surface::capture_opening_over_a_parked_wait_stays_balanced",
                capture_opening_over_a_parked_wait_stays_balanced,
            ),
            Trial::test(
                "gpu_surface::capture_opening_over_a_failed_hold_balances",
                capture_opening_over_a_failed_hold_balances,
            ),
            Trial::test(
                "gpu_surface::a_failed_child_answers_the_terminal_capture_outcome_once",
                a_failed_child_answers_the_terminal_capture_outcome_once,
            ),
            Trial::test(
                "gpu_surface::a_failed_batch_settles_its_requester_once",
                a_failed_batch_settles_its_requester_once,
            ),
            Trial::test(
                "gpu_surface::subpixel_extents_round_up_and_empty_bounds_detach",
                subpixel_extents_round_up_and_empty_bounds_detach,
            ),
            Trial::test(
                "gpu_surface::an_unwritten_production_drops_the_frame_unpresented",
                an_unwritten_production_drops_the_frame_unpresented,
            ),
            Trial::test(
                "gpu_surface::an_animating_scene_presents_its_frame",
                an_animating_scene_presents_its_frame,
            ),
            Trial::test(
                "gpu_surface::a_deferred_external_frame_requests_a_redraw",
                a_deferred_external_frame_requests_a_redraw,
            ),
            Trial::test(
                "gpu_surface::view_render_answers_gpu_failure_over_a_failed_child",
                view_render_answers_gpu_failure_over_a_failed_child,
            ),
            Trial::test(
                "gpu_surface::view_render_succeeds_over_a_healthy_gpu_child",
                view_render_succeeds_over_a_healthy_gpu_child,
            ),
        ]
    }

    /// A typed frame-path failure settling on the owner resolves its
    /// readiness exactly once — the waiter registered through the real
    /// `Capturable::register_waiter` fires, a waiter armed after the
    /// settle never wakes — and the failed owner stops offering its
    /// first frame. The settle is driven through `settle_failed` on the
    /// live generation — the entry the render `Err` arm takes (no
    /// renderable texture extent produces that `Err` itself: the
    /// platform's maximum texture dimension equals the surface's
    /// rejection bound).
    pub fn failed_render_settles_readiness_once() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_fixture_window();
        let probe = mounted.readiness_probe();
        mounted.settle_failure(mounted.current_generation());
        assert_eq!(
            probe.wakes(),
            1,
            "the registered readiness waiter settles exactly once"
        );
        assert!(
            !mounted.frame_presented(),
            "a failed frame presents no receipt"
        );
        assert!(
            !mounted.first_paint_participation(),
            "a failed surface stops offering its first frame"
        );
        let late = mounted.readiness_probe();
        assert_eq!(
            late.wakes(),
            0,
            "a settled owner never arms a later readiness waiter"
        );
        Ok(())
    }

    /// The live submission completion contract on a runner where the
    /// display link never paces: a real external capture renders and
    /// submits through the production sequence —
    /// `render_to_metal_texture` → `submit_with_completion` →
    /// `on_submitted_work_done` → `submission_validity` — and answers
    /// `Ok(())` on a healthy context, never a synthesized deferral.
    /// Onscreen `PresentedFrame` readiness is verified on
    /// physical-device runs, where the link delivers.
    pub fn external_capture_submission_completes_ok() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted
            .install_scene_renderer()
            .map_err(|error| format!("the production renderer install: {error}"))?;
        assert!(
            mounted.capture_external_once().is_ok(),
            "the live submission answers Ok through the shared validity contract"
        );
        assert!(
            mounted.first_paint_participation(),
            "the capture scope restored the presenting surface"
        );
        Ok(())
    }

    /// A batch failure routed to the owner inside an open capture scope
    /// lands its `settle_failed` on the main queue without disturbing
    /// the scope: `end_external_rendering` still matches its `begin`,
    /// and the failed hold survives the scope's close — the surface
    /// keeps refusing a first frame after it.
    pub fn failure_inside_a_capture_scope_still_balances() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        let probe = mounted.readiness_probe();
        mounted.begin_capture(Rc::new(|| {}));
        mounted.route_failure(mounted.current_generation());
        assert!(
            mounted.pump_main(5.0, || !mounted.first_paint_participation()),
            "the routed settle lands inside the open scope"
        );
        mounted.end_capture(true);
        assert_eq!(
            probe.wakes(),
            1,
            "the settle inside the scope resolved readiness once"
        );
        assert!(
            !mounted.first_paint_participation(),
            "the failed hold survives the scope's close"
        );
        Ok(())
    }

    /// A publication wait armed inside an open capture scope: the scope
    /// still closes balanced, a render against the parked surface
    /// answers `Err(CaptureError::Deferred)` — the park owes its next
    /// frame to the publication wake, not to a retry on the same context
    /// — and the surface keeps offering its first frame, since a park is
    /// not a failure.
    pub fn publication_park_inside_a_capture_scope_stays_balanced()
    -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        mounted.begin_capture(Rc::new(|| {}));
        mounted.park_on(mounted.current_generation());
        mounted.end_capture(true);
        assert!(
            matches!(
                mounted.capture_external_once(),
                Err(cocoa_ui::capture::CaptureError::Deferred)
            ),
            "a render against the parked hold answers Err(CaptureError::Deferred)"
        );
        assert!(
            mounted.first_paint_participation(),
            "a parked surface still offers its first frame"
        );
        Ok(())
    }

    /// A capture scope opening over an already-parked wait: the scope
    /// balances, the render inside it answers `Err(CaptureError::Deferred)`
    /// rather than retrying the parked frame, and the surface keeps
    /// offering its first frame.
    pub fn capture_opening_over_a_parked_wait_stays_balanced() -> Result<(), libtest_mimic::Failed>
    {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        mounted.park_on(mounted.current_generation());
        mounted.begin_capture(Rc::new(|| {}));
        assert!(
            matches!(
                mounted.render_capture_frame(64, 64),
                Err(cocoa_ui::capture::CaptureError::Deferred)
            ),
            "a render over the parked hold answers Err(CaptureError::Deferred)"
        );
        mounted.end_capture(true);
        assert!(
            mounted.first_paint_participation(),
            "a parked surface still offers its first frame"
        );
        Ok(())
    }

    /// A capture scope opening over a settled failure: the scope
    /// balances — `end_external_rendering` matches — the render inside
    /// answers `Err(CaptureError::Failed)` carrying the settled typed
    /// failure rather than retrying the failed frame, and the failed
    /// hold is untouched by the scope's open and close.
    pub fn capture_opening_over_a_failed_hold_balances() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_fixture_window();
        mounted.settle_failure(mounted.current_generation());
        mounted.begin_capture(Rc::new(|| {}));
        assert!(
            matches!(
                mounted.render_capture_frame(64, 64),
                Err(cocoa_ui::capture::CaptureError::Failed(_))
            ),
            "a render over the failed hold answers Err(CaptureError::Failed)"
        );
        mounted.end_capture(true);
        assert!(
            !mounted.first_paint_participation(),
            "the failed hold survives the scope's open and close"
        );
        Ok(())
    }

    /// The terminal capture contract for a failed child: a settled
    /// failure inside an open capture scope makes the render's
    /// completion answer `Err(CaptureError::Failed)` — the typed surface
    /// failure itself — once, without waiting on a redraw the failed
    /// context generation can never issue.
    pub fn a_failed_child_answers_the_terminal_capture_outcome_once()
    -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        mounted.begin_capture(Rc::new(|| {}));
        mounted.settle_failure(mounted.current_generation());
        let Err(cocoa_ui::capture::CaptureError::Failed(error)) =
            mounted.render_capture_frame(64, 64)
        else {
            return Err(
                "the failed surface must answer the terminal outcome, not a deferral".into(),
            );
        };
        assert!(
            matches!(
                error.downcast_ref::<waterui_graphics::gpu::runtime::HostedLayerError>(),
                Some(waterui_graphics::gpu::runtime::HostedLayerError::Surface(
                    waterui_graphics::cherenkov::SurfaceError::TooLarge { .. }
                ))
            ),
            "the terminal outcome carries the settled typed failure: {error}"
        );
        mounted.end_capture(true);
        Ok(())
    }

    /// The requesting owner of a failed batch settles exactly once:
    /// the test drives the settle-then-routed-copy ordering directly —
    /// `settle_failure` plays `produce`'s `Err` arm and `route_failure`
    /// plays the copy `EngineGeneration::settle_failed` delivers through
    /// the requester's sink; it never runs `produce`. The routed settle
    /// is a no-op on the same generation — one failure log, one
    /// readiness resolution, and the `Failed` hold stands.
    pub fn a_failed_batch_settles_its_requester_once() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        let probe = mounted.readiness_probe();
        let (log, errors) = ErrorLog::new("waterui_apple::components::gpu_surface");
        let generation = mounted.current_generation();
        let drained = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        tracing::subscriber::with_default(log, || {
            // The requester's own `Err` arm — `produce`'s synchronous settle.
            mounted.settle_failure(generation);
            // The routed copy the batch delivered back through the
            // requester's own sink — must land as a no-op. A marker block
            // queued behind it proves the drain: the main queue is FIFO.
            mounted.route_failure(generation);
            {
                let drained = drained.clone();
                cocoa_ui::main_queue::enqueue(move |_| {
                    drained.store(true, std::sync::atomic::Ordering::SeqCst);
                });
            }
            assert!(
                mounted.pump_main(5.0, || drained.load(std::sync::atomic::Ordering::SeqCst)),
                "the routed settle drains off the main queue"
            );
        });
        assert_eq!(
            errors.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the requester of a failed batch logs its settle exactly once"
        );
        assert_eq!(
            probe.wakes(),
            1,
            "the requester of a failed batch resolves readiness exactly once"
        );
        assert!(
            !mounted.first_paint_participation(),
            "the failed hold survives the routed copy"
        );
        Ok(())
    }

    /// The drawable extent is whole device pixels, never a truncated
    /// zero: a positive sub-pixel bound rounds up through
    /// `initialize_gpu` — the production door layout and backing changes
    /// run — and a genuinely empty view stops presenting entirely
    /// instead of carrying a zero size beside a live drawable
    /// configuration; restored bounds re-enter through the same door.
    pub fn subpixel_extents_round_up_and_empty_bounds_detach() -> Result<(), libtest_mimic::Failed>
    {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_fixture_window();
        // 0.25 pt truncates to 0 device pixels at 1x, 2x and 3x alike —
        // only ceil-rounding produces a nonzero drawable.
        mounted.set_view_frame(0.25, 0.25);
        mounted.initialize_gpu();
        assert_eq!(
            mounted.drawable_size(),
            Some((1, 1)),
            "0.25 pt of bounds still yields one device pixel of drawable"
        );

        // Genuinely empty: the attach epoch drops — no zero-size drawable
        // configuration survives beside it, so the surface stops
        // offering a frame to capture or reveal.
        mounted.set_view_frame(0.0, 0.0);
        let bounds = mounted.view_bounds();
        assert_eq!(
            (bounds.width, bounds.height),
            (0.0, 0.0),
            "the zero bounds write lands"
        );
        mounted.initialize_gpu();
        assert_eq!(
            mounted.drawable_size(),
            None,
            "a genuinely empty view carries no live drawable configuration"
        );

        // Restored bounds re-enter through the same `attach` door.
        mounted.set_view_frame(0.25, 0.25);
        mounted.initialize_gpu();
        assert_eq!(
            mounted.drawable_size(),
            Some((1, 1)),
            "restored bounds re-attach through initialize_gpu"
        );
        Ok(())
    }

    /// An onscreen frame whose scene was invalidated after the batch —
    /// `present` answers `Next::At` and the drawable target is never
    /// written — is dropped unpresented and the frame re-owed, so the
    /// recycled drawable never reaches the screen. The trial produces
    /// the shared batch at the delivered timestamp from a sibling
    /// surface on the same environment, invalidates the sibling's scene,
    /// then issues a real drawable checked out of a standalone
    /// `CAMetalLayer` (a link-bound layer forbids `nextDrawable`):
    /// `render_drawable` itself runs the guard, and the bound
    /// renderer's `wrote_target` report proves the guard saw the
    /// unwritten frame.
    ///
    /// The invalidation is what makes the order deterministic: the
    /// sibling's link may or may not deliver a first-paint frame during
    /// the window spin, but between `invalidate_scene` and
    /// `deliver_frame` there is no run-loop turn for any queued work —
    /// link delivery or redraw — to re-prepare the scene before the
    /// guard reads it.
    pub fn an_unwritten_production_drops_the_frame_unpresented() -> Result<(), libtest_mimic::Failed>
    {
        let mtm = mtm();
        let (producer, surface) = pollster::block_on(MountedSceneSurface::mount_pair(mtm))
            .map_err(|error| format!("two mounted SceneView surfaces: {error}"))?;
        producer
            .install_scene_renderer()
            .map_err(|error| format!("the producing surface's renderer: {error}"))?;
        surface.set_view_frame(64.0, 64.0);
        surface.ensure_attached();
        assert_eq!(
            surface.presenter_pixel_format(),
            Some(surface.capture_pixel_format()),
            "the presenter's layer carries the surface's declared presentation format"
        );
        let media_time = cocoa_ui::objc2_quartz_core::CACurrentMediaTime();
        let target_time = surface.map_frame_time(media_time);
        producer
            .render_scene_frame_at(target_time)
            .map_err(|error| format!("the producing surface's frame: {error}"))?;
        surface.invalidate_scene();
        assert!(
            surface.deliver_frame(media_time),
            "the standalone layer issued a drawable frame"
        );
        assert_eq!(
            surface.wrote_target(target_time),
            Some(false),
            "the bound renderer reported the target unwritten"
        );
        assert!(
            surface.frame_owed(),
            "the unwritten frame stays owed for the next production"
        );
        Ok(())
    }

    /// A scene whose `build_scene` always answers `true` —
    /// self-animating content — still produces the delivered
    /// timestamp: the batch's `again` only schedules the next frame, so
    /// the bound renderer reports the target written and the frame takes
    /// the submit path instead of dropping unwritten and owed. The
    /// drawable checked out of the standalone layer can mint no
    /// receipt — the bound renderer's `wrote_target` report and the
    /// cleared owed flag prove the frame was submitted for presentation.
    pub fn an_animating_scene_presents_its_frame() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let surface = pollster::block_on(MountedSceneSurface::mount_animating(mtm))
            .map_err(|error| format!("a mounted animating SceneView surface: {error}"))?;
        surface.set_view_frame(64.0, 64.0);
        surface.ensure_attached();
        let media_time = cocoa_ui::objc2_quartz_core::CACurrentMediaTime();
        let target_time = surface.map_frame_time(media_time);
        assert!(
            surface.deliver_frame(media_time),
            "the standalone layer issued a drawable frame"
        );
        assert_eq!(
            surface.wrote_target(target_time),
            Some(true),
            "the animating scene's delivered frame wrote its target"
        );
        assert!(
            !surface.frame_owed(),
            "the produced frame was submitted, not re-owed"
        );
        Ok(())
    }

    /// `ViewRenderer::render` over a GPU child on a sealed-failure
    /// generation resolves `Err(RenderError::Gpu(_))` — the Arc'd
    /// `HostedLayerError` stays in `source()` — instead of hanging or
    /// panicking: the central capture's terminal `Failed` arm answers
    /// the typed outcome for a surface that can never produce a frame
    /// on the failed context generation.
    pub fn view_render_answers_gpu_failure_over_a_failed_child() -> Result<(), libtest_mimic::Failed>
    {
        use waterui_apple::dispatch;
        use waterui_apple::native_test_support::gpu_surface::{fixture_env, fixture_scene_view};
        use waterui_apple::native_test_support::view_renderer::{
            GpuRuntime, HostedLayerError, install_service, scene_engine,
        };
        use waterui_backend_core::AnyView;
        use waterui_core::view_renderer::{RenderError, RenderSize, ViewRenderer};
        use waterui_graphics::cherenkov::SurfaceError;

        let (runtime, mut env) = pollster::block_on(async {
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            let env = fixture_env(runtime.clone());
            Ok::<_, String>((runtime, env))
        })
        .map_err(|error| format!("the fixture environment: {error}"))?;
        dispatch::install(&mut env);
        install_service(&mut env);

        // Seal the shared generation the rendered leaf's `SceneView`
        // mounts on — the same typed carrier a real surface rejection
        // produces.
        let generation = scene_engine(&env)
            .generation(&runtime, &runtime.context())
            .map_err(|error| format!("the shared engine generation: {error}"))?;
        let max = runtime.context().device().limits().max_texture_dimension_2d;
        generation.seal_failure_for_test(HostedLayerError::Surface(SurfaceError::TooLarge {
            width: u32::MAX,
            height: u32::MAX,
            max,
        }));

        let renderer = env
            .get::<ViewRenderer>()
            .ok_or("the installed ViewRenderer service")?;
        // `block_on_main`, not `pollster`: the capture's completions
        // land on `DispatchQueue::main()`, which only a turning run
        // loop services.
        let Err(RenderError::Gpu(cause)) = waterui_apple::native_test_support::block_on_main(
            30.0,
            renderer.render(
                AnyView::new(fixture_scene_view()),
                RenderSize::new(64.0, 64.0),
            ),
        ) else {
            return Err(
                "a capture over the failed GPU child resolves Err(RenderError::Gpu)".into(),
            );
        };
        // The Arc'd `HostedLayerError` must stay reachable down the
        // `source()` chain through the `GpuSurfaceFailed` wrapper.
        let mut link: &dyn std::error::Error = &*cause;
        let reachable = loop {
            if let Some(HostedLayerError::Surface(..)) = link.downcast_ref::<HostedLayerError>() {
                break true;
            }
            let Some(next) = link.source() else {
                break false;
            };
            link = next;
        };
        assert!(
            reachable,
            "the Gpu cause's chain reaches the settled HostedLayerError"
        );
        Ok(())
    }

    /// A windowless `ViewRenderer::render` over a healthy `SceneView`
    /// resolves `Ok` with real pixels: the offscreen capture window
    /// declares `DynamicRange::Standard` for the hosted subtree (the
    /// render target is RGBA8), so the leaf's GPU surface resolves a
    /// concrete range instead of panicking on the missing display.
    pub fn view_render_succeeds_over_a_healthy_gpu_child() -> Result<(), libtest_mimic::Failed> {
        use waterui_apple::dispatch;
        use waterui_apple::native_test_support::gpu_surface::{fixture_env, fixture_scene_view};
        use waterui_apple::native_test_support::view_renderer::{GpuRuntime, install_service};
        use waterui_backend_core::AnyView;
        use waterui_core::view_renderer::{RenderSize, ViewRenderer};

        let (_runtime, mut env) = pollster::block_on(async {
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            let env = fixture_env(runtime.clone());
            Ok::<_, String>((runtime, env))
        })
        .map_err(|error| format!("the fixture environment: {error}"))?;
        dispatch::install(&mut env);
        install_service(&mut env);

        let renderer = env
            .get::<ViewRenderer>()
            .ok_or("the installed ViewRenderer service")?;
        let result = waterui_apple::native_test_support::block_on_main(
            30.0,
            renderer.render(
                AnyView::new(fixture_scene_view()),
                RenderSize::new(64.0, 64.0),
            ),
        )
        .map_err(|error| format!("a render over the healthy GPU child resolves Ok: {error}"))?;
        // The rendered leaf is measured detached, so the capture rasters
        // at the backing scale a windowless view reports.
        let scale =
            cocoa_ui::view::backing_scale_factor(&cocoa_ui::PlatformView::new(super::mtm()));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a 64-point extent at a backing scale is a small positive pixel count"
        )]
        let side = (64.0 * scale).ceil() as u32;
        assert_eq!(
            (result.width, result.height),
            (side, side),
            "the raster is the 64-point proposal at the backing scale"
        );
        let (texels, rest) = result.rgba_data.as_chunks::<4>();
        assert!(rest.is_empty(), "the raster is whole RGBA8 texels");
        assert_eq!(
            texels.len(),
            side as usize * side as usize,
            "the raster holds one RGBA8 texel per pixel"
        );
        assert!(
            texels.iter().all(|texel| *texel == [255, 255, 255, 255]),
            "every texel is the fixture's opaque white fill"
        );
        Ok(())
    }

    /// A deferred external frame replays through the open capture
    /// scope's own redraw notification: the production
    /// `defer_unwritten_external_frame` owes the frame and requests the
    /// redraw `handle_redraw_request` turns into the capture's `redraw`
    /// callback — the deferred capture re-renders without relying on an
    /// unrelated invalidation.
    pub fn a_deferred_external_frame_requests_a_redraw() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        let redraws = Rc::new(std::cell::Cell::new(0_u32));
        mounted.begin_capture({
            let redraws = Rc::clone(&redraws);
            Rc::new(move || {
                redraws.set(redraws.get() + 1);
            })
        });
        mounted.defer_external_frame();
        assert!(
            mounted.frame_owed(),
            "the deferred frame stays owed for the replay"
        );
        assert!(
            mounted.pump_main(5.0, || redraws.get() > 0),
            "the deferred frame replays through the capture's redraw notification"
        );
        mounted.end_capture(false);
        Ok(())
    }
}

/// Filtered-view settlement over a failed GPU-surface child: a real
/// filtered leaf mounted through the production `build_filtered_parts`
/// over a real `SceneView` child — the same `FilteredState`, capturable
/// registration and link wiring a mount installs. After the child
/// settles a typed failure, the carrier its own capturable answers a
/// parent capture with — `CaptureError::Failed` — is fed through the
/// filtered leaf's captured-frame completion exactly as
/// `finish_captured_frame` would deliver it, and the settle contract
/// runs against it: one error log, readiness resolves so capture
/// waiters never hang, and the link parks until a new context
/// generation publishes instead of retrying the failed one forever.
#[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
mod filtered {
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use cocoa_ui::capture::CaptureError;
    use libtest_mimic::Trial;
    use waterui_apple::native_test_support::ErrorLog;
    use waterui_apple::native_test_support::filtered::MountedFilteredSurface;
    use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};
    use waterui_graphics::gpu::runtime::HostedLayerError;

    use super::mtm;

    /// The registered trials — the failed-child settle contract and the
    /// zero-bounds detach.
    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "filtered::a_view_over_a_failed_child_settles_once",
                a_view_over_a_failed_child_settles_once,
            ),
            Trial::test(
                "filtered::a_collapsed_view_detaches_and_presents_again",
                a_collapsed_view_detaches_and_presents_again,
            ),
            Trial::test(
                "filtered::a_deferred_capture_pauses_the_link_until_the_child_redraws",
                a_deferred_capture_pauses_the_link_until_the_child_redraws,
            ),
            Trial::test(
                "filtered::a_lost_context_parks_the_link_until_the_publication",
                a_lost_context_parks_the_link_until_the_publication,
            ),
            Trial::test(
                "filtered::a_view_collapsed_mid_capture_renders_transparent",
                a_view_collapsed_mid_capture_renders_transparent,
            ),
        ]
    }

    /// A parent capture that snapshots a filtered view while it has area,
    /// then submits after a layout pass collapsed it, completes with the
    /// view's region transparent — what the screen shows for a collapsed
    /// view — instead of capturing zero-bounds content.
    pub fn a_view_collapsed_mid_capture_renders_transparent() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let filtered = pollster::block_on(MountedFilteredSurface::mount_filling(mtm))
            .map_err(|error| format!("a mounted filling filtered surface: {error}"))?;
        filtered.ensure_attached();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "the filtered view presents its first frame"
        );
        let parent = filtered.parent_capture();

        let control = parent.start();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || control.landed()),
            "the control capture completes"
        );
        let outcome = control.take();
        assert!(
            matches!(outcome, Some(Ok(()))),
            "the control capture lands, got {outcome:?}"
        );
        assert!(
            parent.pixels().iter().all(|pixel| pixel[3] == u8::MAX),
            "an uncollapsed filtered view fills its region of the parent's target"
        );

        let collapsed = parent.start();
        filtered.set_host_frame(0.0, 0.0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || collapsed.landed()),
            "the capture over the view collapsed after its prepare completes"
        );
        let outcome = collapsed.take();
        assert!(
            matches!(outcome, Some(Ok(()))),
            "a capture over a view collapsed between prepare and submit lands, got {outcome:?}"
        );
        assert!(
            parent.pixels().iter().all(|pixel| *pixel == [0; 4]),
            "the collapsed view's region of the parent's target is transparent"
        );
        Ok(())
    }

    /// A frame whose capture defers on a live context — a nested filter
    /// whose effect setup has not landed — leaves the display link paused
    /// rather than recapturing every vsync, and the nested setup's redraw
    /// re-arms it so the owed frame lands once the deferral clears.
    pub fn a_deferred_capture_pauses_the_link_until_the_child_redraws()
    -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let (filtered, gate) =
            pollster::block_on(MountedFilteredSurface::mount_over_pending_filter(mtm))
                .map_err(|error| format!("a mounted nested filtered surface: {error}"))?;
        filtered.ensure_attached();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.nested_joined_capture()
                && !filtered.render_in_flight()),
            "the first frame's capture defers on the nested filter's pending setup"
        );
        assert!(
            filtered.link_paused(),
            "a live-context deferral leaves the link paused — no display-rate recapture"
        );
        assert!(
            !filtered.frame_presented(),
            "the deferred frame never presented"
        );

        gate.release();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "the nested setup's redraw re-arms the link and the owed frame lands"
        );
        Ok(())
    }

    /// A GPU device loss while the filtered view is attached: the
    /// delivered frame parks the leaf — the link pauses, one publication
    /// watch arms, and a delivery that still arrives while parked drops
    /// without rendering — and the rebuilt context's publication wakes
    /// the wait, re-arms the link and lands the owed frame.
    pub fn a_lost_context_parks_the_link_until_the_publication() -> Result<(), libtest_mimic::Failed>
    {
        let mtm = mtm();
        let filtered = pollster::block_on(MountedFilteredSurface::mount_filling(mtm))
            .map_err(|error| format!("a mounted filling filtered surface: {error}"))?;
        filtered.ensure_attached();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "the attached filtered view presents its first frame"
        );
        let encoded = filtered.encoded_frames();
        let lost = filtered.current_generation();

        // The link has demand again when the loss lands, so the next
        // delivered frame hits `render_frame`'s lost-context arm.
        filtered.request_render();
        filtered.lose_device("test device loss");
        assert!(
            filtered.deliver_frame(0.0),
            "a drawable checks out for the frame"
        );
        assert!(
            filtered.parked() && filtered.link_paused(),
            "the lost context parks the leaf: the link pauses and the publication watch arms"
        );

        // A delivery still arriving while parked drops unpresented:
        // nothing renders and the one outstanding watch stays
        // outstanding — no per-vsync re-arm.
        assert!(
            filtered.deliver_frame(0.0),
            "a drawable checks out for the parked leaf"
        );
        assert!(
            filtered.parked() && filtered.link_paused() && filtered.encoded_frames() == encoded,
            "a parked leaf renders nothing and keeps its one outstanding watch"
        );

        // The rebuild's publication wakes the parked wait: the hold
        // clears, the link re-arms, and the owed frame renders on the
        // new generation — the encode count is the render's own record.
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.current_generation() > lost
                && !filtered.parked()
                && filtered.encoded_frames() > encoded),
            "the publication wake re-arms the link and the owed frame renders"
        );
        Ok(())
    }

    /// A filtered view sizes its hidden content with its own bounds, and
    /// a host collapsed to zero detaches — no presenter, so no link
    /// remains to issue a frame over content with nothing to capture —
    /// until restored bounds re-attach it and it presents again.
    pub fn a_collapsed_view_detaches_and_presents_again() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let filtered = pollster::block_on(MountedFilteredSurface::mount(mtm))
            .map_err(|error| format!("a mounted filtered surface: {error}"))?;
        filtered.ensure_attached();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "the attached filtered view presents its first frame"
        );
        assert_eq!(
            filtered.child.view_bounds(),
            filtered.host_bounds(),
            "the hidden content is framed with the host"
        );

        filtered.set_host_frame(0.0, 0.0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || !filtered.attached()
                && !filtered.has_presenter()),
            "a zero-sized filtered view detaches and drops its presenter"
        );

        // 0.4 pt is positive but under one device pixel at 1x and 2x: it
        // rounds up to whole pixels, attaches and presents a real frame.
        filtered.set_host_frame(0.4, 0.4);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "a positive sub-pixel filtered view attaches and presents"
        );
        // `n` is the ceiling of `extent` exactly when `n - 1 < extent <= n`.
        let extent = 0.4 * filtered.backing_scale();
        let (width, height) = filtered.input_size();
        for pixels in [width, height] {
            assert!(
                pixels > 0 && f64::from(pixels - 1) < extent && extent <= f64::from(pixels),
                "{extent} device pixels of bounds round up to whole pixels, got {width}x{height}"
            );
        }

        filtered.set_host_frame(0.0, 0.0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || !filtered.attached()
                && !filtered.has_presenter()),
            "the sub-pixel view collapses and detaches again"
        );

        filtered.set_host_frame(64.0, 64.0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "restored bounds re-attach the filtered view and it presents again"
        );
        Ok(())
    }

    /// The carrier a parent capture receives from the settled child —
    /// `Err(CaptureError::Failed)` carrying the child's typed failure,
    /// read through the real `begin_capture` → `render_capture_frame` →
    /// `end_capture` scope.
    fn failed_child_carrier(
        child: &waterui_apple::native_test_support::gpu_surface::MountedSceneSurface,
    ) -> Arc<dyn std::error::Error + Send + Sync> {
        child.begin_capture(Rc::new(|| {}));
        let outcome = child.render_capture_frame(64, 64);
        child.end_capture(false);
        match outcome {
            Err(CaptureError::Failed(error)) => error,
            _ => panic!("a settled child answers Failed to a parent capture"),
        }
    }

    /// A filtered view drawing on screen over a failed child settles the
    /// terminal failure once: readiness resolves for the waiter a
    /// first-paint collect queued, the link parks and no render remains
    /// in flight, and a subsequent parent capture over the filtered
    /// view answers `Err(Failed)` with the child's typed error rather
    /// than deferring forever. A repeat settle for the same generation
    /// logs no additional error.
    pub fn a_view_over_a_failed_child_settles_once() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let filtered = pollster::block_on(MountedFilteredSurface::mount(mtm))
            .map_err(|error| format!("a mounted filtered surface: {error}"))?;
        filtered.ensure_attached();
        let probe = filtered.readiness_probe();
        let generation = filtered.current_generation();

        // The child genuinely failed — settle it, then read the carrier
        // its own capturable answers a parent capture with.
        filtered.child.settle_failure(generation);
        let first = failed_child_carrier(&filtered.child);

        let (log, errors) = ErrorLog::new("waterui_apple::components::filtered");
        tracing::subscriber::with_default(log, || {
            filtered.settle_failed_capture(generation, &first);
            // A later redraw of the failed child lands the same
            // terminal outcome on the same generation — logged once.
            let repeat = failed_child_carrier(&filtered.child);
            filtered.settle_failed_capture(generation, &repeat);
        });
        assert_eq!(
            errors.load(Ordering::SeqCst),
            1,
            "the filtered leaf logs its failed settle exactly once"
        );
        assert_eq!(
            probe.wakes(),
            1,
            "the filtered leaf resolves its readiness waiter exactly once"
        );
        assert!(
            !filtered.first_paint_participation(),
            "the filtered view leaves first-paint waiting until a new context rebinds"
        );
        assert!(
            filtered.link_paused(),
            "the display link parks until a new context generation publishes"
        );
        assert!(
            !filtered.render_in_flight(),
            "no further render is in flight after the settle"
        );
        filtered.request_render();
        assert!(
            filtered.link_paused() && !filtered.render_in_flight(),
            "a scheduled frame can no longer take off on the failed generation"
        );

        // A parent capture over the filtered view reads the terminal
        // outcome — the child's typed failure, not a deferral.
        match filtered.capture_outcome() {
            Err(CaptureError::Failed(error)) => assert!(
                error.downcast_ref::<HostedLayerError>().is_some(),
                "the forwarded outcome carries the child's typed failure: {error}"
            ),
            other => {
                return Err(format!(
                    "the filtered view must answer Failed to a parent capture, got {other:?}"
                )
                .into());
            }
        }
        Ok(())
    }
}

/// `ViewController` boundary semantics (#1689): under real `UIKit`
/// containment — a `UIViewController` parent, `addChild`/`didMove`, a
/// native container smaller than the window — an embedded controller's
/// host view keeps the bounds its parent assigned through layout, resize
/// and reparenting, while a `window_root` controller still fills its
/// window. The old `viewWillLayoutSubviews` window-bounds override fails
/// the embedded trial on this same harness.
#[cfg(target_os = "ios")]
mod controller_bounds {
    use cocoa_ui::objc2_ui_kit::{UIView, UIViewController, UIWindow};
    use cocoa_ui::uikit::{ViewController, view_controller as vc, window_root};
    use cocoa_ui::{PlatformView, view};
    use objc2::{MainThreadOnly, msg_send};

    use super::{HostView, Retained, mtm};

    /// The `controller_bounds::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "controller_bounds::embedded_child_keeps_parent_bounds",
                || {
                    embedded_child_keeps_parent_bounds();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test("controller_bounds::window_root_host_fills_window", || {
                window_root_host_fills_window();
                Ok(())
            }),
        ]
    }

    fn window(frame: cocoa_ui::Rect) -> Retained<UIWindow> {
        // SAFETY: `initWithFrame:` is `UIWindow`'s plain initializer and
        // `mtm` proves the main-thread confinement the harness provides.
        unsafe {
            msg_send![
                UIWindow::alloc(mtm()),
                initWithFrame: cocoa_ui::objc2_core_foundation::CGRect::from(frame)
            ]
        }
    }

    fn view_controller() -> Retained<UIViewController> {
        // SAFETY: plain `UIViewController` init on the main thread.
        unsafe { msg_send![UIViewController::alloc(mtm()), init] }
    }

    fn plain_view(frame: cocoa_ui::Rect) -> Retained<UIView> {
        // SAFETY: plain `UIView` init on the main thread.
        unsafe {
            msg_send![
                UIView::alloc(mtm()),
                initWithFrame: cocoa_ui::objc2_core_foundation::CGRect::from(frame)
            ]
        }
    }

    /// Tears a window down after the trial: the child controllers leave
    /// their parents through the normal containment callbacks, the root is
    /// cleared and the window hidden.
    fn teardown(window: &UIWindow, children: &[&UIViewController]) {
        for child in children {
            vc::will_move_to_parent(child);
            child.view().unwrap().removeFromSuperview();
            vc::remove_from_parent(child);
        }
        window.setRootViewController(None);
        window.setHidden(true);
        window.layoutIfNeeded();
    }

    /// A `ViewController` embedded as a child of a real
    /// `UIViewController` inside a 340x460 native container — the
    /// `WaterUIHostController` shape. Its host view must keep the bounds
    /// the container assigns through layout, resize and reparenting: the
    /// unmodified window-bounds override would reset it to the window's.
    fn embedded_child_keeps_parent_bounds() {
        let mtm = mtm();
        let window = window(cocoa_ui::Rect::new(0.0, 0.0, 390.0, 844.0));
        let parent = view_controller();
        window.setRootViewController(Some(&parent));

        let container = plain_view(cocoa_ui::Rect::new(50.0, 140.0, 340.0, 460.0));
        parent.view().unwrap().addSubview(&container);

        let embedded = cocoa_ui::Rect::new(0.0, 0.0, 340.0, 460.0);
        let controller = ViewController::new(mtm, HostView::new(mtm, embedded));
        let host: &PlatformView = controller.host_view();

        vc::add_child(&parent, &controller);
        host.setFrame(cocoa_ui::objc2_core_foundation::CGRect::from(embedded));
        host.setAutoresizingMask(
            cocoa_ui::objc2_ui_kit::UIViewAutoresizing::FlexibleWidth
                | cocoa_ui::objc2_ui_kit::UIViewAutoresizing::FlexibleHeight,
        );
        container.addSubview(host);
        vc::did_move_to_parent(&controller);

        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        assert_eq!(
            view::frame(host).size,
            embedded.size,
            "embedded host must keep the parent-assigned bounds"
        );

        // A container resize flows through the normal layout path.
        container.setFrame(cocoa_ui::objc2_core_foundation::CGRect::from(
            cocoa_ui::Rect::new(50.0, 140.0, 300.0, 220.0),
        ));
        window.layoutIfNeeded();
        assert_eq!(
            view::frame(host).size,
            cocoa_ui::geometry::Size::new(300.0, 220.0),
            "resized embedded host tracks its container, not the window"
        );

        // Reparent under a second native container through the full
        // containment sequence.
        let other = plain_view(cocoa_ui::Rect::new(0.0, 0.0, 340.0, 460.0));
        parent.view().unwrap().addSubview(&other);
        vc::will_move_to_parent(&controller);
        host.removeFromSuperview();
        other.addSubview(host);
        host.setFrame(cocoa_ui::objc2_core_foundation::CGRect::from(
            cocoa_ui::Rect::new(0.0, 0.0, 340.0, 460.0),
        ));
        vc::did_move_to_parent(&controller);
        window.layoutIfNeeded();
        assert_eq!(
            view::frame(host).size,
            cocoa_ui::geometry::Size::new(340.0, 460.0),
            "reparented embedded host keeps the parent's assignment"
        );

        teardown(&window, &[&controller]);
    }

    /// The same controller type at a true window root — a `window_root`
    /// host view — still fills its window through layout.
    fn window_root_host_fills_window() {
        let mtm = mtm();
        let window = window(cocoa_ui::Rect::new(0.0, 0.0, 390.0, 844.0));
        let controller = ViewController::new(mtm, window_root(mtm));
        window.setRootViewController(Some(&controller));
        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        let host: &PlatformView = controller.host_view();
        assert_eq!(
            view::frame(host).size,
            cocoa_ui::geometry::Size::new(390.0, 844.0),
            "window_root host must fill its window"
        );
        teardown(&window, &[]);
    }
}

/// §7.1's two safe-area regions on `UIKit`: the container region
/// `safeAreaInsets` reports and the keyboard region the window's
/// `KeyboardRegion` object derives from the real `UIKeyboard`
/// notifications. Hydrolysis's
/// `tests/safe_area.rs` is the reference — these trials mount the same
/// view shapes through `mount_uikit` and assert laid-out frames, content
/// insets and scroll offsets, never pixels.
#[cfg(target_os = "ios")]
mod safe_area {
    use cocoa_ui::objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use cocoa_ui::objc2_foundation::{
        NSDictionary, NSNotificationCenter, NSNumber, NSObjectProtocol, NSString, NSValue,
    };
    use cocoa_ui::objc2_ui_kit::{
        UIBarPosition, UIBarPositioning, UIEdgeInsets, UIKeyboardAnimationCurveUserInfoKey,
        UIKeyboardAnimationDurationUserInfoKey, UIKeyboardFrameEndUserInfoKey,
        UIKeyboardWillChangeFrameNotification, UIKeyboardWillHideNotification,
        UIKeyboardWillShowNotification, UINavigationBar, UINavigationController, UIScrollView,
        UITabBar, UITextView, UIView,
    };
    use cocoa_ui::uikit::view_controller::owning_controller;
    use cocoa_ui::uikit::{
        ColorView, Label, NavContentController, ScrollView, TableView, TextField, keyboard,
    };
    use cocoa_ui::{PlatformView, Retained, view};
    use objc2::runtime::AnyObject;
    use objc2::{MainThreadOnly, msg_send};
    use waterui::component::list::{List, ListItem};
    use waterui::graphics::color::Srgb;
    use waterui::id::SelfId;
    use waterui::layout::safe_area::{EdgeSet, SafeAreaRegions};
    use waterui::navigation::{
        NavigationStack, NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement,
        NavigationView, Tab, Tabs,
    };
    use waterui::prelude::*;
    use waterui::reactive::{Binding, binding};
    use waterui::{AnyView, Color, Str};
    use waterui_apple::native_test_support::{
        MAIN_QUEUE_DEADLINE, UIKitMount, mount_uikit, mount_uikit_embedded, mount_uikit_window,
        pump_main_until, render_environment, spawned_window,
    };

    use super::mtm;

    /// One `safe_area::` trial entry.
    fn t(name: &'static str, body: fn()) -> libtest_mimic::Trial {
        libtest_mimic::Trial::test(format!("safe_area::{name}"), move || {
            body();
            Ok(())
        })
    }

    /// The container-region and `IgnoreSafeArea` declaration trials.
    fn container_trials() -> Vec<libtest_mimic::Trial> {
        vec![
            t(
                "root_content_stays_inside_the_safe_area",
                root_content_stays_inside_the_safe_area,
            ),
            t(
                "ignore_safe_area_all_reaches_the_window_origin",
                ignore_safe_area_all_reaches_the_window_origin,
            ),
            t(
                "ignore_safe_area_releases_only_the_flagged_edges",
                ignore_safe_area_releases_only_the_flagged_edges,
            ),
            t(
                "an_ignore_releases_only_on_an_edge_the_frame_touches",
                an_ignore_releases_only_on_an_edge_the_frame_touches,
            ),
            t(
                "a_safe_area_change_relays_out_the_root",
                a_safe_area_change_relays_out_the_root,
            ),
            t(
                "ignoring_only_the_keyboard_region_keeps_the_container_inset",
                ignoring_only_the_keyboard_region_keeps_the_container_inset,
            ),
            t(
                "ignoring_only_the_container_region_releases_nothing_under_the_keyboard",
                ignoring_only_the_container_region_releases_nothing_under_the_keyboard,
            ),
            t(
                "a_fractional_keyboard_still_touches_the_boundary",
                a_fractional_keyboard_still_touches_the_boundary,
            ),
        ]
    }

    /// The background-slot fill extension trials.
    fn fill_trials() -> Vec<libtest_mimic::Trial> {
        vec![
            t(
                "a_fill_background_paints_under_the_keyboard_region",
                a_fill_background_paints_under_the_keyboard_region,
            ),
            t(
                "a_fill_inside_opacity_still_extends",
                a_fill_inside_opacity_still_extends,
            ),
            t(
                "a_fill_with_an_empty_ignore_extends_nowhere",
                a_fill_with_an_empty_ignore_extends_nowhere,
            ),
            t(
                "a_fill_ignore_names_the_only_edge_that_extends",
                a_fill_ignore_names_the_only_edge_that_extends,
            ),
            t(
                "non_fill_leaves_at_an_edge_do_not_extend",
                non_fill_leaves_at_an_edge_do_not_extend,
            ),
            t(
                "a_non_fill_background_stays_inside_the_keyboard_region",
                a_non_fill_background_stays_inside_the_keyboard_region,
            ),
            t(
                "a_fill_under_an_ancestor_top_ignore_still_extends_at_the_bottom",
                a_fill_under_an_ancestor_top_ignore_still_extends_at_the_bottom,
            ),
            t(
                "a_moving_container_re_extends_its_fill_to_each_region_boundary",
                a_moving_container_re_extends_its_fill_to_each_region_boundary,
            ),
            t(
                "a_navigation_page_touching_the_bottom_still_extends_its_fill",
                a_navigation_page_touching_the_bottom_still_extends_its_fill,
            ),
            t(
                "a_navigation_page_background_stays_below_the_bar",
                a_navigation_page_background_stays_below_the_bar,
            ),
            t(
                "an_ignore_top_inside_navigation_content_stays_below_the_bar",
                an_ignore_top_inside_navigation_content_stays_below_the_bar,
            ),
        ]
    }

    /// The scroll-surface, focused-field and window-region trials.
    fn scroll_trials() -> Vec<libtest_mimic::Trial> {
        vec![
            t(
                "a_scroll_surface_scrolls_the_focused_field_clear_of_the_keyboard",
                a_scroll_surface_scrolls_the_focused_field_clear_of_the_keyboard,
            ),
            t(
                "a_keyboard_inset_change_clears_the_field_on_that_frame",
                a_keyboard_inset_change_clears_the_field_on_that_frame,
            ),
            t(
                "a_field_in_a_nested_scroll_is_cleared_once",
                a_field_in_a_nested_scroll_is_cleared_once,
            ),
            t(
                "a_focused_field_clears_to_the_surface_frame_above_a_toolbar",
                a_focused_field_clears_to_the_surface_frame_above_a_toolbar,
            ),
            t(
                "a_list_row_field_scrolls_clear_of_the_keyboard",
                a_list_row_field_scrolls_clear_of_the_keyboard,
            ),
            t(
                "a_form_inside_navigation_content_clears_the_focused_field",
                a_form_inside_navigation_content_clears_the_focused_field,
            ),
            t(
                "a_composer_hstack_keeps_its_intrinsic_height_under_the_keyboard",
                a_composer_hstack_keeps_its_intrinsic_height_under_the_keyboard,
            ),
            t(
                "a_scroll_above_a_composer_keeps_a_zero_keyboard_inset",
                a_scroll_above_a_composer_keeps_a_zero_keyboard_inset,
            ),
            t(
                "a_text_view_inside_the_tree_is_not_a_scroll_surface",
                a_text_view_inside_the_tree_is_not_a_scroll_surface,
            ),
            t(
                "a_user_scroll_under_the_keyboard_is_not_undone_by_the_layout_pass",
                a_user_scroll_under_the_keyboard_is_not_undone_by_the_layout_pass,
            ),
            t(
                "nested_hosts_and_a_scroll_share_the_windows_keyboard_region",
                nested_hosts_and_a_scroll_share_the_windows_keyboard_region,
            ),
            t(
                "a_page_pushed_over_the_keyboard_returns_keyboard_free",
                a_page_pushed_over_the_keyboard_returns_keyboard_free,
            ),
            t(
                "a_scroll_pushed_into_the_band_at_constant_size_updates_its_inset",
                a_scroll_pushed_into_the_band_at_constant_size_updates_its_inset,
            ),
            t(
                "a_toolbar_appearing_while_the_keyboard_is_up_rederives_the_inset",
                a_toolbar_appearing_while_the_keyboard_is_up_rederives_the_inset,
            ),
            t(
                "a_keyboard_notification_before_mount_is_not_replayed",
                a_keyboard_notification_before_mount_is_not_replayed,
            ),
            t(
                "a_guide_frame_equal_to_the_bottom_band_seeds_zero",
                a_guide_frame_equal_to_the_bottom_band_seeds_zero,
            ),
            t(
                "a_guide_frame_above_the_bottom_band_seeds_the_frame",
                a_guide_frame_above_the_bottom_band_seeds_the_frame,
            ),
        ]
    }

    /// The chrome-docking trials under navigation and tab hosts.
    fn chrome_trials() -> Vec<libtest_mimic::Trial> {
        vec![
            t(
                "a_tab_bar_stays_docked_under_the_keyboard",
                a_tab_bar_stays_docked_under_the_keyboard,
            ),
            t(
                "a_navigation_page_without_a_bar_passes_the_edge_through",
                a_navigation_page_without_a_bar_passes_the_edge_through,
            ),
            t(
                "a_navigation_bottom_toolbar_stays_docked_under_the_keyboard",
                a_navigation_bottom_toolbar_stays_docked_under_the_keyboard,
            ),
            t(
                "a_stack_page_toolbar_inside_a_tab_stacks_on_the_docked_tab_bar",
                a_stack_page_toolbar_inside_a_tab_stacks_on_the_docked_tab_bar,
            ),
            t(
                "a_fixed_height_tabs_inside_tab_content_docks_its_own_bar",
                a_fixed_height_tabs_inside_tab_content_docks_its_own_bar,
            ),
        ]
    }

    /// The `safe_area::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        [
            container_trials(),
            fill_trials(),
            scroll_trials(),
            chrome_trials(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// The keyboard band's height — the layout-spec's worked examples
    /// use 336 points — at the window's real screen width.
    const KEYBOARD_HEIGHT: f64 = 336.0;

    /// The end frame a keyboard `height` points tall reports against
    /// `window`'s screen — screen coordinates, the whole screen width.
    fn keyboard_end_sized(window: &cocoa_ui::objc2_ui_kit::UIWindow, height: f64) -> CGRect {
        let screen = window.screen().bounds();
        CGRect::new(
            CGPoint::new(0.0, screen.size.height - height),
            CGSize::new(screen.size.width, height),
        )
    }

    /// The end frame a keyboard occupying the bottom `KEYBOARD_HEIGHT`
    /// points reports.
    fn keyboard_end(window: &cocoa_ui::objc2_ui_kit::UIWindow) -> CGRect {
        keyboard_end_sized(window, KEYBOARD_HEIGHT)
    }

    /// The end frame `UIKeyboardWillHideNotification` reports: the
    /// keyboard slid to just under the screen's bottom edge, overlapping
    /// the window not at all.
    fn keyboard_hide_end(window: &cocoa_ui::objc2_ui_kit::UIWindow) -> CGRect {
        let screen = window.screen().bounds();
        CGRect::new(
            CGPoint::new(0.0, screen.size.height),
            CGSize::new(screen.size.width, KEYBOARD_HEIGHT),
        )
    }

    /// Frame-comparison slack — within one point of the expected edge.
    const TOLERANCE: f64 = 1.0;

    /// Mounts `content` through the real embedding path and lays the
    /// window out once. The window takes the screen's bounds, so it
    /// carries the device's real `safeAreaInsets` — the notch band
    /// above and the home-indicator band below — and the covered chrome
    /// bands, the navigation bar's band and the home indicator stay
    /// exercised by the container region the window itself reports.
    ///
    /// # Panics
    ///
    /// When the window never reports a nonzero top and bottom container
    /// band — the device type the suite is pinned to must carry both.
    fn mount(content: impl View) -> UIKitMount {
        let env = render_environment();
        let mount = mount_uikit(mtm(), AnyView::new(content), &env);
        let bands = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            mount.host.safeAreaInsets().top > 0.0 && mount.host.safeAreaInsets().bottom > 0.0
        });
        assert!(
            bands,
            "the test window must report nonzero top and bottom container bands"
        );
        mount
    }

    /// The first view of type `T` in a depth-first walk of `view`'s subtree.
    fn find_view<T: objc2::DowncastTarget>(view: &PlatformView) -> Option<Retained<PlatformView>> {
        for sub in view::subviews(view) {
            if sub.downcast_ref::<T>().is_some() {
                return Some(sub);
            }
            if let Some(found) = find_view::<T>(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// The first text leaf showing `text` in a depth-first walk — a
    /// `Label` whose rendered string matches exactly.
    fn find_label(view: &PlatformView, text: &str) -> Option<Retained<PlatformView>> {
        for sub in view::subviews(view) {
            if let Some(label) = sub.downcast_ref::<Label>()
                && label.text().map(|t| t.to_string()).as_deref() == Some(text)
            {
                return Some(sub);
            }
            if let Some(found) = find_label(&sub, text) {
                return Some(found);
            }
        }
        None
    }

    /// `view`'s frame in window coordinates.
    fn window_frame(view: &PlatformView) -> CGRect {
        view.convertRect_toView(view.bounds(), None)
    }

    /// The mounted leaf's view — what `mount_uikit` mounted into the host.
    fn leaf(mount: &UIKitMount) -> Retained<PlatformView> {
        Retained::from(mount.content.view())
    }

    /// Descends `view` through single-subview wrappers — the containers a
    /// mounted leaf passes through on its way into the host — to the
    /// first view that fans out to two or more children.
    fn fanned(view: &PlatformView) -> Retained<PlatformView> {
        let mut current = Retained::from(view);
        loop {
            let children = view::subviews(&current);
            if children.len() != 1 {
                return current;
            }
            current = children
                .into_iter()
                .next()
                .expect("a single-subview chain continues");
        }
    }

    /// The children of `mount`'s leaf, descending the transparent
    /// single-subview wrappers a mounted tree adds between the leaf and
    /// the real container.
    fn children(mount: &UIKitMount) -> Vec<Retained<PlatformView>> {
        view::subviews(&fanned(&leaf(mount)))
    }

    /// The container region's top boundary: the root host's
    /// `safeAreaInsets` — the window's ambient insets — the same
    /// boundary the layout's region context reads.
    fn container_top(mount: &UIKitMount) -> f64 {
        mount.host.safeAreaInsets().top
    }

    /// The container region's bottom boundary.
    fn container_bottom(mount: &UIKitMount) -> f64 {
        mount.host.bounds().size.height - mount.host.safeAreaInsets().bottom
    }

    /// The window's keyboard region frame — `CGRect::ZERO` until a
    /// notification lands or the first layout pass seeds it.
    fn region_frame(view: &UIView) -> CGRect {
        keyboard::window_keyboard(view)
            .expect("a windowed view reports its window's region")
            .0
    }

    /// The keyboard region's top boundary — `window.height` once the
    /// keyboard is gone.
    fn keyboard_top(mount: &UIKitMount) -> f64 {
        region_frame(&mount.host).origin.y
    }

    /// The `userInfo` a real keyboard notification carries: the end frame
    /// in screen coordinates, the duration and the animation curve.
    fn keyboard_user_info(end_frame: CGRect, duration: f64, curve: i64) -> Retained<NSDictionary> {
        let frame = NSValue::new(end_frame);
        let duration = NSNumber::new_f64(duration);
        let curve = NSNumber::new_i64(curve);
        let dict = NSDictionary::<NSString, AnyObject>::from_retained_objects(
            &[
                // SAFETY: `UIKit` exports the keys as `NSString`
                // constants for the process's lifetime.
                unsafe { UIKeyboardFrameEndUserInfoKey },
                // SAFETY: `UIKit` exports the keys as `NSString`
                // constants for the process's lifetime.
                unsafe { UIKeyboardAnimationDurationUserInfoKey },
                // SAFETY: `UIKit` exports the keys as `NSString`
                // constants for the process's lifetime.
                unsafe { UIKeyboardAnimationCurveUserInfoKey },
            ],
            &[
                // SAFETY: every `NSObject` is an `AnyObject`; the cast
                // only erases the concrete class.
                unsafe { Retained::cast_unchecked::<AnyObject>(frame) },
                // SAFETY: every `NSObject` is an `AnyObject`; the cast
                // only erases the concrete class.
                unsafe { Retained::cast_unchecked::<AnyObject>(duration) },
                // SAFETY: every `NSObject` is an `AnyObject`; the cast
                // only erases the concrete class.
                unsafe { Retained::cast_unchecked::<AnyObject>(curve) },
            ],
        );
        // SAFETY: the generic key and value parameters are erased at the
        // class boundary — the same `NSDictionary` `UIKit` hands the
        // observer.
        unsafe { Retained::cast_unchecked(dict) }
    }

    /// The animation a real keyboard notification reports: 250 ms on
    /// the keyboard's own curve — `UIViewAnimationCurveKeyboard`, which
    /// `UIKit` posts for keyboard transitions.
    const KEYBOARD_ANIMATION: (f64, i64) = (0.25, 7);

    /// Posts a keyboard `notification` with `end_frame` into `window` —
    /// the way `UIKit` announces the keyboard, duration and curve
    /// included — and pumps the main queue until `applied` holds, the
    /// region frame or laid-out geometry the caller waits on. `UIKit`
    /// posts `UIKeyboardWillChangeFrameNotification` for every
    /// transition alongside the semantic name, so the post sends both
    /// when the semantic name is not the frame change itself.
    fn post_keyboard_raw(
        window: &cocoa_ui::objc2_ui_kit::UIWindow,
        name: &'static NSString,
        end_frame: CGRect,
        applied: impl Fn() -> bool,
    ) {
        let user_info = keyboard_user_info(end_frame, KEYBOARD_ANIMATION.0, KEYBOARD_ANIMATION.1);
        // SAFETY: `UIKit` exports the name as a constant.
        let frame_change: &'static NSString = unsafe { UIKeyboardWillChangeFrameNotification };
        let names: &[&'static NSString] = if std::ptr::eq(name, frame_change) {
            &[name]
        } else {
            &[name, frame_change]
        };
        for notification in names {
            // SAFETY: a real keyboard notification — a name `UIKit`
            // exports, the `userInfo` shape it documents.
            unsafe {
                NSNotificationCenter::defaultCenter().postNotificationName_object_userInfo(
                    notification,
                    None,
                    Some(&user_info),
                );
            }
        }
        let landed = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            window.layoutIfNeeded();
            applied()
        });
        assert!(landed, "the keyboard frame never reached its region");
    }

    /// Posts `notification` into `mount`'s window and pumps until the
    /// window root has applied the end frame.
    fn post_keyboard(mount: &UIKitMount, name: &'static NSString, end_frame: CGRect) {
        post_keyboard_raw(&mount.window, name, end_frame, || {
            region_frame(&mount.host) == end_frame
        });
    }

    /// Posts `UIKeyboardWillShowNotification` for the window's keyboard
    /// end frame.
    fn show_keyboard(mount: &UIKitMount) {
        let end_frame = keyboard_end(&mount.window);
        // SAFETY: `UIKit` exports the name as a constant.
        post_keyboard(mount, unsafe { UIKeyboardWillShowNotification }, end_frame);
    }

    /// Posts `UIKeyboardWillChangeFrameNotification` for `end_frame`.
    fn change_keyboard(mount: &UIKitMount, end_frame: CGRect) {
        // SAFETY: `UIKit` exports the name as a constant.
        post_keyboard(
            mount,
            unsafe { UIKeyboardWillChangeFrameNotification },
            end_frame,
        );
    }

    /// Posts `UIKeyboardWillHideNotification`: the keyboard slides to just
    /// under the window's bottom edge and the region empties.
    fn hide_keyboard(mount: &UIKitMount) {
        // SAFETY: `UIKit` exports the name as a constant.
        post_keyboard(
            mount,
            unsafe { UIKeyboardWillHideNotification },
            keyboard_hide_end(&mount.window),
        );
    }

    /// Lays the window out again and pumps until `predicate` holds — the
    /// completion signal for the `UIView` animation the backend applies
    /// the inset change inside.
    fn pump_layout(mount: &UIKitMount, predicate: impl Fn() -> bool) {
        let landed = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            predicate()
        });
        assert!(landed, "the layout never settled to the expected frame");
    }

    /// Pumps until `view`'s frame bottom in window coordinates sits on
    /// `boundary` — within the sub-pixel touch slack.
    fn pump_until_bottom_on(mount: &UIKitMount, view: &PlatformView, boundary: f64) {
        let landed = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            let frame = window_frame(view);
            (frame.origin.y + frame.size.height - boundary).abs() <= 0.5
        });
        assert!(
            landed,
            "the bottom never landed on {boundary} — frame {:?}",
            window_frame(view),
        );
    }

    /// Focuses a field inside the mounted scroll — `becomeFirstResponder`
    /// fires the real `UITextFieldTextDidBeginEditingNotification` the
    /// surface's observer scrolls on — and pumps until the animated
    /// clearance lands the field's frame bottom on `boundary()`, read
    /// live so a keyboard that moves during the pump stays the target.
    fn focus_and_clear(mount: &UIKitMount, field: &PlatformView, boundary: impl Fn() -> f64) {
        assert!(field.becomeFirstResponder(), "the field must accept focus");
        pump_layout(mount, || (window_bottom(field) - boundary()).abs() <= 0.5);
    }

    /// A messenger-style panel: a scrollable conversation over the
    /// composer `hstack` the `keyboard_panel` example builds — a
    /// stretching text field beside a button.
    fn conversation_panel(draft: &Binding<Str>) -> impl View {
        vstack((
            scroll(vstack((
                text("Are we still on for Saturday?"),
                text("Nine works. The weather is supposed to be clear."),
            ))),
            hstack({
                let draft = draft.clone();
                (
                    field("Message", &draft),
                    button("Send").action(move || draft.set(Str::from(""))),
                )
            })
            .padding_with(12.0)
            .background(waterui::graphics::color::Srgb::from_hex("#F7F7F9")),
        ))
    }

    /// The composer keeps the height its content measures: the field
    /// stretches horizontally, so its height must be its own intrinsic
    /// answer under every proposal — never the offered height echoed
    /// back. When the keyboard shrinks the panel's region the field must
    /// stay one field tall beside the Send button, not split the window
    /// in two. (The `d6171eed2` bug shape: the field answered its own
    /// open offer, so the `hstack` claimed the whole band.)
    fn a_composer_hstack_keeps_its_intrinsic_height_under_the_keyboard() {
        let draft = binding(Str::from(""));
        let mount = mount(conversation_panel(&draft));

        let field = find_view::<cocoa_ui::uikit::TextField>(&mount.host)
            .expect("the composer mounts a text field");
        let field_host = field
            .superview()
            .expect("the field sits inside its leaf host");
        let hstack = field_host
            .superview()
            .expect("the field sits inside the composer's hstack");
        let composer = hstack
            .superview()
            .and_then(|padding| padding.superview())
            .expect("padding and background wrap the composer's hstack");

        // The same answers the leaf's measure gives `sizeThatFits`: the
        // proposals the negotiation probes — minimum, a finite offer and
        // the maximum probe §2 reserves an INFINITY answer for on the
        // axis the field stretches.
        let probes = [
            CGSize::new(366.0, 0.0),
            CGSize::new(366.0, 200.0),
            CGSize::new(366.0, f64::INFINITY),
        ]
        .map(|size| field_host.sizeThatFits(size));

        // Every level's measure answers for the same probes: the level
        // that first echoes a finite offer up the chain is where the
        // band claim starts.
        let chain: Vec<(CGRect, [CGSize; 3])> = std::iter::once(field_host.clone())
            .chain(std::iter::successors(field_host.superview(), |v| {
                v.superview()
            }))
            .map(|host| {
                (
                    host.frame(),
                    [
                        CGSize::new(366.0, 0.0),
                        CGSize::new(366.0, 200.0),
                        CGSize::new(366.0, f64::INFINITY),
                    ]
                    .map(|size| host.sizeThatFits(size)),
                )
            })
            .collect();

        assert!(
            view::frame(&field_host).size.height < 120.0,
            "the field must stay near its intrinsic height — frame {:?}, composer {:?}, probes {probes:?}, chain {chain:?}",
            field_host.frame(),
            composer.frame(),
        );
        assert!(
            view::frame(&hstack).size.height < 120.0,
            "the composer row must not claim the band — frame {:?}, probes {probes:?}",
            hstack.frame(),
        );

        show_keyboard(&mount);
        let chain_after: Vec<(CGRect, [CGSize; 3])> = std::iter::once(field_host.clone())
            .chain(std::iter::successors(field_host.superview(), |v| {
                v.superview()
            }))
            .map(|host| {
                (
                    host.frame(),
                    [
                        CGSize::new(366.0, 0.0),
                        CGSize::new(366.0, 200.0),
                        CGSize::new(366.0, f64::INFINITY),
                    ]
                    .map(|size| host.sizeThatFits(size)),
                )
            })
            .collect();
        assert!(
            view::frame(&field_host).size.height < 120.0,
            "under the keyboard the field must still be one field tall — frame {:?}, probes {probes:?}, chain {chain_after:?}",
            field_host.frame(),
        );
        assert!(
            view::frame(&hstack).size.height < 120.0,
            "under the keyboard the composer row must still be its own height — frame {:?}, probes {probes:?}, chain {chain_after:?}",
            hstack.frame(),
        );
        // The composer's container manages the safe area and touches
        // the keyboard edge, so it extends under the band (the panel's
        // fill behind the translucent keyboard) — but only downward:
        // its top edge still sits inside the content region.
        assert!(
            window_frame(&composer).origin.y < keyboard_top(&mount) - 20.0,
            "the composer must not grow upward into the region — frame {:?}",
            composer.frame(),
        );
    }

    /// A styled card: a `body` label padded inside a fill background —
    /// the same building block the Hydrolysis suite uses.
    fn card(label: &'static str) -> impl View {
        text(label)
            .body()
            .padding_with(8.0)
            .background(Color::new(Srgb::new(0.0, 0.35, 0.85)))
    }

    /// A labelled probe carrying an `.ignore_safe_area` release — the
    /// host's frame lands on the released boundary only when the edge its
    /// frame touches is reachable; a covered edge releases nothing.
    fn edge_probe(
        view: impl View,
        label: &'static str,
        ignore: impl Into<waterui::layout::safe_area::IgnoreSafeArea>,
    ) -> impl View {
        view.ignore_safe_area(ignore).a11y_id(label)
    }

    /// The first view carrying `accessibilityIdentifier` in a depth-first
    /// walk — how the probes are located without pixel hunting.
    fn find_id(view: &PlatformView, label: &str) -> Option<Retained<PlatformView>> {
        for sub in view::subviews(view) {
            if view::accessibility_identifier(&sub).as_deref() == Some(label) {
                return Some(sub);
            }
            if let Some(found) = find_id(&sub, label) {
                return Some(found);
            }
        }
        None
    }

    /// The first view whose Objective-C class is literally `name` —
    /// how a `UIToolbar` is found when the vendored `objc2-ui-kit`
    /// carries no binding for the class.
    fn find_class(
        view: &PlatformView,
        name: &'static std::ffi::CStr,
    ) -> Option<Retained<PlatformView>> {
        for sub in view::subviews(view) {
            if sub.class().name() == name {
                return Some(sub);
            }
            if let Some(found) = find_class(&sub, name) {
                return Some(found);
            }
        }
        None
    }

    /// Every view of type `T` in `view`'s subtree, ancestors first.
    fn find_views<T: objc2::DowncastTarget>(view: &PlatformView) -> Vec<Retained<PlatformView>> {
        let mut found = Vec::new();
        for sub in view::subviews(view) {
            if sub.downcast_ref::<T>().is_some() {
                found.push(sub.clone());
            }
            found.extend(find_views::<T>(&sub));
        }
        found
    }

    /// `view`'s frame bottom in window coordinates.
    fn window_bottom(view: &PlatformView) -> f64 {
        let frame = window_frame(view);
        frame.origin.y + frame.size.height
    }

    /// `view`'s frame top in window coordinates.
    fn window_top(view: &PlatformView) -> f64 {
        window_frame(view).origin.y
    }

    /// Asserts `view`'s window-space bottom lands within `TOLERANCE` of
    /// `expected`.
    fn expect_bottom(view: &PlatformView, expected: f64, note: &str) {
        let bottom = window_bottom(view);
        assert!(
            (bottom - expected).abs() <= TOLERANCE,
            "{note}: bottom {bottom} differs from {expected} by more than {TOLERANCE}"
        );
    }

    /// Asserts `view`'s window-space top lands within `TOLERANCE` of
    /// `expected`.
    fn expect_top(view: &PlatformView, expected: f64, note: &str) {
        let top = window_top(view);
        assert!(
            (top - expected).abs() <= TOLERANCE,
            "{note}: top {top} differs from {expected} by more than {TOLERANCE}"
        );
    }

    /// The scroll above the composer ends above the keyboard band — the
    /// band covers none of its frame — so its content inset takes no
    /// keyboard contribution and stays zero, and the delta-write never
    /// rewrites another `contentInset.bottom` term.
    fn a_scroll_above_a_composer_keeps_a_zero_keyboard_inset() {
        let draft = binding(Str::from(""));
        let mount = mount(conversation_panel(&draft));
        let surface = find_view::<ScrollView>(&mount.host).expect("scroll mounts a UIScrollView");
        let scroll_view = surface
            .downcast_ref::<ScrollView>()
            .expect("the found view is a scroll view");
        show_keyboard(&mount);
        assert!(
            window_bottom(&surface) <= keyboard_top(&mount) + TOLERANCE,
            "precondition: the scroll ends above the keyboard band — bottom {}, band top {}",
            window_bottom(&surface),
            keyboard_top(&mount),
        );
        assert!(
            scroll_view.contentInset().bottom.abs() <= f64::EPSILON,
            "the scroll keeps a zero keyboard inset — contentInset {:?}",
            scroll_view.contentInset(),
        );
    }

    /// A `UITextView` is a `UIScrollView` but not a kit scroll surface —
    /// the `cocoaUiIsScrollSurface` marker, not the class chain, is what
    /// counts. Parked around a real surface it must not claim the
    /// enclosing-surface role: the inner surface still computes its own
    /// keyboard inset and still clears a focused field.
    fn a_text_view_inside_the_tree_is_not_a_scroll_surface() {
        let value = binding(Str::from(""));
        let mount = mount(scroll(vstack((
            spacer().size(390.0, 560.0),
            field("Message", &value).size(350.0, 44.0),
            spacer().size(390.0, 380.0),
        ))));
        let surface = find_view::<ScrollView>(&mount.host).expect("scroll mounts a UIScrollView");
        let scroll_view = surface
            .downcast_ref::<ScrollView>()
            .expect("the found view is a scroll view");
        let field = find_view::<TextField>(&mount.host).expect("the form mounts the field");

        // A raw `UITextView` wrapped around the surface, spanning the
        // window — a foreign `UIScrollView` the surface check must not
        // count as an enclosing scroll surface.
        // SAFETY: `initWithFrame:` is `UITextView`'s designated
        // initializer — a plain untracked text view.
        let text_view: Retained<UITextView> = unsafe {
            msg_send![
                UITextView::alloc(mtm()),
                initWithFrame: mount.window.bounds()
            ]
        };
        mount.host.addSubview(&text_view);
        let frame = window_frame(&surface);
        surface.removeFromSuperview();
        text_view.addSubview(&surface);
        // The text view's bounds share the window's origin, so the
        // surface's window frame is its frame in the new parent.
        view::set_frame(&surface, frame.into());

        assert!(
            !text_view.respondsToSelector(cocoa_ui::objc2::sel!(cocoaUiIsScrollSurface)),
            "a `UITextView` does not answer the kit scroll-surface marker",
        );
        show_keyboard(&mount);
        let cover = window_bottom(&surface) - keyboard_top(&mount);
        let want = (cover - surface.safeAreaInsets().bottom).max(0.0);
        // The notification's own mark walk crosses the foreign parent —
        // no manual drive: the surface's pass must apply the inset, and
        // `nested_in_scroll` must not count the `UITextView` as an
        // enclosing surface.
        pump_layout(&mount, || {
            (scroll_view.contentInset().bottom - want).abs() <= TOLERANCE
        });
        focus_and_clear(&mount, &field, || keyboard_top(&mount));
    }

    /// The region mark reaches a background-slot container that moves
    /// without resizing: pinned at the region's bottom, the strip holds
    /// its height while the keyboard band lifts its position — a pure
    /// move gives the container no `layoutSubviews` of its own, so the
    /// tracking owner's per-notification mark is what re-places the
    /// fill against each new boundary.
    fn a_moving_container_re_extends_its_fill_to_each_region_boundary() {
        let mount = mount(vstack((
            spacer(),
            spacer()
                .size(390.0, 48.0)
                .background(Color::new(Srgb::new(0.3, 0.5, 0.9))),
        )));
        let fill = find_view::<ColorView>(&mount.host).expect("the fill is mounted");
        let window_bottom_edge = mount.window.bounds().size.height;
        show_keyboard(&mount);
        expect_top(
            &fill,
            keyboard_top(&mount) - 48.0,
            "the fill follows the moved container to the keyboard band",
        );
        expect_bottom(
            &fill,
            window_bottom_edge,
            "the fill still paints to the window's bottom edge",
        );
        hide_keyboard(&mount);
        expect_top(
            &fill,
            container_bottom(&mount) - 48.0,
            "the fill re-extends when the container moves back",
        );
        expect_bottom(
            &fill,
            window_bottom_edge,
            "the fill still paints to the window's bottom edge",
        );
    }

    /// A user scroll that pushes the focused field under the keyboard
    /// is not undone by the layout pass: the pass re-clears the field
    /// only when the keyboard contribution itself changes — never
    /// because a content-offset change re-ran it.
    fn a_user_scroll_under_the_keyboard_is_not_undone_by_the_layout_pass() {
        let value = binding(Str::from(""));
        let mount = mount(scroll(vstack((
            spacer().size(390.0, 560.0),
            field("Message", &value).size(350.0, 44.0),
            spacer().size(390.0, 380.0),
        ))));
        show_keyboard(&mount);
        let surface = find_view::<ScrollView>(&mount.host).expect("scroll mounts a UIScrollView");
        let scroll_view = surface
            .downcast_ref::<ScrollView>()
            .expect("the found view is a scroll view");
        let field = find_view::<TextField>(&mount.host).expect("the form mounts the field");
        focus_and_clear(&mount, &field, || keyboard_top(&mount));

        // The user's own scroll pushes the field back under the band.
        scroll_view.setContentOffset_animated(CGPoint::new(0.0, 0.0), false);
        assert!(
            window_bottom(&field) > keyboard_top(&mount) + TOLERANCE,
            "precondition: the offset pushed the field back under the keyboard"
        );
        mount.window.layoutIfNeeded();
        assert!(
            scroll_view.contentOffset().y.abs() <= TOLERANCE,
            "the layout pass left the user's offset alone — {:?}",
            scroll_view.contentOffset(),
        );
    }

    /// A window owns exactly one keyboard region however the host
    /// reaches it: content carrying nested hosts — the page inside a
    /// `NavigationStack` — and a scroll mounts into an embedded host
    /// before and after the host joins the windowed tree; both orders
    /// read the same region object and the scroll takes the same
    /// inset.
    fn nested_hosts_and_a_scroll_share_the_windows_keyboard_region() {
        for content_first in [true, false] {
            let value = binding(Str::from(""));
            let env = render_environment();
            let mount = mount_uikit_embedded(
                mtm(),
                AnyView::new(NavigationStack::new(NavigationView::new(
                    "Form",
                    scroll(vstack((
                        spacer().size(390.0, 560.0),
                        field("Message", &value).size(350.0, 44.0),
                        spacer().size(390.0, 380.0),
                    ))),
                ))),
                &env,
                content_first,
            );
            mount.window.layoutIfNeeded();
            let end = keyboard_end(&mount.window);
            post_keyboard_raw(
                &mount.window,
                // SAFETY: `UIKit` exports the name as a constant.
                unsafe { UIKeyboardWillShowNotification },
                end,
                || region_frame(&mount.host) == end,
            );
            let surface =
                find_view::<ScrollView>(&mount.host).expect("scroll mounts a UIScrollView");
            let scroll_view = surface
                .downcast_ref::<ScrollView>()
                .expect("the found view is a scroll view");
            let cover = window_bottom(&surface) - end.origin.y;
            let want = (cover - surface.safeAreaInsets().bottom).max(0.0);
            let landed = pump_main_until(MAIN_QUEUE_DEADLINE, || {
                mount.window.layoutIfNeeded();
                (scroll_view.contentInset().bottom - want).abs() <= TOLERANCE
            });
            assert!(
                landed,
                "the scroll read the window's keyboard region — {} vs {want}",
                scroll_view.contentInset().bottom,
            );
        }
    }

    /// The `UINavigationController` owning a view in the subtree —
    /// either the view's own controller is the nav controller, or the
    /// page controller answers one.
    fn nav_controller_in(view: &PlatformView) -> Option<Retained<UINavigationController>> {
        if let Some(controller) = owning_controller(view) {
            if let Ok(nav) = controller.clone().downcast() {
                return Some(nav);
            }
            if let Some(nav) = controller.navigationController() {
                return Some(nav);
            }
        }
        for sub in view::subviews(view) {
            if let Some(found) = nav_controller_in(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// A surface pushed into the keyboard band at a constant size — a
    /// banner moving it down — recomputes the covered depth from its
    /// new window frame without a notification. The wrapper case is the
    /// real shape: a container the vstack translates gives its scroll no
    /// `setFrame:` of its own, so the wrapper's `setFrame:` marks the
    /// region readers in its subtree — the surface re-reads the band it
    /// moved into, and the wrapper's own fill re-extends. The bare case
    /// — the surface's own `setFrame:` — keeps its mark too.
    fn a_scroll_pushed_into_the_band_at_constant_size_updates_its_inset() {
        // The scroll sits inside a background wrapper the container
        // region pushes down at constant size — a banner's exact move.
        let mount = mount(vstack((
            scroll(vstack((
                spacer().size(390.0, 240.0),
                card("row"),
                spacer().size(390.0, 240.0),
            )))
            .size(390.0, 300.0)
            .background(Color::new(Srgb::new(0.2, 0.4, 0.7))),
            spacer(),
        )));
        show_keyboard(&mount);
        let surface = find_view::<ScrollView>(&mount.host).expect("scroll mounts a UIScrollView");
        let scroll_view = surface
            .downcast_ref::<ScrollView>()
            .expect("the found view is a scroll view");
        let clear = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            scroll_view.contentInset().bottom.abs() <= TOLERANCE
        });
        assert!(clear, "precondition: the band does not reach the scroll");
        // The wrapper's fill is the ColorView sibling of the scroll's
        // surface; its frame is the container's own while it touches no
        // region edge.
        let fill = find_view::<ColorView>(&mount.host).expect("the wrapper mounts its fill");
        let wrapper = fill.superview().expect("the fill sits inside the wrapper");
        assert!(
            (window_top(&fill) - window_top(&wrapper)).abs() <= TOLERANCE,
            "precondition: the fill covers the wrapper — {:?} vs {:?}",
            window_frame(&fill),
            window_frame(&wrapper),
        );

        // The push: the wrapper goes down 300pt at constant size — the
        // scroll's own `setFrame:` never runs (its superview-relative
        // frame is unchanged), so the wrapper's mark is the only path.
        let frame = window_frame(&wrapper);
        view::set_frame(
            &wrapper,
            cocoa_ui::Rect::new(
                frame.origin.x,
                frame.origin.y + 300.0,
                frame.size.width,
                frame.size.height,
            ),
        );
        let grew = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            scroll_view.contentInset().bottom > TOLERANCE
        });
        assert!(
            grew,
            "the wrapper's move re-derived the scroll's inset — {}",
            scroll_view.contentInset().bottom,
        );
        // The wrapper's own layout pass re-derived its fill too: off the
        // window's top edge, the fill's top re-docks at the container's
        // moved top — a stale fill would still paint from the old y.
        assert!(
            (window_top(&fill) - window_top(&wrapper)).abs() <= TOLERANCE,
            "the wrapper's fill followed the move — fill {:?} vs container {:?}",
            window_frame(&fill),
            window_frame(&wrapper),
        );

        // The bare surface: its own `setFrame:` marks it.
        let mtm = mtm();
        let scroll = ScrollView::new(mtm, true, false);
        let window = crate::leaf::attach(mtm, &scroll);
        view::set_frame(&scroll, cocoa_ui::Rect::new(0.0, 100.0, 390.0, 400.0));
        window.layoutIfNeeded();
        let end = keyboard_end(&window);
        post_keyboard_raw(
            &window,
            // SAFETY: `UIKit` exports the name as a constant.
            unsafe { UIKeyboardWillShowNotification },
            end,
            || region_frame(&scroll) == end,
        );
        let clear = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            window.layoutIfNeeded();
            scroll.contentInset().bottom.abs() <= TOLERANCE
        });
        assert!(clear, "precondition: the band does not reach the scroll");
        let before = scroll.contentInset().bottom;

        view::set_frame(&scroll, cocoa_ui::Rect::new(0.0, 200.0, 390.0, 400.0));
        let grown = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            window.layoutIfNeeded();
            scroll.contentInset().bottom >= before + 50.0
        });
        assert!(
            grown,
            "a same-size move into the band re-derived the inset — {before} vs {}",
            scroll.contentInset().bottom,
        );
        window.setHidden(true);
    }

    /// A safe-area change at the same frame — a toolbar appearing while
    /// the keyboard is up — changes the depth the surface must inset
    /// for: the gate reads the bottom safe-area inset too, or the band
    /// would be counted twice.
    fn a_toolbar_appearing_while_the_keyboard_is_up_rederives_the_inset() {
        let mount = mount(scroll(vstack((
            spacer().size(390.0, 560.0),
            card("tail"),
            spacer().size(390.0, 380.0),
        ))));
        let surface = find_view::<ScrollView>(&mount.host).expect("scroll mounts a UIScrollView");
        let scroll_view = surface
            .downcast_ref::<ScrollView>()
            .expect("the found view is a scroll view");
        show_keyboard(&mount);
        let applied = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            scroll_view.contentInset().bottom > TOLERANCE
        });
        assert!(applied, "precondition: the keyboard covers the scroll");
        let inset_before = scroll_view.contentInset().bottom;
        let safe_before = surface.safeAreaInsets().bottom;

        // The toolbar arriving is an `additionalSafeAreaInsets` change —
        // the surface's frame does not move.
        mount
            .window
            .rootViewController()
            .expect("the mount window has a root controller")
            .setAdditionalSafeAreaInsets(UIEdgeInsets {
                top: 0.0,
                left: 0.0,
                bottom: 40.0,
                right: 0.0,
            });
        let grew = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            surface.safeAreaInsets().bottom >= safe_before + 40.0 - TOLERANCE
        });
        assert!(grew, "precondition: the added safe area reached the scroll");
        let landed = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            (scroll_view.contentInset().bottom - (inset_before - 40.0)).abs() <= TOLERANCE
        });
        assert!(
            landed,
            "a same-frame safe-area change re-derived the inset — {inset_before} vs {}",
            scroll_view.contentInset().bottom,
        );
    }

    /// A keyboard notification posted before the first `WaterUI` view
    /// reaches a window is not replayed: the region comes into
    /// existence with the mount and stays `CGRect::ZERO` — nothing is
    /// docked in this harness — until the first real notification after
    /// it arrives, which lands on time.
    fn a_keyboard_notification_before_mount_is_not_replayed() {
        let mtm = mtm();
        let env = render_environment();
        let window = spawned_window(mtm);
        window.makeKeyAndVisible();
        let end = keyboard_end(&window);
        // Shown before any `WaterUI` view reaches the window: no region
        // exists to hear it.
        post_keyboard_raw(
            &window,
            // SAFETY: `UIKit` exports the name as a constant.
            unsafe { UIKeyboardWillShowNotification },
            end,
            || true,
        );
        let mount = mount_uikit_window(
            mtm,
            AnyView::new(scroll(vstack((spacer().size(390.0, 1200.0),)))),
            &env,
            window,
        );
        assert_eq!(
            region_frame(&mount.host),
            CGRect::ZERO,
            "a pre-mount notification must not replay into the region",
        );
        show_keyboard(&mount);
        assert_eq!(
            region_frame(&mount.host),
            end,
            "the first notification after the mount lands on time",
        );
    }

    /// The resting `keyboardLayoutGuide` reports the window's bottom
    /// safe-area band — `usesBottomSafeArea` parks it there while no
    /// keyboard is docked — and a band-equal guide frame must seed
    /// nothing: it is the parked guide, not a keyboard.
    fn a_guide_frame_equal_to_the_bottom_band_seeds_zero() {
        let mtm = mtm();
        let window = spawned_window(mtm);
        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        let band_top = window.bounds().size.height - window.safeAreaInsets().bottom;
        let band = CGRect::new(
            CGPoint::new(0.0, band_top),
            CGSize::new(window.bounds().size.width, window.safeAreaInsets().bottom),
        );
        assert_eq!(
            keyboard::guide_seed_frame(&window, band),
            CGRect::ZERO,
            "a frame parked inside the band is not a keyboard",
        );
        window.setHidden(true);
    }

    /// A `keyboardLayoutGuide` frame whose top edge sits above the
    /// window's bottom safe-area band reports a docked keyboard and
    /// seeds the region with the frame the guide reports.
    fn a_guide_frame_above_the_bottom_band_seeds_the_frame() {
        let mtm = mtm();
        let window = spawned_window(mtm);
        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        let keyboard = CGRect::new(
            CGPoint::new(0.0, 300.0),
            CGSize::new(window.bounds().size.width, 200.0),
        );
        assert_eq!(
            keyboard::guide_seed_frame(&window, keyboard),
            keyboard,
            "a frame above the band seeds as the docked keyboard",
        );
        window.setHidden(true);
    }

    /// A page pushed over while the keyboard is up and hidden under
    /// while it is away misses no change: on pop the field page's
    /// scroll inset and its fill are back at the keyboard-free values
    /// — the window's one region object marked every reader and the
    /// re-entering views re-read it.
    fn a_page_pushed_over_the_keyboard_returns_keyboard_free() {
        let value = binding(Str::from(""));
        let mount = mount(NavigationStack::new(NavigationView::new(
            "Form",
            scroll(vstack((
                spacer().size(390.0, 560.0),
                field("Message", &value).size(350.0, 44.0),
                spacer().size(390.0, 380.0),
            )))
            .background(Color::new(Srgb::new(0.2, 0.4, 0.8))),
        )));
        let nav = nav_controller_in(&mount.host).expect("a stack mounts the nav controller");
        let surface = find_view::<ScrollView>(&mount.host).expect("scroll mounts a UIScrollView");
        let scroll_view = surface
            .downcast_ref::<ScrollView>()
            .expect("the found view is a scroll view");
        let fill = find_view::<ColorView>(&mount.host).expect("the fill is mounted");
        let free_inset = scroll_view.contentInset().bottom;

        show_keyboard(&mount);
        let covered = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            scroll_view.contentInset().bottom > free_inset + 1.0
        });
        assert!(covered, "precondition: the keyboard covers the scroll");

        // A page hides the field's page; the keyboard goes away while
        // the page is out of the visible stack.
        let cover_scroll = UIScrollView::new(mtm());
        let cover_page = NavContentController::new(mtm(), &cover_scroll);
        nav.pushViewController_animated(&cover_page, false);
        mount.window.layoutIfNeeded();
        assert!(
            surface.window().is_none() && fill.window().is_none(),
            "precondition: the push carried the field's page out of the window",
        );
        hide_keyboard(&mount);
        nav.popViewControllerAnimated(false);

        let restored = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            (scroll_view.contentInset().bottom - free_inset).abs() <= TOLERANCE
        });
        assert!(
            restored,
            "the returning page's scroll inset is keyboard-free — {} vs {free_inset}",
            scroll_view.contentInset().bottom,
        );
        expect_bottom(
            &fill,
            mount.window.bounds().size.height,
            "the returning page's fill re-extends to the window's bottom edge",
        );
    }

    /// §7.1 "Layout avoids the regions": the root leaf's children lay
    /// out inside the deepest region on every edge — the window's
    /// `safeAreaInsets` form the container region and the keyboard band
    /// sits deeper still. The leaf itself is a safe-area manager and
    /// fills the window; its children carry the bound.
    fn root_content_stays_inside_the_safe_area() {
        let mount = mount(vstack((card("content"), spacer())));
        let children = children(&mount);
        let spacer = &children[1];
        let label = find_label(&mount.host, "content").expect("the label is mounted");

        expect_top(
            &label,
            container_top(&mount) + 8.0,
            "the card content clears the container top band",
        );
        expect_bottom(
            spacer,
            container_bottom(&mount),
            "the stack's content clears the container bottom band",
        );

        show_keyboard(&mount);
        pump_until_bottom_on(&mount, spacer, keyboard_top(&mount));
    }

    /// `IgnoreSafeArea::ALL` releases every region on every edge — the
    /// released stack's children reach the window bounds even while the
    /// keyboard is up; the plain stack's children still stop at the
    /// keyboard boundary. A released leaf fills the window either way,
    /// so the assertion lands on the children the regions still bind.
    fn ignore_safe_area_all_reaches_the_window_origin() {
        let released = mount(vstack((card("released"), spacer())).ignore_safe_area(EdgeSet::ALL));
        let plain = mount(vstack((card("plain"), spacer())));
        show_keyboard(&released);
        show_keyboard(&plain);
        let bounds = released.window.bounds();
        let frame = window_frame(&leaf(&released));
        assert!(
            (frame.origin.x - bounds.origin.x).abs() <= TOLERANCE
                && (frame.origin.y - bounds.origin.y).abs() <= TOLERANCE
                && (frame.size.width - bounds.size.width).abs() <= TOLERANCE
                && (frame.size.height - bounds.size.height).abs() <= TOLERANCE,
            "an all-edges release reaches the window bounds — frame {frame:?}, bounds {bounds:?}"
        );
        expect_bottom(
            &children(&released)[1],
            bounds.size.height,
            "the released stack's children reach the window bottom",
        );
        expect_bottom(
            &children(&plain)[1],
            keyboard_top(&plain),
            "the plain stack's children stay bound by the keyboard",
        );
    }

    /// Naming only `TOP` releases the top band alone: the leaf's bottom
    /// still stops at the keyboard region.
    fn ignore_safe_area_releases_only_the_flagged_edges() {
        let mount = mount(vstack((card("content"), spacer())).ignore_safe_area(EdgeSet::TOP));
        show_keyboard(&mount);
        let leaf = leaf(&mount);
        expect_top(
            &leaf,
            mount.window.bounds().origin.y,
            "the named edge releases",
        );
        expect_bottom(
            &leaf,
            keyboard_top(&mount),
            "an edge the declaration does not name stays bound",
        );
    }

    /// A release fires only on an edge the laid-out frame touches: the
    /// top-touching probe releases to the window top, the bottom sibling
    /// — which does not touch the edge it names — stays put.
    fn an_ignore_releases_only_on_an_edge_the_frame_touches() {
        let mount = mount(vstack((
            edge_probe(card("touches"), "top-edge", EdgeSet::TOP),
            spacer(),
            card("plain"),
        )));
        show_keyboard(&mount);
        let probe = find_id(&mount.host, "top-edge").expect("the probe is mounted");
        expect_top(
            &probe,
            mount.window.bounds().origin.y,
            "a touched edge releases through the container band",
        );
        let sibling = find_label(&mount.host, "plain").expect("the sibling is mounted");
        expect_bottom(
            &sibling,
            keyboard_top(&mount) - 8.0,
            "the sibling names no edge and stays bound",
        );
    }

    /// A keyboard frame change relays the root out again: bottom region
    /// down → up → grown → hidden, the stack's bound child follows every
    /// frame — the leaf itself is a safe-area manager and fills the
    /// window regardless.
    fn a_safe_area_change_relays_out_the_root() {
        let mount = mount(vstack((card("content"), spacer())));
        let spacer = children(&mount).swap_remove(1);
        expect_bottom(&spacer, container_bottom(&mount), "keyboard down");

        show_keyboard(&mount);
        expect_bottom(&spacer, keyboard_top(&mount), "keyboard shown");

        let grown = keyboard_end_sized(&mount.window, 400.0);
        change_keyboard(&mount, grown);
        expect_bottom(&spacer, grown.origin.y, "the grown band lifts the content");

        hide_keyboard(&mount);
        expect_bottom(&spacer, container_bottom(&mount), "keyboard hidden");
    }

    /// `KEYBOARD.on(BOTTOM)` releases through the keyboard band down to the
    /// container boundary — the deepest region it does not name. A plain
    /// sibling at the same edge stays bound by the keyboard region.
    fn ignoring_only_the_keyboard_region_keeps_the_container_inset() {
        let mount = mount(vstack((
            card("plain"),
            spacer(),
            hstack((
                text("ref").body(),
                edge_probe(
                    text("released").body(),
                    "bottom-edge",
                    SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM),
                ),
            )),
        )));
        show_keyboard(&mount);
        let probe = find_id(&mount.host, "bottom-edge").expect("the probe is mounted");
        expect_bottom(
            &probe,
            container_bottom(&mount),
            "the keyboard release reaches through to the container boundary",
        );
        let sibling = find_label(&mount.host, "ref").expect("the sibling is mounted");
        expect_bottom(
            &sibling,
            keyboard_top(&mount),
            "the sibling naming nothing still stops at the keyboard",
        );
    }

    /// `CONTAINER.on(BOTTOM)` names the container band, not the keyboard —
    /// under the keyboard nothing is released and the leaf stops at the
    /// keyboard top; hiding the keyboard releases it to the window bottom.
    fn ignoring_only_the_container_region_releases_nothing_under_the_keyboard() {
        let mount = mount(
            vstack((spacer(), card("content")))
                .ignore_safe_area(SafeAreaRegions::CONTAINER.on(EdgeSet::BOTTOM)),
        );
        let leaf = leaf(&mount);
        show_keyboard(&mount);
        expect_bottom(
            &leaf,
            keyboard_top(&mount),
            "a container-only release clears nothing under the keyboard",
        );
        hide_keyboard(&mount);
        expect_bottom(
            &leaf,
            mount.window.bounds().size.height,
            "with the keyboard gone the named container band releases",
        );
    }

    /// A fractional keyboard frame is still a boundary the layout reads
    /// exactly — the bound child's bottom lands on the reported end
    /// frame's top.
    fn a_fractional_keyboard_still_touches_the_boundary() {
        let mount = mount(vstack((card("content"), spacer())));
        let spacer = children(&mount).swap_remove(1);
        let fractional = CGRect::new(
            CGPoint::new(0.0, mount.window.bounds().size.height - 336.25),
            CGSize::new(mount.window.bounds().size.width, 336.25),
        );
        change_keyboard(&mount, fractional);
        expect_bottom(
            &spacer,
            fractional.origin.y,
            "the fractional band is the boundary",
        );
    }

    /// A fill in a `.background` slot paints through the keyboard and the
    /// container band to the window edge, while the composer's laid-out
    /// content still ends on the keyboard boundary and the label rides
    /// above it. The composer container itself is a safe-area manager —
    /// its frame extends to the window edge it touches, carrying the fill.
    fn a_fill_background_paints_under_the_keyboard_region() {
        let mount = mount(vstack((spacer(), card("composer"))));
        show_keyboard(&mount);
        let children = children(&mount);
        let composer = &children[1];
        let fill = view::subviews(composer)
            .into_iter()
            .next()
            .expect("a background container holds its fill first");
        expect_bottom(
            &fill,
            mount.window.bounds().size.height,
            "the fill extends through both regions",
        );
        let label = find_label(&mount.host, "composer").expect("the label is mounted");
        expect_bottom(
            &label,
            keyboard_top(&mount) - 8.0,
            "the label keeps its padding above the boundary",
        );
    }

    /// An `.opacity` wrapper is transparent to the fill pass — the color
    /// inside still extends past the laid-out frame to the window edge,
    /// while the composer's content stays bound by the keyboard region.
    fn a_fill_inside_opacity_still_extends() {
        let mount = mount(vstack((
            spacer(),
            text("composer")
                .body()
                .padding_with(8.0)
                .background(Color::new(Srgb::new(0.85, 0.2, 0.3)).opacity(0.6)),
        )));
        show_keyboard(&mount);
        let fill = find_view::<ColorView>(&mount.host).expect("the opacity wrap keeps a ColorView");
        expect_bottom(
            &fill,
            mount.window.bounds().size.height,
            "the fill still extends under the keyboard",
        );
        let label = find_label(&mount.host, "composer").expect("the label is mounted");
        expect_bottom(
            &label,
            keyboard_top(&mount) - 8.0,
            "the content keeps its padding above the boundary",
        );
    }

    /// An `IgnoreSafeArea` on the fill replaces the default extension with
    /// what it names — `EdgeSet::NONE` names nothing, so the fill stops at
    /// the laid-out frame like any other leaf.
    fn a_fill_with_an_empty_ignore_extends_nowhere() {
        let mount = mount(vstack((
            spacer(),
            text("composer")
                .body()
                .padding_with(8.0)
                .background(Color::new(Srgb::new(0.2, 0.6, 0.4)).ignore_safe_area(EdgeSet::NONE)),
        )));
        show_keyboard(&mount);
        let fill = find_view::<ColorView>(&mount.host).expect("the fill is mounted");
        expect_bottom(
            &fill,
            keyboard_top(&mount),
            "an empty ignore name-set extends nowhere",
        );
    }

    /// The fill's own `.ignore_safe_area` names the only edges it extends
    /// on: `BOTTOM` extends it to the window bottom — through both bands —
    /// while its other edges stay at the laid-out frame and the hosted
    /// content still stops at the keyboard region.
    fn a_fill_ignore_names_the_only_edge_that_extends() {
        let mount = mount(
            vstack((spacer(), text("content").body()))
                .background(Color::new(Srgb::new(0.9, 0.7, 0.1)).ignore_safe_area(EdgeSet::BOTTOM)),
        );
        show_keyboard(&mount);
        let children = children(&mount);
        let fill = &children[0];
        expect_bottom(
            fill,
            mount.window.bounds().size.height,
            "the named edge extends to the window edge",
        );
        expect_top(
            fill,
            container_top(&mount),
            "an edge the fill does not name stays at the container boundary",
        );
        let label = find_label(&mount.host, "content").expect("the label is mounted");
        expect_bottom(
            &label,
            keyboard_top(&mount),
            "the hosted content still stops at the keyboard",
        );
    }

    /// A color leaf in a plain slot — not a `.background` fill — is an
    /// ordinary leaf: its frame ends on the keyboard boundary without
    /// extending under it.
    fn non_fill_leaves_at_an_edge_do_not_extend() {
        let mount = mount(vstack((
            spacer(),
            Color::new(Srgb::new(0.7, 0.2, 0.8)).size(390.0, 4.0),
        )));
        show_keyboard(&mount);
        let strip = find_view::<ColorView>(&mount.host).expect("the color strip mounts");
        expect_bottom(
            &strip,
            keyboard_top(&mount),
            "a non-fill leaf does not extend",
        );
    }

    /// A background slot holding anything other than a fill — another
    /// view — extends nowhere by default; it is bound by the same regions.
    fn a_non_fill_background_stays_inside_the_keyboard_region() {
        let mount = mount(vstack((
            spacer(),
            text("content")
                .body()
                .padding_with(8.0)
                .background(text("panel")),
        )));
        show_keyboard(&mount);
        let background = view::subviews(&children(&mount)[1])
            .into_iter()
            .next()
            .expect("the background slot holds the panel leaf");
        expect_bottom(
            &background,
            keyboard_top(&mount),
            "a non-fill background stays inside the region",
        );
    }

    /// An ancestor's `TOP` release moves the hosted stack's top edge to
    /// the window; the fill it carries still extends through the bottom
    /// bands it touches — extension follows the laid-out frame, not the
    /// ancestor's declaration.
    fn a_fill_under_an_ancestor_top_ignore_still_extends_at_the_bottom() {
        let mount = mount(
            vstack((card("content"), spacer()))
                .ignore_safe_area(EdgeSet::TOP)
                .background(Color::new(Srgb::new(0.3, 0.5, 0.9))),
        );
        show_keyboard(&mount);
        let children = children(&mount);
        let (fill, content) = (&children[0], &children[1]);
        expect_top(content, 0.0, "the ancestor's named edge releases the top");
        let inner = view::subviews(&fanned(content));
        expect_bottom(
            &inner[1],
            keyboard_top(&mount),
            "the unnamed bottom still stops at the keyboard",
        );
        expect_bottom(
            fill,
            mount.window.bounds().size.height,
            "the fill extends on the touched bottom edge",
        );
    }

    /// A scroll surface extends under the touched edge and insets its
    /// content by the band; focusing a covered field scrolls the minimum
    /// distance that brings its frame clear of the keyboard region.
    fn a_scroll_surface_scrolls_the_focused_field_clear_of_the_keyboard() {
        let value = binding(Str::from(""));
        let mount = mount(scroll(vstack((
            spacer().size(390.0, 560.0),
            field("Message", &value).size(350.0, 44.0),
            spacer().size(390.0, 380.0),
        ))));
        show_keyboard(&mount);

        let surface = find_view::<ScrollView>(&mount.host).expect("scroll mounts a UIScrollView");
        let field = find_view::<TextField>(&mount.host).expect("the form mounts the field");
        let scroll_view = surface
            .downcast_ref::<ScrollView>()
            .expect("the found view is a scroll view");

        expect_bottom(
            &surface,
            mount.window.bounds().size.height,
            "the surface extends under the touched edge",
        );
        let cover = window_bottom(&surface) - keyboard_top(&mount);
        assert!(
            (scroll_view.contentInset().bottom
                - (cover - surface.safeAreaInsets().bottom).max(0.0))
            .abs()
                <= TOLERANCE,
            "the content inset carries the covered band minus the surface's own inset"
        );
        assert!(
            window_bottom(&field) > keyboard_top(&mount) + TOLERANCE,
            "precondition: the field starts under the keyboard region"
        );

        focus_and_clear(&mount, &field, || keyboard_top(&mount));
    }

    /// The keyboard region growing while a field holds focus re-runs the
    /// same minimum-scroll clearance against the deeper boundary — the
    /// field's frame lands on the new keyboard top on that frame.
    fn a_keyboard_inset_change_clears_the_field_on_that_frame() {
        let value = binding(Str::from(""));
        let mount = mount(scroll(vstack((
            spacer().size(390.0, 560.0),
            field("Message", &value).size(350.0, 44.0),
            spacer().size(390.0, 380.0),
        ))));
        show_keyboard(&mount);
        let field = find_view::<TextField>(&mount.host).expect("the form mounts the field");
        focus_and_clear(&mount, &field, || keyboard_top(&mount));

        let grown = keyboard_end_sized(&mount.window, 400.0);
        change_keyboard(&mount, grown);
        pump_until_bottom_on(&mount, &field, grown.origin.y);
    }

    /// A field nested inside a second scroll is cleared once — by the
    /// innermost surface that can, which is the outermost one here because
    /// the keyboard band sits below the inner surface's own frame.
    fn a_field_in_a_nested_scroll_is_cleared_once() {
        let value = binding(Str::from(""));
        let mount = mount(scroll(vstack((
            spacer().size(390.0, 430.0),
            scroll(vstack((
                spacer().size(350.0, 120.0),
                field("Nested", &value).size(350.0, 44.0),
                spacer().size(350.0, 300.0),
            )))
            .size(350.0, 200.0),
            spacer().size(390.0, 380.0),
        ))));
        show_keyboard(&mount);

        let surfaces = find_views::<ScrollView>(&mount.host);
        assert!(
            surfaces.len() == 2,
            "the form mounts an outer and an inner surface"
        );
        let (outer, inner) = (&surfaces[0], &surfaces[1]);
        let field = find_view::<TextField>(&mount.host).expect("the form mounts the field");
        assert!(
            window_bottom(&field) > keyboard_top(&mount) + TOLERANCE,
            "precondition: the nested field starts under the keyboard region"
        );

        focus_and_clear(&mount, &field, || keyboard_top(&mount));
        let outer_inset = outer
            .downcast_ref::<ScrollView>()
            .expect("a scroll view")
            .contentInset()
            .bottom;
        assert!(
            outer_inset > TOLERANCE,
            "the outermost surface carries the keyboard inset — {outer_inset}"
        );
        assert!(
            inner
                .downcast_ref::<ScrollView>()
                .expect("a scroll view")
                .contentInset()
                .bottom
                <= TOLERANCE,
            "the inner surface the band does not reach stays untouched"
        );
    }

    /// The clearance boundary is the nearer of the keyboard top and the
    /// surface's own frame: a toolbar card under the scroll keeps the
    /// field clearing to the surface's bottom edge, not the keyboard's.
    fn a_focused_field_clears_to_the_surface_frame_above_a_toolbar() {
        let value = binding(Str::from(""));
        let mount = mount(vstack((
            scroll(vstack((
                spacer().size(390.0, 560.0),
                field("Message", &value).size(350.0, 44.0),
                spacer().size(390.0, 380.0),
            ))),
            card("toolbar").size(390.0, 56.0),
        )));
        show_keyboard(&mount);
        let surface = find_view::<ScrollView>(&mount.host).expect("scroll mounts a UIScrollView");
        let field = find_view::<TextField>(&mount.host).expect("the form mounts the field");
        let surface_bottom = window_bottom(&surface);
        assert!(
            surface_bottom < keyboard_top(&mount) - TOLERANCE,
            "the toolbar holds the surface above the keyboard region"
        );
        focus_and_clear(&mount, &field, || surface_bottom);
    }

    /// A `List` is a scroll surface too: a focused row field scrolls
    /// clear of the keyboard and the `UITableView` carries the inset.
    fn a_list_row_field_scrolls_clear_of_the_keyboard() {
        let value = binding(Str::from(""));
        let field_row = 10usize;
        let mount = mount(List::for_each(
            (0..40).map(SelfId::new).collect::<Vec<_>>(),
            move |item| {
                if *item == field_row {
                    ListItem::new(AnyView::new(field("Notes", &value).size(350.0, 44.0)))
                } else {
                    ListItem::new(text(format!("row-{}", *item)))
                }
            },
        ));
        show_keyboard(&mount);

        let table = find_view::<TableView>(&mount.host).expect("the list mounts a UITableView");
        let field = find_view::<TextField>(&mount.host).expect("the row field is mounted");
        assert!(
            window_bottom(&field) > keyboard_top(&mount) + TOLERANCE,
            "precondition: the row field starts under the keyboard region"
        );

        focus_and_clear(&mount, &field, || keyboard_top(&mount));
        let cover = window_bottom(&table) - keyboard_top(&mount);
        assert!(
            (table
                .downcast_ref::<TableView>()
                .expect("a table view")
                .contentInset()
                .bottom
                - (cover - table.safeAreaInsets().bottom).max(0.0))
            .abs()
                <= TOLERANCE,
            "the table carries the keyboard inset",
        );
    }

    /// The tab bar stays docked clear of the container region only — the
    /// keyboard covers it instead of lifting it — while the hosted
    /// content's bottom edge binds to the deeper of the two boundaries.
    fn a_tab_bar_stays_docked_under_the_keyboard() {
        let selection = binding(0i32);
        let mount = mount(Tabs::new(
            &selection,
            vec![Tab::new(0i32, "Messages", move || {
                NavigationView::new(
                    "Messages",
                    vstack((
                        card("conversation"),
                        spacer(),
                        edge_probe(
                            card("edge"),
                            "bottom-edge",
                            SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM),
                        ),
                    )),
                )
            })],
        ));
        let tab_bar = find_view::<UITabBar>(&mount.host).expect("the tabs mount a UITabBar");
        pump_layout(&mount, || window_bottom(&tab_bar) > TOLERANCE);
        let docked = window_bottom(&tab_bar);
        assert!(
            (docked - container_bottom(&mount)).abs() <= TOLERANCE
                || (docked - mount.window.bounds().size.height).abs() <= TOLERANCE,
            "the bar docks at the container boundary — bottom {docked}"
        );

        show_keyboard(&mount);
        pump_layout(&mount, || {
            (window_bottom(&tab_bar) - docked).abs() <= TOLERANCE
        });
        assert!(
            window_top(&tab_bar) > keyboard_top(&mount),
            "the keyboard covers the docked bar rather than lifting it"
        );
        let probe = find_id(&mount.host, "bottom-edge").expect("the probe is mounted");
        pump_until_bottom_on(&mount, &probe, keyboard_top(&mount));
    }

    /// With the navigation bar hidden the page's top edge passes straight
    /// through to the window — a released `TOP` reaches the window origin
    /// — and the bottom content still binds the keyboard.
    fn a_navigation_page_without_a_bar_passes_the_edge_through() {
        let mount = mount(
            NavigationView::new(
                "Home",
                vstack((
                    edge_probe(card("hero"), "top-edge", EdgeSet::TOP),
                    card("content"),
                    spacer(),
                    text("tail").body(),
                ))
                .background(Color::new(Srgb::new(0.2, 0.4, 0.7))),
            )
            .navigation_bar_visibility(false),
        );
        show_keyboard(&mount);
        let probe = find_id(&mount.host, "top-edge").expect("the probe is mounted");
        expect_top(
            &probe,
            mount.window.bounds().origin.y,
            "a pass-through edge releases to the window top",
        );
        let tail = find_label(&mount.host, "tail").expect("the tail is mounted");
        expect_bottom(
            &tail,
            keyboard_top(&mount),
            "the page's bottom still stops at the keyboard",
        );
    }

    /// A navigation page whose laid-out frame touches the bottom edge
    /// still extends its fill through it — the fill reaches the window
    /// bottom under the keyboard while the hosted content stops at the
    /// boundary.
    fn a_navigation_page_touching_the_bottom_still_extends_its_fill() {
        let mount = mount(NavigationView::new(
            "Home",
            vstack((text("content").body(), spacer(), text("tail").body()))
                .background(Color::new(Srgb::new(0.5, 0.3, 0.7))),
        ));
        show_keyboard(&mount);
        let tail = find_label(&mount.host, "tail").expect("the tail is mounted");
        expect_bottom(
            &tail,
            keyboard_top(&mount),
            "the hosted page still stops at the keyboard",
        );
        let fill = find_view::<ColorView>(&mount.host).expect("the page mounts its fill");
        expect_bottom(
            &fill,
            mount.window.bounds().size.height,
            "the page fill extends through the touched bottom edge",
        );
    }

    /// The navigation bar covers the top edge for the page's content: a
    /// `TOP` release inside reaches no deeper than the band the bar
    /// occupies — the probe stops at the bar's bottom, not the origin.
    fn an_ignore_top_inside_navigation_content_stays_below_the_bar() {
        let mount = mount(NavigationView::new(
            "Home",
            vstack((edge_probe(card("hero"), "top-edge", EdgeSet::TOP), spacer()))
                .background(Color::new(Srgb::new(0.2, 0.4, 0.7))),
        ));
        let bar = find_view::<UINavigationBar>(&mount.host)
            .expect("the page mounts a nav bar")
            .downcast::<UINavigationBar>()
            .expect("the found view is a navigation bar");
        assert_eq!(
            bar.barPosition(),
            UIBarPosition::TopAttached,
            "the bar keeps the top-attached position that extends its \
             background over the status-bar band",
        );
        let background = view::subviews(&bar).into_iter().find(|sub| {
            let frame = window_frame(sub);
            frame.origin.y <= TOLERANCE
                && (frame.size.width - window_frame(&bar).size.width).abs() <= TOLERANCE
        });
        assert!(
            background.is_some(),
            "the bar's background reaches the window's top edge spanning its width",
        );
        let bar_bottom = window_bottom(&bar);
        assert!(
            bar_bottom > TOLERANCE,
            "precondition: the bar covers the top edge"
        );
        let probe = find_id(&mount.host, "top-edge").expect("the probe is mounted");
        expect_top(
            &probe,
            bar_bottom,
            "a covered edge releases nothing past the covering band",
        );
    }

    /// The page's own background starts at its laid-out frame — below the
    /// bar the host draws — and still extends through the bottom edge the
    /// frame touches.
    fn a_navigation_page_background_stays_below_the_bar() {
        let mount = mount(NavigationView::new(
            "Home",
            vstack((text("content").body(), spacer(), text("tail").body()))
                .background(Color::new(Srgb::new(0.5, 0.3, 0.7))),
        ));
        let bar = find_view::<UINavigationBar>(&mount.host).expect("the page mounts a nav bar");
        let fill = find_view::<ColorView>(&mount.host).expect("the page mounts its fill");
        expect_top(
            &fill,
            window_bottom(&bar),
            "the page background starts below the covering bar",
        );
        show_keyboard(&mount);
        let tail = find_label(&mount.host, "tail").expect("the tail is mounted");
        expect_bottom(
            &tail,
            keyboard_top(&mount),
            "the hosted page still stops at the keyboard",
        );
        expect_bottom(
            &fill,
            mount.window.bounds().size.height,
            "the fill extends through the touched bottom edge",
        );
    }

    /// A bottom toolbar docks clear of the container region only: the
    /// keyboard covers it rather than lifting it, and the hosted
    /// content's released bottom reaches the deeper of the two
    /// boundaries — the keyboard top.
    fn a_navigation_bottom_toolbar_stays_docked_under_the_keyboard() {
        let mount = mount(NavigationStack::new(
            NavigationView::new(
                "Home",
                vstack((
                    card("content"),
                    spacer(),
                    edge_probe(
                        card("edge"),
                        "bottom-edge",
                        SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM),
                    ),
                ))
                .background(Color::new(Srgb::new(0.2, 0.4, 0.7))),
            )
            .navigation_toolbar(NavigationToolbar::new(vec![
                NavigationToolbarItem::action(NavigationToolbarPlacement::BottomBar, "New", || {}),
            ])),
        ));
        // iOS 26 draws the page's bottom bar as a floating glass capsule,
        // not a `UIToolbar` — the item's own control is the bar's proof.
        let item =
            find_class(&mount.host, c"CocoaUiBarButton").expect("the page mounts its bar item");
        let docked = window_frame(&item);
        assert!(
            docked.origin.y + docked.size.height > container_bottom(&mount) - 80.0,
            "the bar item docks at the bottom edge — frame {docked:?}"
        );

        show_keyboard(&mount);
        pump_layout(&mount, || keyboard_top(&mount) > TOLERANCE);
        let covered = window_frame(&item);
        assert!(
            (covered.origin.y - docked.origin.y).abs() <= TOLERANCE,
            "the keyboard covers the docked bar item rather than lifting it — {docked:?} -> {covered:?}"
        );
        assert!(
            covered.origin.y > keyboard_top(&mount),
            "the bar item stays inside the keyboard band — {covered:?}"
        );
        let probe = find_id(&mount.host, "bottom-edge").expect("the probe is mounted");
        pump_until_bottom_on(&mount, &probe, keyboard_top(&mount));
    }

    /// A page toolbar inside a tab stacks on the docked tab bar: the
    /// toolbar sits on the tab bar's top, the tab bar on the container
    /// boundary, and the content binds the deeper boundary under the
    /// keyboard.
    fn a_stack_page_toolbar_inside_a_tab_stacks_on_the_docked_tab_bar() {
        let selection = binding(0i32);
        let mount = mount(Tabs::new(
            &selection,
            vec![Tab::container(0i32, "Home", move || {
                NavigationStack::new(
                    NavigationView::new(
                        "Home",
                        vstack((
                            card("content"),
                            spacer(),
                            edge_probe(
                                card("edge"),
                                "bottom-edge",
                                SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM),
                            ),
                        ))
                        .background(Color::new(Srgb::new(0.2, 0.4, 0.7))),
                    )
                    .navigation_toolbar(NavigationToolbar::new(vec![
                        NavigationToolbarItem::action(
                            NavigationToolbarPlacement::BottomBar,
                            "New",
                            || {},
                        ),
                    ])),
                )
            })],
        ));
        let tab_bar = find_view::<UITabBar>(&mount.host).expect("the tabs mount a UITabBar");
        pump_layout(&mount, || window_bottom(&tab_bar) > TOLERANCE);
        // iOS 26 draws the page's bottom bar as a floating glass capsule,
        // not a `UIToolbar` — the item's own control is the bar's proof.
        let item =
            find_class(&mount.host, c"CocoaUiBarButton").expect("the page mounts its bar item");
        let docked = window_frame(&item);
        assert!(
            docked.origin.y + docked.size.height > container_bottom(&mount) - 80.0,
            "the bar item docks at the bottom edge — frame {docked:?}"
        );

        show_keyboard(&mount);
        pump_layout(&mount, || keyboard_top(&mount) > TOLERANCE);
        let covered = window_frame(&item);
        assert!(
            (covered.origin.y - docked.origin.y).abs() <= TOLERANCE,
            "the keyboard covers the docked bar item rather than lifting it — {docked:?} -> {covered:?}"
        );
        assert!(
            window_top(&tab_bar) > keyboard_top(&mount),
            "the keyboard covers the stacked bars rather than lifting them"
        );
        let probe = find_id(&mount.host, "bottom-edge").expect("the probe is mounted");
        pump_until_bottom_on(&mount, &probe, keyboard_top(&mount));
    }

    /// A fixed-height `Tabs` inside a page docks its own bar at its own
    /// frame — a bar docks at the boundary of the edge it touches, which
    /// for a mid-window bar is the frame its host gave it.
    fn a_fixed_height_tabs_inside_tab_content_docks_its_own_bar() {
        let selection = binding(0i32);
        let inner_selection = binding(0i32);
        let mount = mount(Tabs::new(
            &selection,
            vec![
                Tab::new(0i32, "Outer", move || {
                    NavigationView::new(
                        "Outer",
                        vstack((
                            Tabs::new(
                                &inner_selection,
                                vec![
                                    Tab::new(1i32, "Inner A", move || {
                                        NavigationView::new("A", text("a"))
                                    }),
                                    Tab::new(2i32, "Inner B", move || {
                                        NavigationView::new("B", text("b"))
                                    }),
                                ],
                            )
                            .size(390.0, 200.0),
                            card("below").a11y_id("below-inner"),
                            spacer(),
                        )),
                    )
                }),
                Tab::new(3i32, "Other", move || {
                    NavigationView::new("Other", text("other"))
                }),
            ],
        ));
        // Two bars: the inner controller's inside its 200pt host and the
        // outer one docked at the window bottom — order them by where
        // they landed rather than the tree walk's order.
        let mut bars = find_views::<UITabBar>(&mount.host);
        bars.sort_by(|a, b| {
            window_top(a)
                .partial_cmp(&window_top(b))
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        assert!(bars.len() == 2, "the two tab controllers mount two bars");
        let (inner_bar, outer_bar) = (bars[0].clone(), bars[1].clone());
        let below = find_id(&mount.host, "below-inner").expect("the sibling card mounts");
        assert!(
            window_bottom(&inner_bar) <= window_top(&below) + TOLERANCE,
            "a nested bar that does not reach the dock stays inside its \
             own frame, above the next sibling — inner {:?}, sibling {:?}",
            window_frame(&inner_bar),
            window_frame(&below),
        );
        assert!(
            (window_bottom(&outer_bar) - container_bottom(&mount)).abs() <= TOLERANCE
                || (window_bottom(&outer_bar) - mount.window.bounds().size.height).abs()
                    <= TOLERANCE,
            "the outer bar docks on the container boundary — {:?}",
            window_frame(&outer_bar),
        );

        show_keyboard(&mount);
        pump_layout(&mount, || keyboard_top(&mount) > TOLERANCE);
        assert!(
            window_bottom(&inner_bar) <= window_top(&below) + TOLERANCE,
            "keyboard up: the nested bar still stays inside its own frame"
        );
        assert!(
            (window_bottom(&outer_bar) - container_bottom(&mount)).abs() <= TOLERANCE
                || (window_bottom(&outer_bar) - mount.window.bounds().size.height).abs()
                    <= TOLERANCE,
            "keyboard up: the outer bar stays docked on the boundary — {:?}",
            window_frame(&outer_bar),
        );
    }

    /// A form hosted inside navigation content clears its focused field
    /// to the deeper boundary under the keyboard — chrome hosting does not
    /// change the scroll surface's contract.
    fn a_form_inside_navigation_content_clears_the_focused_field() {
        let value = binding(Str::from(""));
        let mount = mount(NavigationView::new(
            "Form",
            scroll(vstack((
                spacer().size(390.0, 560.0),
                field("Message", &value).size(350.0, 44.0),
                spacer().size(390.0, 380.0),
            ))),
        ));
        show_keyboard(&mount);
        let field = find_view::<TextField>(&mount.host).expect("the form mounts the field");
        assert!(
            window_bottom(&field) > keyboard_top(&mount) + TOLERANCE,
            "precondition: the field starts under the keyboard region"
        );
        focus_and_clear(&mount, &field, || keyboard_top(&mount));
    }
}
/// `.size`/`padding`/`hstack`/`zstack` containers plus a lazy `ForEach`
/// membership — must release its owners, child guards and host views when
/// the mounted tree drops. Weak handles into the tree's views die after
/// queued work drains, post-owner writes cease naturally, and a remount
/// of the same view still measures, binds and recycles membership.
mod owner_lifetimes {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use waterui::Identifiable;
    use waterui::component::lazy::Lazy;
    use waterui::prelude::*;
    use waterui::reactive::binding;
    use waterui::reactive::collection::List as ReactiveList;
    use waterui::views::ForEach;
    use waterui_core::layout::ProposalSize;

    use super::{HostView, Label, MainThreadMarker, PlatformView, Retained, leaf, mtm, resolve};

    /// A stable-id row for the `ForEach` membership.
    #[derive(Clone, Copy, Identifiable)]
    struct Row {
        #[id]
        id: u64,
    }

    /// Drains queued main-queue work — binding flushes and enqueued drops
    /// alike land at a real run-loop boundary. A sentinel block enqueued
    /// behind everything already queued bounds the wait: the main queue is
    /// FIFO, so the sentinel running means the work ahead of it ran.
    fn pump() {
        assert!(
            waterui_apple::native_test_support::drain_main_queue(mtm()),
            "the main queue must drain inside its deadline"
        );
    }

    /// Every `Label` payload in the subtree, depth-first — the observable
    /// end of the binding path on both kits.
    fn label_texts(view: &PlatformView, out: &mut Vec<String>) {
        for sub in cocoa_ui::view::subviews(view) {
            if let Some(label) = sub.downcast_ref::<Label>()
                && let Some(attributed) = label.source_text()
            {
                out.push(attributed.string().to_string());
            }
            label_texts(&sub, out);
        }
    }

    /// A weak handle for every view in the mounted subtree. `load`
    /// answering `None` is the observable proof the owner that retained
    /// the view is gone.
    fn weak_views(view: &PlatformView, out: &mut Vec<objc2::rc::Weak<PlatformView>>) {
        out.push(objc2::rc::Weak::new(&cocoa_ui::view::retain_base(view)));
        for sub in cocoa_ui::view::subviews(view) {
            weak_views(&sub, out);
        }
    }

    /// The hierarchy the repairs cover: `.size`/`padding` wrappers around
    /// `hstack`/`zstack` containers plus a lazy `ForEach` membership — one
    /// tree holding every repaired owner class.
    fn hierarchy(
        track: &waterui::reactive::Binding<String>,
        items: &ReactiveList<Row>,
    ) -> impl View {
        let track = track.clone();
        let items = items.clone();
        zstack((
            hstack((text!("{track}"), spacer()))
                .padding()
                .size(320.0, 60.0),
            Lazy::vstack(ForEach::new(items, |row: Row| {
                text(format!("row {}", row.id))
            })),
        ))
    }

    #[cfg(target_os = "macos")]
    use cocoa_ui::objc2_app_kit::{NSColor as PlatformColor, NSForegroundColorAttributeName};
    use cocoa_ui::objc2_core_graphics::CGColor;
    use cocoa_ui::objc2_foundation::NSAttributedString;
    #[cfg(target_os = "ios")]
    use cocoa_ui::objc2_ui_kit::{NSForegroundColorAttributeName, UIColor as PlatformColor};
    use waterui::graphics::color::WorkingColor;

    /// The attributed text's native foreground attribute read as its
    /// four extended-linear-P3 channels — the semantic value the
    /// default-foreground watcher writes, compared against the working
    /// color it was given rather than a rebuild pointer.
    fn assert_foreground(attributed: &NSAttributedString, expected: WorkingColor) {
        // SAFETY: a null effective-range pointer is permitted.
        let value = unsafe {
            attributed.attribute_atIndex_effectiveRange(
                NSForegroundColorAttributeName,
                0,
                std::ptr::null_mut(),
            )
        }
        .expect("the track chunk carries a foreground attribute");
        let color = value
            .downcast::<PlatformColor>()
            .expect("the foreground attribute is a platform color");
        #[cfg(target_os = "macos")]
        let cg = color.CGColor();
        #[cfg(target_os = "ios")]
        // SAFETY: the retained UIKit color is read on the actual main thread.
        let cg = unsafe { color.CGColor() };
        assert_eq!(CGColor::number_of_components(Some(&cg)), 4);
        // SAFETY: the color owns the four components asserted above.
        let actual = unsafe { std::slice::from_raw_parts(CGColor::components(Some(&cg)), 4) };
        for (actual, expected) in actual.iter().zip(expected.components) {
            assert!(
                (actual - f64::from(expected)).abs() < 1e-4,
                "native foreground channel {actual} differs from working channel {expected}"
            );
        }
    }

    /// The first `Label` inside the subtree — the observable end of the
    /// per-chunk signal path this fix touches.
    fn first_label(view: &PlatformView) -> Option<Retained<Label>> {
        for sub in cocoa_ui::view::subviews(view) {
            if let Some(label) = sub.downcast_ref::<Label>() {
                return Some(label.into());
            }
            if let Some(label) = first_label(&sub) {
                return Some(label);
            }
        }
        None
    }

    /// The class names of every subtree view that still lives — the
    /// observable list of real retainers a drop failed to release.
    fn surviving_classes(weaks: &[objc2::rc::Weak<PlatformView>]) -> Vec<String> {
        weaks
            .iter()
            .filter_map(|weak| {
                weak.load()
                    .map(|view| view.class().name().to_string_lossy().into_owned())
            })
            .collect()
    }

    /// Mounts `leaf` on a fresh host whose layout handler frames it.
    /// The host is a bare `HostView` that no leaf owns, so nothing clears
    /// its handlers; the handler borrows the child weakly so dropping the
    /// `Mounted` alone decides whether the child's views survive.
    fn mount_hosted(
        mtm: MainThreadMarker,
        leaf_inst: waterui_apple::contract::NativeLeaf,
    ) -> (Retained<HostView>, waterui_apple::contract::Mounted) {
        let parent = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mounted = leaf_inst.mount(&parent);
        let child_view: objc2::rc::Weak<PlatformView> =
            objc2::rc::Weak::new(&cocoa_ui::view::retain_base(mounted.view()));
        parent.set_layout_handler(move |host| {
            let host_view: &PlatformView = host;
            if let Some(child) = child_view.load() {
                cocoa_ui::view::set_frame(&child, cocoa_ui::view::bounds(host_view));
            }
        });
        (parent, mounted)
    }

    /// One consolidated regression for #1575: mount the repaired owner
    /// classes in a single tree against ONE explicit environment, prove
    /// live updates still land, prove the drop releases the whole
    /// subtree, then prove a remount on the same env keeps working. The
    /// mount, the submitted work, the drop and the queued drain all live
    /// inside bounded `autoreleasepool`s — only `Weak` handles escape
    /// them, so a surviving read outside proves a real retainer, never a
    /// pooled temporary (mirrors the ownership fixture's idiom).
    pub fn a_dropped_mounted_hierarchy_releases_views_and_remounts() {
        let mtm = mtm();
        // The environment, binding, membership and per-chunk signal all
        // live outside the pools — a weak read that still answers `Some`
        // afterwards names a real retainer, not a pooled autorelease.
        let mut env = resolve::env();
        let track = binding(String::from("first"));
        let items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }]);
        // The theme `Foreground` slot backs every unstyled chunk — the
        // `label_leaf` default watcher this fix touches. Reinstall it
        // from a binding the test keeps, so a live update is reachable.
        let foreground = binding(waterui::graphics::color::WorkingColor::BLACK);
        waterui::theme::install_color_signal::<waterui::theme::color::Foreground>(
            &mut env,
            foreground.computed(),
        );

        let weaks = objc2::rc::autoreleasepool(|_| {
            let leaf_inst = waterui_apple::dispatch::render(
                waterui_backend_core::AnyView::new(hierarchy(&track, &items)),
                &env,
            );
            let (parent, mounted) = mount_hosted(mtm, leaf_inst);
            let _window = leaf::attach(mtm, &parent);
            parent.set_needs_layout();
            parent.layout_if_needed();
            pump();
            parent.layout_if_needed();

            let mut weaks = Vec::new();
            weak_views(mounted.view(), &mut weaks);
            assert!(weaks.len() > 3, "the fixture must mount a real hierarchy");
            assert!(weaks.iter().all(|w| w.load().is_some()));

            // Submitted work while alive: the binding write lands on the
            // mounted text and the pushed member mounts through membership.
            track.set(String::from("second"));
            items.push(Row { id: 3 });
            parent.layout_if_needed();
            pump();
            parent.layout_if_needed();
            let mut texts = Vec::new();
            label_texts(mounted.view(), &mut texts);
            assert!(texts.iter().any(|t| t.contains("second")));
            assert!(texts.iter().any(|t| t.contains("row 3")));

            // The default-foreground signal path still updates while
            // live: a new theme foreground lands in the label's
            // attributed text as the native foreground attribute — the
            // `Weak`-borrowing watcher answers instead of being frozen.
            let label = first_label(mounted.view()).expect("the track label mounted");
            let before = label.source_text().expect("attributed text");
            assert_foreground(&before, WorkingColor::BLACK);
            foreground.set(WorkingColor::WHITE);
            pump();
            let after = label.source_text().expect("attributed text");
            assert_foreground(&after, WorkingColor::WHITE);

            drop(mounted);
            pump();
            weaks
        });

        let survivors = surviving_classes(&weaks);
        assert!(
            survivors.is_empty(),
            "mounted child views survived the owner drop: {survivors:?}"
        );

        // Post-owner callbacks cease naturally: a later write reaches no
        // dead owner and cannot panic through a cleared weak edge.
        objc2::rc::autoreleasepool(|_| {
            track.set(String::from("third"));
            foreground.set(waterui::graphics::color::WorkingColor::BLACK);
            pump();
        });

        // Remount on the same env: measurement, binding and membership
        // all still work, and the remounted tree releases too.
        let weaks = objc2::rc::autoreleasepool(|_| {
            let leaf_inst = waterui_apple::dispatch::render(
                waterui_backend_core::AnyView::new(hierarchy(&track, &items)),
                &env,
            );
            assert!(
                leaf_inst
                    .layout()
                    .measure(ProposalSize::new(Some(400.0), Some(200.0)))
                    .size
                    .width
                    > 0.0,
                "the remounted hierarchy still answers measure"
            );
            let (parent, mounted) = mount_hosted(mtm, leaf_inst);
            let _window = leaf::attach(mtm, &parent);
            parent.set_needs_layout();
            parent.layout_if_needed();
            track.set(String::from("fourth"));
            items.push(Row { id: 4 });
            pump();
            parent.layout_if_needed();
            pump();
            let mut texts = Vec::new();
            label_texts(mounted.view(), &mut texts);
            assert!(texts.iter().any(|t| t.contains("fourth")));
            assert!(texts.iter().any(|t| t.contains("row 4")));
            let mut weaks = Vec::new();
            weak_views(mounted.view(), &mut weaks);
            drop(mounted);
            pump();
            weaks
        });
        let survivors = surviving_classes(&weaks);
        assert!(
            survivors.is_empty(),
            "the remounted hierarchy must release the same way: {survivors:?}"
        );
    }

    /// The views a scroll surface hosts as mounted content — the document's
    /// subviews on `AppKit`; the `HostView`-topped subviews on `UIKit`.
    /// The kit's own chrome (clip/document views, scroll indicators) is
    /// the scroll view's own property and legitimately survives with it.
    #[cfg(target_os = "macos")]
    fn scroll_content_subviews(scroll: &PlatformView) -> Vec<Retained<PlatformView>> {
        scroll
            .downcast_ref::<cocoa_ui::appkit::ScrollView>()
            .and_then(cocoa_ui::appkit::ScrollView::document_view)
            .map(|document| cocoa_ui::view::subviews(&document))
            .unwrap_or_default()
    }

    /// The `UIKit` scroll surface mounts the child on the scroll view
    /// itself; scroll indicators are not `HostView`s.
    #[cfg(target_os = "ios")]
    fn scroll_content_subviews(scroll: &PlatformView) -> Vec<Retained<PlatformView>> {
        cocoa_ui::view::subviews(scroll)
            .into_iter()
            .filter(|sub| sub.downcast_ref::<HostView>().is_some())
            .collect()
    }

    /// The trials this module registers — the same submodule-registry
    /// shape `migration`/`tabs`/`controller_bounds` use.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "owner_lifetimes::a_dropped_mounted_hierarchy_releases_views_and_remounts",
                || {
                    a_dropped_mounted_hierarchy_releases_views_and_remounts();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "owner_lifetimes::a_released_scroll_leaf_frees_its_content",
                || {
                    a_released_scroll_leaf_frees_its_content();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "owner_lifetimes::a_scroll_leaf_dropped_inside_its_own_layout_handler_does_not_abort",
                || {
                    a_scroll_leaf_dropped_inside_its_own_layout_handler_does_not_abort();
                    Ok(())
                },
            ),
        ]
    }

    /// The leaf's scroll surface as the kit type the handler API lives on,
    /// retained so the borrow the `Mounted` would tie up is released.
    #[cfg(target_os = "macos")]
    fn kit_scroll_view(view: &PlatformView) -> Retained<cocoa_ui::appkit::ScrollView> {
        cocoa_ui::view::retain_base(view)
            .downcast::<cocoa_ui::appkit::ScrollView>()
            .expect("a scroll leaf's view is the kit ScrollView")
    }
    /// The leaf's scroll surface as the kit type the handler API lives on,
    /// retained so the borrow the `Mounted` would tie up is released.
    #[cfg(target_os = "ios")]
    fn kit_scroll_view(view: &PlatformView) -> Retained<cocoa_ui::uikit::ScrollView> {
        cocoa_ui::view::retain_base(view)
            .downcast::<cocoa_ui::uikit::ScrollView>()
            .expect("a scroll leaf's view is the kit ScrollView")
    }

    /// #1908: a released scroll leaf frees its content. The scroll view's
    /// handler slots hold `ScrollContent` — which owns the mounted child
    /// leaf — so while the leaf lives, the scroll view retains the whole
    /// content subtree through its handlers. The `HandlerTeardown` guard
    /// in the leaf's keepalive clears the slots at the leaf's release
    /// boundary, the same teardown mounted `HostView` handlers get since
    /// #1860. Retaining the scroll view past the drop proves the slots,
    /// not the view's deallocation, do the release: everything the
    /// handlers pinned dies even though the view they belong to lives.
    pub fn a_released_scroll_leaf_frees_its_content() {
        let mtm = mtm();
        let env = resolve::env();

        let (_scroll_view, content_weaks) = objc2::rc::autoreleasepool(|_| {
            let leaf_inst = waterui_apple::dispatch::render(
                waterui_backend_core::AnyView::new(scroll(vstack((text!("top"), text!("bottom"))))),
                &env,
            );
            let (parent, mounted) = mount_hosted(mtm, leaf_inst);
            let _window = leaf::attach(mtm, &parent);
            parent.set_needs_layout();
            parent.layout_if_needed();

            // While alive the layout handler framed the mounted content.
            let content = scroll_content_subviews(mounted.view());
            assert_eq!(content.len(), 1, "the scroll mounts one child view");
            let frame = cocoa_ui::view::frame(&content[0]);
            assert!(
                frame.size.width > 0.0 && frame.size.height > 0.0,
                "the layout handler must frame the mounted content while alive"
            );

            // Retain the scroll view itself past the leaf's drop, then take
            // weak handles into the mounted content subtree.
            let scroll_view = cocoa_ui::view::retain_base(mounted.view());
            let mut content_weaks = Vec::new();
            for sub in content {
                weak_views(&sub, &mut content_weaks);
            }
            assert!(
                content_weaks.iter().all(|weak| weak.load().is_some()),
                "the mounted content must be alive before the drop"
            );

            drop(mounted);
            (scroll_view, content_weaks)
        });

        // The scroll view lives — this test retains it — but every view
        // its handlers kept in the content subtree died with the leaf.
        let survivors = surviving_classes(&content_weaks);
        assert!(
            survivors.is_empty(),
            "scroll content views survived the leaf drop: {survivors:?}"
        );
    }

    /// A `clear_handlers` reached while its own handler is running — the
    /// re-entrant teardown `callback::emit`'s borrow release makes legal.
    /// The layout handler takes the mounted leaf out of a cell and drops
    /// it: the leaf's `HandlerTeardown` guard clears this same slot
    /// mid-callback. Borrowing the slot across the call would hit a live
    /// borrow here and `guarded` would abort the process; the surviving
    /// asserts are the proof it did not.
    pub fn a_scroll_leaf_dropped_inside_its_own_layout_handler_does_not_abort() {
        let mtm = mtm();
        let env = resolve::env();

        objc2::rc::autoreleasepool(|_| {
            let leaf_inst = waterui_apple::dispatch::render(
                waterui_backend_core::AnyView::new(scroll(text!("body"))),
                &env,
            );
            let (parent, mounted) = mount_hosted(mtm, leaf_inst);
            let _window = leaf::attach(mtm, &parent);

            let scroll_view = kit_scroll_view(mounted.view());
            // The leaf is the handler's to drop, in a cell it can empty
            // on its first run.
            let leaf_cell = Rc::new(RefCell::new(Some(mounted)));
            let runs = Rc::new(Cell::new(0u32));
            scroll_view.set_layout_handler({
                let leaf_cell = Rc::clone(&leaf_cell);
                let runs = Rc::clone(&runs);
                move |_| {
                    runs.set(runs.get() + 1);
                    drop(leaf_cell.borrow_mut().take());
                }
            });

            scroll_view.set_needs_layout();
            scroll_view.layout_if_needed();
            assert!(runs.get() >= 1, "the layout handler must run");
            assert!(
                leaf_cell.borrow().is_none(),
                "the handler must have dropped the leaf"
            );

            // The clear the drop performed is observable: a later layout
            // pass reaches an empty slot and runs nothing more.
            let observed = runs.get();
            scroll_view.set_needs_layout();
            scroll_view.layout_if_needed();
            assert_eq!(runs.get(), observed, "a cleared slot must not run again");
        });
    }
}

/// `ScrollController` requests against the kit's real scroll surfaces —
/// the water-rs/waterui#1901 contract: a bare request jumps to its
/// target, `Animation::Default` plays the platform's smooth scroll, an
/// explicit animation moves the offset along its timing, and every
/// flight lands where the jump would have. A request issued mid-flight
/// takes over.
mod scroll {
    use std::time::Duration;

    use waterui::animation::Animation;
    use waterui::layout::scroll::ScrollController;
    use waterui::reactive::{Binding, Signal, binding};
    use waterui_apple::contract::NativeLeaf;
    use waterui_core::layout::Point;

    use super::{
        AttachedWindow, Retained, animation_cases, assert_offset_eq, distinct_offsets,
        mount_and_order_front, pump_flight, trial_each,
    };

    use waterui_apple::native_test_support::{ScrollSurface, assert_native_scroll, scroll_surface};

    /// A scroll surface mounted in the harness window with a 2000pt
    /// document — the controller and report binding the suite drives,
    /// the leaf that owns the watchers, and the window that gives layout
    /// a real home, all kept alive for the case.
    struct Fixture {
        surface: Retained<ScrollSurface>,
        controller: ScrollController<Point>,
        offset: Binding<Point>,
        _leaf: NativeLeaf,
        _window: AttachedWindow,
    }

    /// How far the document can travel — extent minus viewport, read
    /// through each kit's own vocabulary.
    fn vertical_travel(surface: &ScrollSurface) -> f64 {
        #[cfg(target_os = "macos")]
        {
            surface
                .document_view()
                .expect("a laid-out scroll surface has a document")
                .frame()
                .size
                .height
                - surface.viewport_size().height
        }
        #[cfg(target_os = "ios")]
        {
            surface.content_extent().height - surface.viewport_size().height
        }
    }

    /// The offset the surface reports right now — the value
    /// `report_offset` publishes. Every animated write moves the model
    /// (the flight writes it per tick; the platform's animated scroll
    /// writes it per frame), so the binding tracks the flight.
    fn reported_y(fixture: &Fixture) -> f64 {
        f64::from(fixture.offset.snapshot().y)
    }

    /// The raw `contentOffset` a logical `target_y` lands on — `AppKit`'s
    /// clip bounds take the request verbatim.
    #[cfg(target_os = "macos")]
    const fn raw_target_y(_fixture: &Fixture, target_y: f64) -> f64 {
        target_y
    }

    /// The raw `contentOffset` a logical `target_y` lands on — `UIKit`
    /// subtracts `adjustedContentInset.top` from the request. Asserting
    /// the raw offset verifies that inset mapping end to end.
    #[cfg(target_os = "ios")]
    fn raw_target_y(fixture: &Fixture, target_y: f64) -> f64 {
        target_y - fixture.surface.adjusted_content_inset().top
    }

    /// The largest offset the document admits — the clip's constrained
    /// end on `AppKit`, the inset-adjusted end on `UIKit`.
    fn end_offset(fixture: &Fixture) -> f64 {
        #[cfg(target_os = "macos")]
        {
            let clip = fixture.surface.clip_view();
            clip.constrainBoundsRect(cocoa_ui::objc2_foundation::NSRect::new(
                cocoa_ui::objc2_foundation::NSPoint::new(0.0, 1.0e6),
                clip.bounds().size,
            ))
            .origin
            .y
        }
        #[cfg(target_os = "ios")]
        {
            let inset = fixture.surface.adjusted_content_inset();
            let minimum = -inset.top;
            (fixture.surface.content_extent().height - fixture.surface.viewport_size().height
                + inset.bottom)
                .max(minimum)
        }
    }

    /// Mounts a vertical scroll surface driven by a fresh controller.
    fn mounted() -> Fixture {
        let controller = ScrollController::new(Point::new(0.0, 0.0));
        let offset = binding(Point::new(0.0, 0.0));
        let (leaf, surface) = scroll_surface(&controller, &offset);
        let window = mount_and_order_front(leaf.view());
        surface.set_needs_layout();
        surface.layout_if_needed();
        #[cfg(target_os = "macos")]
        super::appkit_input::present_first_frame(super::mtm(), leaf.view());
        let travel = vertical_travel(&surface);
        assert!(
            travel > 1100.0,
            "the 2000pt document must out-travel a 1000pt target: {travel}"
        );
        Fixture {
            surface,
            controller,
            offset,
            _leaf: leaf,
            _window: window,
        }
    }

    /// No animation on the request: the offset is the target before any
    /// pumping — the jump it always was.
    fn a_bare_request_jumps_to_the_target() {
        let fixture = mounted();
        fixture.controller.scroll_to(Point::new(0.0, 800.0));
        assert_offset_eq(
            fixture.surface.content_offset().y,
            raw_target_y(&fixture, 800.0),
            "a jump lands immediately",
        );
        assert_offset_eq(
            reported_y(&fixture),
            800.0,
            "the reported offset is the logical target",
        );
    }

    /// An animated request ends on the target a jump would have taken.
    /// An explicit curve moves the reported offset through intermediate
    /// values — every clocked write lands on the model, so
    /// `report_offset` publishes the ramp. `Default` is the platform's
    /// wall-clock animation: it does not jump, and it takes time to land.
    fn an_animation_scrolls_to_the_target(name: &'static str, animation: Animation) {
        let fixture = mounted();
        let native = matches!(animation, Animation::Default);
        let request = || {
            fixture
                .controller
                .animate_to(Point::new(0.0, 800.0), animation);
            assert!(
                fixture.surface.scroll_animation_in_flight(),
                "the {name} flight must be in progress after the request"
            );
        };
        if native {
            assert_native_scroll(
                &format!("the {name} flight"),
                request,
                || fixture.surface.content_offset(),
                || cocoa_ui::Point::new(0.0, raw_target_y(&fixture, 800.0)),
            );
            pump_flight(
                || fixture.surface.scroll_animation_in_flight(),
                || reported_y(&fixture),
            );
        } else {
            request();
            let samples = pump_flight(
                || fixture.surface.scroll_animation_in_flight(),
                || reported_y(&fixture),
            );
            assert!(
                distinct_offsets(&samples) >= 3,
                "the {name} flight must move the reported offset through intermediate values: {samples:?}"
            );
        }
        assert_offset_eq(
            fixture.surface.content_offset().y,
            raw_target_y(&fixture, 800.0),
            "the flight lands on the target",
        );
        assert_offset_eq(
            reported_y(&fixture),
            800.0,
            "the reported offset ends on the logical target",
        );
    }

    /// A target past the document's end lands at the constrained offset —
    /// the clamp the clocked and `Default` paths compute before
    /// animating.
    fn an_animation_past_the_end_lands_at_the_clamped_offset() {
        let fixture = mounted();
        fixture.controller.animate_to(
            Point::new(0.0, 1.0e6),
            Animation::linear(Duration::from_millis(400)),
        );
        pump_flight(
            || fixture.surface.scroll_animation_in_flight(),
            || reported_y(&fixture),
        );
        assert_offset_eq(
            fixture.surface.content_offset().y,
            end_offset(&fixture),
            "a flight past the end lands at the scrollable end",
        );
    }

    /// A bare request past the document's end lands at the constrained
    /// offset — the same end the animated paths aim at (#2110). `AppKit`'s
    /// `scrollToPoint:` applies its argument verbatim, so the shared jump
    /// clamps the point through `ScrollFlight::constrained` first.
    fn a_bare_request_past_the_end_lands_at_the_clamped_offset() {
        let fixture = mounted();
        fixture.controller.scroll_to(Point::new(0.0, 1.0e6));
        assert_offset_eq(
            fixture.surface.content_offset().y,
            end_offset(&fixture),
            "a bare jump past the end lands at the scrollable end",
        );
    }

    /// A second request during a flight replaces it: the offset settles
    /// on the later target, and the superseded flight never lands.
    fn a_later_request_takes_over_an_in_flight_animation() {
        let fixture = mounted();
        fixture.controller.animate_to(
            Point::new(0.0, 1000.0),
            Animation::linear(Duration::from_secs(2)),
        );
        // No pump: the two-second flight is provably still in progress
        // when the new request supersedes it.
        fixture.controller.animate_to(
            Point::new(0.0, 200.0),
            Animation::linear(Duration::from_millis(500)),
        );
        let samples = pump_flight(
            || fixture.surface.scroll_animation_in_flight(),
            || reported_y(&fixture),
        );
        assert!(
            samples.iter().all(|&y| (y - 1000.0).abs() > 1.0),
            "the superseded flight must never land: {samples:?}"
        );
        assert_offset_eq(
            fixture.surface.content_offset().y,
            raw_target_y(&fixture, 200.0),
            "the later request lands on its own target",
        );
    }

    /// A bare request during a flight jumps at once — the in-flight
    /// animation is cancelled, not resumed, and nothing is left to
    /// rewrite the offset.
    fn a_bare_request_takes_over_an_in_flight_animation() {
        let fixture = mounted();
        fixture.controller.animate_to(
            Point::new(0.0, 1000.0),
            Animation::linear(Duration::from_secs(2)),
        );
        fixture.controller.scroll_to(Point::new(0.0, 300.0));
        assert_offset_eq(
            fixture.surface.content_offset().y,
            raw_target_y(&fixture, 300.0),
            "the interrupting jump lands at once",
        );
        assert!(
            !fixture.surface.scroll_animation_in_flight(),
            "the cancelled flight must leave no animation running"
        );
        assert_offset_eq(
            reported_y(&fixture),
            300.0,
            "the reported offset follows the jump",
        );
    }

    /// The user's own wheel scroll wins mid-flight: a phase-less legacy
    /// wheel event sent through the surface's `scrollWheel:` makes
    /// `AppKit` post `NSScrollViewDidLiveScrollNotification`, the
    /// animation retires, and the offset the wheel produced stands past
    /// the flight's original end.
    #[cfg(target_os = "macos")]
    fn a_users_wheel_scroll_takes_over_an_in_flight_animation() {
        use cocoa_ui::objc2_quartz_core::CACurrentMediaTime;
        use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};

        let fixture = mounted();
        let (_observer, live_scrolls) =
            super::appkit_input::count_live_scrolls(super::mtm(), &fixture.surface);
        fixture.controller.animate_to(
            Point::new(0.0, 1000.0),
            Animation::linear(Duration::from_secs(2)),
        );
        assert!(
            fixture.surface.scroll_animation_in_flight(),
            "the flight must be in progress when the user's scroll starts"
        );
        let flight_end = CACurrentMediaTime() + 2.0;
        let before = fixture.surface.content_offset().y;
        super::appkit_input::wheel(&fixture.surface, -40);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || live_scrolls.get() > 0),
            "AppKit must report the wheel scroll as a live scroll"
        );
        assert!(
            !fixture.surface.scroll_animation_in_flight(),
            "the user's wheel scroll must retire the flight"
        );
        // Outlast the flight's original schedule: had it survived, its
        // ticks would still be writing toward 1000 — the user's offset
        // stands.
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || CACurrentMediaTime() >= flight_end),
            "the flight's original schedule must elapse"
        );
        let settled = fixture.surface.content_offset().y;
        assert!(
            settled > before && (settled - 1000.0).abs() > 1.0,
            "the wheel's scroll stands — the retired flight never rewrites the model: \
             before {before}, settled {settled}"
        );
    }

    /// The keyboard wins mid-flight: a page-down sent through the window
    /// to a focused view in the document reaches the scroll view's
    /// `pageDown:`, whose clip write is not the flight's own — the
    /// animation retires synchronously and the paged offset stands past
    /// the flight's original end.
    #[cfg(target_os = "macos")]
    fn a_page_down_takes_over_an_in_flight_animation() {
        use cocoa_ui::objc2_app_kit::NSResponder;
        use cocoa_ui::objc2_quartz_core::CACurrentMediaTime;
        use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};

        let fixture = mounted();
        let window = fixture
            .surface
            .window()
            .expect("the mounted surface is in a window");
        let focus = super::appkit_input::FocusTarget::new(super::mtm());
        fixture
            .surface
            .document_view()
            .expect("a laid-out scroll surface has a document")
            .addSubview(&focus);
        let responder: &NSResponder = &focus;
        assert!(
            window.makeFirstResponder(Some(responder)),
            "the focus target must take first responder"
        );
        fixture.controller.animate_to(
            Point::new(0.0, 1000.0),
            Animation::linear(Duration::from_secs(2)),
        );
        let flight_end = CACurrentMediaTime() + 2.0;
        let before = fixture.surface.content_offset().y;
        super::appkit_input::press(&window, super::appkit_input::PAGE_DOWN);
        assert!(
            !fixture.surface.scroll_animation_in_flight(),
            "the keyboard's page-down must retire the flight"
        );
        let paged = fixture.surface.content_offset().y;
        assert!(
            paged > before,
            "page-down scrolls the document: before {before}, after {paged}"
        );
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || CACurrentMediaTime() >= flight_end),
            "the flight's original schedule must elapse"
        );
        assert_offset_eq(
            fixture.surface.content_offset().y,
            paged,
            "the paged offset stands — the retired flight never rewrites the model",
        );
    }

    /// Nothing the kit installs on the surface costs `AppKit`'s responsive
    /// scrolling: the scroll view, its kit clip view and the document all
    /// stay compatible.
    #[cfg(target_os = "macos")]
    fn the_surface_keeps_responsive_scrolling() {
        use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};

        let fixture = mounted();
        let clip = fixture.surface.clip_view();
        // `isKindOfClass:` — key-value observation swaps the instance's
        // class for a runtime subclass.
        let flight_clip = AnyClass::get(c"CocoaUiFlightClipView")
            .expect("the kit registers its clip view class on first use");
        assert!(
            clip.isKindOfClass(flight_clip),
            "the surface scrolls through the kit's clip view"
        );
        let document = fixture
            .surface
            .document_view()
            .expect("a laid-out scroll surface has a document");
        let views: [(&str, &AnyObject); 3] = [
            ("scroll view", &fixture.surface),
            ("clip view", &clip),
            ("document view", &document),
        ];
        for (role, view) in views {
            assert!(
                super::appkit_input::responsive_scrolling_compatible(view),
                "the {role} must stay responsive-scrolling compatible"
            );
        }
        let regression = super::appkit_input::WheelScrollView::new(super::mtm());
        assert!(
            !super::appkit_input::responsive_scrolling_compatible(&regression),
            "a scrollWheel: override must read as responsive-scrolling incompatible"
        );
    }

    /// `AppKit`'s own clip corrections — the document growing, the
    /// window resizing — leave a flight running, and it lands on its
    /// target. The 1000pt target stays inside the travel throughout (the
    /// end offset never drops below it), so the landing checks the
    /// flight, not a clamp.
    #[cfg(target_os = "macos")]
    fn appkit_corrections_keep_an_in_flight_animation() {
        use cocoa_ui::objc2_foundation::NSSize;

        let fixture = mounted();
        let window = fixture
            .surface
            .window()
            .expect("the mounted surface is in a window");
        fixture.controller.animate_to(
            Point::new(0.0, 1000.0),
            Animation::linear(Duration::from_secs(2)),
        );
        super::appkit_input::assert_flight_survives_corrections(
            &window,
            1000.0,
            || fixture.surface.scroll_animation_in_flight(),
            || fixture.surface.content_offset().y,
            || {
                let extent = fixture
                    .surface
                    .document_view()
                    .expect("a laid-out scroll surface has a document")
                    .frame()
                    .size;
                fixture
                    .surface
                    .set_document_extent(NSSize::new(extent.width, extent.height + 500.0).into());
            },
        );
        assert_offset_eq(
            fixture.surface.content_offset().y,
            raw_target_y(&fixture, 1000.0),
            "the flight lands on its target",
        );
    }

    /// A surface leaving its window lands the flight it was running
    /// exactly on the target, at once.
    fn leaving_the_window_lands_an_in_flight_animation(name: &'static str, animation: Animation) {
        let fixture = mounted();
        fixture
            .controller
            .animate_to(Point::new(0.0, 800.0), animation);
        assert!(
            fixture.surface.scroll_animation_in_flight(),
            "the {name} flight must be in progress after the request"
        );
        fixture.surface.removeFromSuperview();
        assert!(
            !fixture.surface.scroll_animation_in_flight(),
            "leaving the window must retire the {name} flight"
        );
        assert_offset_eq(
            fixture.surface.content_offset().y,
            raw_target_y(&fixture, 800.0),
            &format!("the {name} flight lands on its target"),
        );
    }

    /// The `scroll::` trials — one per animation case plus the takeover
    /// and clamp cases.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        let mut named: Vec<(String, Box<dyn FnOnce() + Send>)> = vec![(
            "scroll::a_bare_request_jumps_to_the_target".to_owned(),
            Box::new(a_bare_request_jumps_to_the_target),
        )];
        named.extend(animation_cases().into_iter().map(|(name, animation)| {
            let case: Box<dyn FnOnce() + Send> =
                Box::new(move || an_animation_scrolls_to_the_target(name, animation));
            (
                format!("scroll::an_{name}_animation_scrolls_to_the_target"),
                case,
            )
        }));
        named.extend(animation_cases().into_iter().map(|(name, animation)| {
            let case: Box<dyn FnOnce() + Send> =
                Box::new(move || leaving_the_window_lands_an_in_flight_animation(name, animation));
            (
                format!("scroll::leaving_the_window_lands_an_in_flight_{name}_animation"),
                case,
            )
        }));
        named.push((
            "scroll::an_animation_past_the_end_lands_at_the_clamped_offset".to_owned(),
            Box::new(an_animation_past_the_end_lands_at_the_clamped_offset),
        ));
        named.push((
            "scroll::a_bare_request_past_the_end_lands_at_the_clamped_offset".to_owned(),
            Box::new(a_bare_request_past_the_end_lands_at_the_clamped_offset),
        ));
        named.push((
            "scroll::a_later_request_takes_over_an_in_flight_animation".to_owned(),
            Box::new(a_later_request_takes_over_an_in_flight_animation),
        ));
        named.push((
            "scroll::a_bare_request_takes_over_an_in_flight_animation".to_owned(),
            Box::new(a_bare_request_takes_over_an_in_flight_animation),
        ));
        #[cfg(target_os = "macos")]
        named.extend([
            (
                "scroll::a_users_wheel_scroll_takes_over_an_in_flight_animation".to_owned(),
                Box::new(a_users_wheel_scroll_takes_over_an_in_flight_animation)
                    as Box<dyn FnOnce() + Send>,
            ),
            (
                "scroll::a_page_down_takes_over_an_in_flight_animation".to_owned(),
                Box::new(a_page_down_takes_over_an_in_flight_animation),
            ),
            (
                "scroll::the_surface_keeps_responsive_scrolling".to_owned(),
                Box::new(the_surface_keeps_responsive_scrolling),
            ),
            (
                "scroll::appkit_corrections_keep_an_in_flight_animation".to_owned(),
                Box::new(appkit_corrections_keep_an_in_flight_animation),
            ),
        ]);
        trial_each(named)
    }
}

/// `ScrollController<usize>` drives a row target into a mounted list:
/// a bare request puts the row's top at the clip's top at once, an
/// animated request lands on the same position — `Animation::Default`
/// through the platform's animated row scroll, an explicit animation
/// written to the model each frame-clock tick — and a request issued
/// mid-flight takes over.
mod list_scroll {
    use std::time::Duration;

    use waterui::ViewExt;
    use waterui::animation::Animation;
    use waterui::component::list::ListItem;
    use waterui::layout::scroll::ScrollController;
    use waterui::prelude::text;
    use waterui_apple::contract::NativeLeaf;

    use super::{
        AttachedWindow, Retained, animation_cases, assert_offset_eq, distinct_offsets,
        mount_and_order_front, pump_flight, trial_each,
    };

    use waterui_apple::native_test_support::{
        APPROACH_LIST_ROWS, APPROACH_ROW, APPROACH_TARGET_ROW, ListSurface as TableSurface,
        list_row_top, row_item, row_list,
    };

    /// A list taller than the window by far — row targets past the fold.
    const ROWS: usize = 200;
    /// The row the cases aim at.
    const TARGET: usize = 40;
    /// Slack for sub-point rounding when an offset is compared with the
    /// scrollable end on either kit: a row scroll can land a fraction of a
    /// thousandth of a point short of the end its content size implies, as
    /// `UIKit`'s own row scroll does.
    const EPSILON: f64 = 1.0e-3;

    /// A mounted list of `ROWS` rows, the controller that drives it, and
    /// the leaf and window that keep the wiring alive.
    struct Fixture {
        table: Retained<TableSurface>,
        controller: ScrollController<usize>,
        _leaf: NativeLeaf,
        _window: AttachedWindow,
    }

    /// The clip offset the jump's family lands `row` on: its
    /// [`list_row_top`]. `TARGET` sits far from either end, where no
    /// clamp applies.
    fn row_top_offset(fixture: &Fixture, row: usize) -> f64 {
        list_row_top(&fixture.table, row).y
    }

    /// The clip's current scroll position.
    fn offset_y(fixture: &Fixture) -> f64 {
        #[cfg(target_os = "macos")]
        {
            fixture
                .table
                .table_view()
                .enclosingScrollView()
                .expect("the list table lives in its scroll view")
                .contentView()
                .bounds()
                .origin
                .y
        }
        #[cfg(target_os = "ios")]
        {
            fixture.table.contentOffset().y
        }
    }

    /// The largest offset the list admits — the clip's constrained end on
    /// `AppKit`, the inset-adjusted end on `UIKit`.
    fn end_offset(fixture: &Fixture) -> f64 {
        #[cfg(target_os = "macos")]
        {
            let clip = fixture
                .table
                .table_view()
                .enclosingScrollView()
                .expect("the list table lives in its scroll view")
                .contentView();
            clip.constrainBoundsRect(cocoa_ui::objc2_foundation::NSRect::new(
                cocoa_ui::objc2_foundation::NSPoint::new(0.0, 1.0e6),
                clip.bounds().size,
            ))
            .origin
            .y
        }
        #[cfg(target_os = "ios")]
        {
            let inset = fixture.table.adjustedContentInset();
            (fixture.table.contentSize().height - fixture.table.bounds().size.height + inset.bottom)
                .max(-inset.top)
        }
    }

    /// A row taller than the rest — mixed heights keep the last row's
    /// offset an honest measure instead of a multiple of the first.
    fn tall_row_item() -> ListItem {
        ListItem::new(text("row").padding_with(24.0))
    }

    /// Mounts `rows` in a list driven by a fresh `usize` controller.
    fn mounted_with(rows: Vec<fn() -> ListItem>) -> Fixture {
        let controller = ScrollController::new(0usize);
        let (leaf, table) = row_list(rows, &controller);
        let window = mount_and_order_front(leaf.view());
        table.layout_if_needed();
        #[cfg(target_os = "macos")]
        super::appkit_input::present_first_frame(super::mtm(), leaf.view());
        let fixture = Fixture {
            table,
            controller,
            _leaf: leaf,
            _window: window,
        };
        assert!(
            row_top_offset(&fixture, TARGET) > 0.0,
            "row {TARGET} must sit below the fold"
        );
        fixture
    }

    /// Mounts a uniform list.
    fn mounted() -> Fixture {
        mounted_with(vec![row_item as fn() -> ListItem; ROWS])
    }

    /// Mounts a list whose rows differ in height — every fourth row
    /// carries padding.
    fn mounted_mixed_heights() -> Fixture {
        mounted_with(
            (0..ROWS)
                .map(|index| {
                    if index % 4 == 0 {
                        tall_row_item as fn() -> ListItem
                    } else {
                        row_item as fn() -> ListItem
                    }
                })
                .collect(),
        )
    }

    /// No animation on the request: `row`'s top is the clip's top before
    /// any pumping.
    fn a_bare_request_scrolls_the_row_to_the_top() {
        let fixture = mounted();
        fixture.controller.scroll_to(TARGET);
        assert_offset_eq(
            offset_y(&fixture),
            row_top_offset(&fixture, TARGET),
            "a jump lands the row's top on the clip's top immediately",
        );
    }

    /// An animated request ends exactly where the jump lands. An
    /// explicit curve moves the clip through intermediate offsets — the
    /// model follows each tick, so lazily built rows materialize as the
    /// flight passes them. `Default` is the platform's wall-clock
    /// animation: it does not jump, and it takes time to land.
    fn an_animation_scrolls_the_row_to_the_top(name: &'static str, animation: Animation) {
        let fixture = mounted();
        let native = matches!(animation, Animation::Default);
        let request = || {
            fixture.controller.animate_to(TARGET, animation);
            assert!(
                fixture.table.scroll_animation_in_flight(),
                "the {name} row flight must be in progress after the request"
            );
        };
        if native {
            waterui_apple::native_test_support::assert_native_scroll(
                &format!("the {name} row flight"),
                request,
                || cocoa_ui::Point::new(0.0, offset_y(&fixture)),
                || cocoa_ui::Point::new(0.0, row_top_offset(&fixture, TARGET)),
            );
            pump_flight(
                || fixture.table.scroll_animation_in_flight(),
                || offset_y(&fixture),
            );
        } else {
            request();
            let samples = pump_flight(
                || fixture.table.scroll_animation_in_flight(),
                || offset_y(&fixture),
            );
            assert!(
                distinct_offsets(&samples) >= 3,
                "the {name} row flight must move through intermediate offsets: {samples:?}"
            );
        }
        assert_offset_eq(
            offset_y(&fixture),
            row_top_offset(&fixture, TARGET),
            "the row flight lands on the jump's position",
        );
    }

    /// A target further than the approach bound animates only the final
    /// stretch: the request jumps unanimated to the approach row's top
    /// first — the first offset seen after it — and the animation then
    /// lands on the target row's top. An explicit curve moves the clip
    /// through intermediate offsets from there; `Default` is the
    /// platform's wall-clock animation from the approach row.
    fn a_far_animation_jumps_to_the_approach_row_first(name: &'static str, animation: Animation) {
        let fixture = mounted_with(vec![row_item as fn() -> ListItem; APPROACH_LIST_ROWS]);
        let native = matches!(animation, Animation::Default);
        let request = || {
            fixture
                .controller
                .animate_to(APPROACH_TARGET_ROW, animation);
            assert!(
                fixture.table.scroll_animation_in_flight(),
                "the far {name} row flight must be in progress after the request"
            );
        };
        if native {
            waterui_apple::native_test_support::assert_native_scroll_from(
                &format!("the far {name} row flight"),
                request,
                || cocoa_ui::Point::new(0.0, row_top_offset(&fixture, APPROACH_ROW)),
                || cocoa_ui::Point::new(0.0, offset_y(&fixture)),
                || cocoa_ui::Point::new(0.0, row_top_offset(&fixture, APPROACH_TARGET_ROW)),
            );
            pump_flight(
                || fixture.table.scroll_animation_in_flight(),
                || offset_y(&fixture),
            );
        } else {
            request();
            let approach_top = row_top_offset(&fixture, APPROACH_ROW);
            assert_offset_eq(
                offset_y(&fixture),
                approach_top,
                "the far request jumps to the approach row's top before it animates",
            );
            let samples = pump_flight(
                || fixture.table.scroll_animation_in_flight(),
                || offset_y(&fixture),
            );
            assert!(
                distinct_offsets(&samples) >= 3,
                "the far {name} row flight must move through intermediate offsets: {samples:?}"
            );
            assert!(
                samples.iter().all(|&y| y >= approach_top),
                "the far {name} row flight never falls back short of the approach row's top {approach_top}: {samples:?}"
            );
        }
        assert_offset_eq(
            offset_y(&fixture),
            row_top_offset(&fixture, APPROACH_TARGET_ROW),
            "the far row flight lands on the target row's top",
        );
    }

    /// An overshooting spring toward the last row holds at the scrollable
    /// end: the curve passes 1 — unclamped, it would carry the clip more
    /// than a point into the blank space past the last row — yet every
    /// frame's write is clamped, so no sample passes the end as it
    /// stands at that frame, and the flight lands on it.
    fn an_overshooting_spring_toward_the_last_rows_never_passes_the_end() {
        let spring = Animation::Spring {
            stiffness: 300.0,
            damping: 6.0,
        };
        let fixture = mounted_mixed_heights();
        fixture.controller.animate_to(ROWS - 1, spring.clone());
        let from = offset_y(&fixture);
        let past_end = pump_flight(
            || fixture.table.scroll_animation_in_flight(),
            || offset_y(&fixture) - end_offset(&fixture),
        );
        let end = end_offset(&fixture);
        let peak = (0..=spring.duration().as_millis())
            .map(|ms| {
                spring.progress(Duration::from_millis(
                    u64::try_from(ms).expect("the spring's duration fits u64 milliseconds"),
                ))
            })
            .fold(f32::MIN, f32::max);
        assert!(
            (f64::from(peak) - 1.0) * (end - from) > 1.0,
            "the spring must overshoot the end by more than a point unclamped: peak progress {peak} from {from} to {end}"
        );
        assert!(
            past_end.iter().all(|&beyond| beyond <= EPSILON),
            "no sample may pass the scrollable end; offsets past it: {past_end:?}"
        );
        let landed = offset_y(&fixture);
        assert!(
            (landed - end).abs() <= EPSILON,
            "the overshooting spring lands on the scrollable end: landed {landed}, end {end}"
        );
    }

    /// A row in the last screenful resolves inside the scrollable range:
    /// every sample of the linear flight climbs monotonically toward the
    /// landed offset and never passes it or the document's end — no
    /// overshoot into the blank space past the last row — and it lands
    /// on the scrollable end, where the jump lands. Rows differ in height
    /// so the last row's rect is a real measure, not a multiple of the
    /// first.
    fn the_last_rows_animation_lands_where_the_jump_lands() {
        let fixture = mounted_mixed_heights();
        fixture
            .controller
            .animate_to(ROWS - 1, Animation::linear(Duration::from_millis(400)));
        let samples = pump_flight(
            || fixture.table.scroll_animation_in_flight(),
            || offset_y(&fixture),
        );
        let landed = offset_y(&fixture);
        let end = end_offset(&fixture);
        assert!(
            (landed - end).abs() <= EPSILON,
            "the last row's flight lands on the scrollable end: landed {landed}, end {end}"
        );
        assert!(
            samples
                .iter()
                .all(|&y| y <= landed + EPSILON && y <= end + EPSILON),
            "no sample may pass the landed offset {landed} or the end {end}: {samples:?}"
        );
        assert!(
            samples.windows(2).all(|pair| pair[1] >= pair[0] - EPSILON),
            "the linear flight's samples must never decrease: {samples:?}"
        );
        // The real jump to the same row must not move the offset: the
        // flight's landing is the jump's by construction.
        fixture.controller.scroll_to(ROWS - 1);
        assert_offset_eq(
            offset_y(&fixture),
            landed,
            "the row flight lands where the jump lands",
        );
    }

    /// A request issued mid-flight replaces the running one: the clip
    /// settles on the later row, and the superseded flight never lands.
    fn a_later_request_takes_over_an_in_flight_animation() {
        let fixture = mounted();
        fixture
            .controller
            .animate_to(150, Animation::linear(Duration::from_secs(2)));
        // No pump: the two-second flight is provably still in progress
        // when the new request supersedes it.
        fixture
            .controller
            .animate_to(20, Animation::linear(Duration::from_millis(500)));
        let samples = pump_flight(
            || fixture.table.scroll_animation_in_flight(),
            || offset_y(&fixture),
        );
        let far = row_top_offset(&fixture, 150);
        assert!(
            samples.iter().all(|&y| (y - far).abs() > 1.0),
            "the superseded row flight must never land: {samples:?}"
        );
        assert_offset_eq(
            offset_y(&fixture),
            row_top_offset(&fixture, 20),
            "the later request lands on its own row",
        );
    }

    /// A bare request during a flight jumps at once — the in-flight
    /// animation is cancelled, not resumed, and nothing is left to
    /// rewrite the offset.
    fn a_bare_request_takes_over_an_in_flight_animation() {
        let fixture = mounted();
        fixture
            .controller
            .animate_to(150, Animation::linear(Duration::from_secs(2)));
        fixture.controller.scroll_to(10);
        assert_offset_eq(
            offset_y(&fixture),
            row_top_offset(&fixture, 10),
            "the interrupting jump lands at once",
        );
        assert!(
            !fixture.table.scroll_animation_in_flight(),
            "the cancelled flight must leave no animation running"
        );
    }

    /// The keyboard wins mid-flight: `key`, sent through the window to the
    /// focused table, scrolls the clip through a write that is not the
    /// flight's own — `pageDown:` on the scroll view, or the table's own
    /// `scrollToEndOfDocument:`, which posts no live-scroll notification —
    /// and the flight retires synchronously; the keyed offset stands past
    /// the flight's original end.
    #[cfg(target_os = "macos")]
    fn the_keyboard_takes_over_an_in_flight_animation(
        name: &'static str,
        key: super::appkit_input::FunctionKey,
    ) {
        use cocoa_ui::objc2_app_kit::NSResponder;
        use cocoa_ui::objc2_quartz_core::CACurrentMediaTime;
        use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};

        let fixture = mounted();
        let table = fixture.table.table_view();
        let window = table.window().expect("the mounted list is in a window");
        let responder: &NSResponder = &table;
        assert!(
            window.makeFirstResponder(Some(responder)),
            "the table must take first responder"
        );
        fixture
            .controller
            .animate_to(150, Animation::linear(Duration::from_secs(2)));
        assert!(
            fixture.table.scroll_animation_in_flight(),
            "the row flight must be in progress when {name} is pressed"
        );
        let flight_end = CACurrentMediaTime() + 2.0;
        super::appkit_input::press(&window, key);
        assert!(
            !fixture.table.scroll_animation_in_flight(),
            "{name} must retire the flight"
        );
        let keyed = offset_y(&fixture);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || CACurrentMediaTime() >= flight_end),
            "the flight's original schedule must elapse"
        );
        assert_offset_eq(
            offset_y(&fixture),
            keyed,
            "the keyed offset stands — the retired flight never rewrites the model",
        );
        assert!(
            (keyed - row_top_offset(&fixture, 150)).abs() > 1.0,
            "the superseded flight never landed: {keyed}"
        );
    }

    /// Nothing the kit installs on the list costs `AppKit`'s responsive
    /// scrolling: the scroll view, its kit clip view and the table all
    /// stay compatible.
    #[cfg(target_os = "macos")]
    fn the_list_keeps_responsive_scrolling() {
        use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};

        let fixture = mounted();
        let table = fixture.table.table_view();
        let clip = fixture.table.contentView();
        // `isKindOfClass:` — key-value observation swaps the instance's
        // class for a runtime subclass.
        let flight_clip = AnyClass::get(c"CocoaUiFlightClipView")
            .expect("the kit registers its clip view class on first use");
        assert!(
            clip.isKindOfClass(flight_clip),
            "the list scrolls through the kit's clip view"
        );
        let views: [(&str, &AnyObject); 3] = [
            ("scroll view", &fixture.table),
            ("clip view", &clip),
            ("table", &table),
        ];
        for (role, view) in views {
            assert!(
                super::appkit_input::responsive_scrolling_compatible(view),
                "the {role} must stay responsive-scrolling compatible"
            );
        }
    }

    /// `AppKit`'s own clip corrections — the table growing, the window
    /// resizing — leave a row flight running, and it lands on the row's
    /// top as the final geometry places it. Row 40 sits far from the end,
    /// so no clamp is involved.
    #[cfg(target_os = "macos")]
    fn appkit_corrections_keep_an_in_flight_animation() {
        let fixture = mounted();
        let window = fixture
            .table
            .window()
            .expect("the mounted list is in a window");
        let target = row_top_offset(&fixture, TARGET);
        fixture
            .controller
            .animate_to(TARGET, Animation::linear(Duration::from_secs(2)));
        super::appkit_input::assert_flight_survives_corrections(
            &window,
            target,
            || fixture.table.scroll_animation_in_flight(),
            || offset_y(&fixture),
            || {
                let table = fixture.table.table_view();
                let mut size = table.frame().size;
                size.height += 500.0;
                table.setFrameSize(size);
            },
        );
        assert_offset_eq(
            offset_y(&fixture),
            row_top_offset(&fixture, TARGET),
            "the row flight lands on the row's top",
        );
    }

    /// A list leaving its window lands the row flight it was running
    /// exactly where the jump lands, at once.
    fn leaving_the_window_lands_an_in_flight_animation(name: &'static str, animation: Animation) {
        let fixture = mounted();
        fixture.controller.animate_to(TARGET, animation);
        assert!(
            fixture.table.scroll_animation_in_flight(),
            "the {name} row flight must be in progress after the request"
        );
        fixture.table.removeFromSuperview();
        assert!(
            !fixture.table.scroll_animation_in_flight(),
            "leaving the window must retire the {name} row flight"
        );
        assert_offset_eq(
            offset_y(&fixture),
            row_top_offset(&fixture, TARGET),
            &format!("the {name} row flight lands on the jump's position"),
        );
    }

    /// The `list_scroll::` trials — one per animation case plus the
    /// takeover and last-row cases.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        let mut named: Vec<(String, Box<dyn FnOnce() + Send>)> = vec![(
            "list_scroll::a_bare_request_scrolls_the_row_to_the_top".to_owned(),
            Box::new(a_bare_request_scrolls_the_row_to_the_top),
        )];
        named.extend(animation_cases().into_iter().map(|(name, animation)| {
            let case: Box<dyn FnOnce() + Send> =
                Box::new(move || an_animation_scrolls_the_row_to_the_top(name, animation));
            (
                format!("list_scroll::an_{name}_animation_scrolls_the_row_to_the_top"),
                case,
            )
        }));
        named.extend(animation_cases().into_iter().map(|(name, animation)| {
            let case: Box<dyn FnOnce() + Send> =
                Box::new(move || a_far_animation_jumps_to_the_approach_row_first(name, animation));
            (
                format!("list_scroll::a_far_{name}_animation_jumps_to_the_approach_row_first"),
                case,
            )
        }));
        named.extend(animation_cases().into_iter().map(|(name, animation)| {
            let case: Box<dyn FnOnce() + Send> =
                Box::new(move || leaving_the_window_lands_an_in_flight_animation(name, animation));
            (
                format!("list_scroll::leaving_the_window_lands_an_in_flight_{name}_animation"),
                case,
            )
        }));
        #[cfg(target_os = "macos")]
        named.extend([
            (
                "list_scroll::a_page_down_takes_over_an_in_flight_animation".to_owned(),
                Box::new(|| {
                    the_keyboard_takes_over_an_in_flight_animation(
                        "page-down",
                        super::appkit_input::PAGE_DOWN,
                    );
                }) as Box<dyn FnOnce() + Send>,
            ),
            (
                "list_scroll::the_end_key_takes_over_an_in_flight_animation".to_owned(),
                Box::new(|| {
                    the_keyboard_takes_over_an_in_flight_animation("End", super::appkit_input::END);
                }),
            ),
            (
                "list_scroll::the_list_keeps_responsive_scrolling".to_owned(),
                Box::new(the_list_keeps_responsive_scrolling),
            ),
            (
                "list_scroll::appkit_corrections_keep_an_in_flight_animation".to_owned(),
                Box::new(appkit_corrections_keep_an_in_flight_animation),
            ),
        ]);
        named.push((
            "list_scroll::the_last_rows_animation_lands_where_the_jump_lands".to_owned(),
            Box::new(the_last_rows_animation_lands_where_the_jump_lands),
        ));
        named.push((
            "list_scroll::an_overshooting_spring_toward_the_last_rows_never_passes_the_end"
                .to_owned(),
            Box::new(an_overshooting_spring_toward_the_last_rows_never_passes_the_end),
        ));
        named.push((
            "list_scroll::a_later_request_takes_over_an_in_flight_animation".to_owned(),
            Box::new(a_later_request_takes_over_an_in_flight_animation),
        ));
        named.push((
            "list_scroll::a_bare_request_takes_over_an_in_flight_animation".to_owned(),
            Box::new(a_bare_request_takes_over_an_in_flight_animation),
        ));
        trial_each(named)
    }
}

/// Navigation-chrome trials: the bar intents a page records in
/// `set_page` apply through the stack's `UINavigationControllerDelegate`.
#[cfg(target_os = "ios")]
mod navigation {
    use cocoa_ui::objc2_ui_kit::{UINavigationController, UIView, UIViewController};
    use cocoa_ui::uikit::view_controller::owning_controller;
    use cocoa_ui::uikit::{NavContentController, NavPage};
    use cocoa_ui::{PlatformView, Retained, view};
    use waterui::navigation::{
        NavigationStack, NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement,
        NavigationView,
    };
    use waterui::prelude::text;
    use waterui::reactive::binding;
    use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};

    use super::{mtm, resolve};

    /// The `UINavigationController` owning a view in the subtree,
    /// depth-first — either the view's own controller is the nav
    /// controller, or the page controller answers one.
    fn nav_controller_in(view: &PlatformView) -> Option<Retained<UINavigationController>> {
        if let Some(controller) = owning_controller(view) {
            if let Ok(nav) = controller.clone().downcast() {
                return Some(nav);
            }
            if let Some(nav) = controller.navigationController() {
                return Some(nav);
            }
        }
        for sub in view::subviews(view) {
            if let Some(found) = nav_controller_in(&sub) {
                return Some(found);
            }
        }
        None
    }

    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "navigation::a_first_page_bottom_bar_unhides_the_toolbar",
                || {
                    a_first_page_bottom_bar_unhides_the_toolbar();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "navigation::the_top_page_drives_the_navigation_bar",
                || {
                    the_top_page_drives_the_navigation_bar();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "navigation::a_buried_pages_set_page_leaves_the_bar_alone",
                || {
                    a_buried_pages_set_page_leaves_the_bar_alone();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "navigation::the_top_pages_reactive_hidden_writes_and_rides_transitions",
                || {
                    the_top_pages_reactive_hidden_writes_and_rides_transitions();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "navigation::a_buried_pages_reactive_hidden_leaves_the_bar_alone",
                || {
                    a_buried_pages_reactive_hidden_leaves_the_bar_alone();
                    Ok(())
                },
            ),
        ]
    }

    /// The root page's intent is recorded in `set_page` while
    /// `navigationController()` is still nil — a first page declaring
    /// bottom items must still unhide the stack's toolbar once the page
    /// resolves. Without it the iOS 26 floating bottom bar never mounts.
    ///
    /// A cancelled interactive pop cannot be driven here: starting one
    /// needs the edge pan's `UITouch` stream, and `UITouch` has no
    /// public initializer the harness can construct.
    fn a_first_page_bottom_bar_unhides_the_toolbar() {
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Root", text("root")).navigation_toolbar(NavigationToolbar::new(
                vec![NavigationToolbarItem::new(
                    NavigationToolbarPlacement::BottomBar,
                    text("Action"),
                )],
            )),
        ));
        let nav =
            nav_controller_in(leaf.view()).expect("the stack mounts a UINavigationController");
        assert!(
            !nav.isToolbarHidden(),
            "the first page's bottom bar unhides the toolbar"
        );
    }

    /// A stack whose root page hides the navigation bar — the fixture
    /// the bar trials push onto. The leaf mounts in a key window:
    /// `UINavigationController` only delivers its transition delegate
    /// callbacks to an attached view hierarchy. The returned leaf and
    /// window keep the driver and the hierarchy alive; the controller
    /// is the stack's `UINavigationController`.
    fn hidden_root_stack() -> (
        waterui_apple::contract::NativeLeaf,
        Retained<cocoa_ui::objc2_ui_kit::UIWindow>,
        Retained<UINavigationController>,
    ) {
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Root", text("root")).navigation_bar_visibility(false),
        ));
        let window = super::mount_and_order_front(leaf.view());
        let nav =
            nav_controller_in(leaf.view()).expect("the stack mounts a UINavigationController");
        (leaf, window, nav)
    }

    /// A pushed page carrying its own `hidden` intent — chrome the
    /// harness installs natively, the way the driver installs the
    /// model's pushed pages.
    fn pushed_page(hidden: bool) -> Retained<NavContentController> {
        let page = NavContentController::new(mtm(), &UIView::new(mtm()));
        page.set_page(&NavPage {
            hidden,
            ..NavPage::default()
        });
        page
    }

    /// Root hides the bar, a pushed page shows it: each transition
    /// applies the incoming page's recorded intent — the bar is shown
    /// after the push and hidden again after the pop. `willShow` lands
    /// inside the transition's main-queue delivery, so the assertions
    /// pump the run loop rather than read the flag mid-flush.
    ///
    /// A cancelled interactive pop cannot be driven here: starting one
    /// needs the edge pan's `UITouch` stream, and `UITouch` has no
    /// public initializer the harness can construct.
    fn the_top_page_drives_the_navigation_bar() {
        let (_leaf, _window, nav) = hidden_root_stack();
        assert!(
            nav.isNavigationBarHidden(),
            "the root page's hidden intent applies at mount"
        );

        nav.pushViewController_animated(&pushed_page(false), false);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || !nav.isNavigationBarHidden()),
            "the pushed page's shown intent rides the push"
        );

        nav.popViewControllerAnimated(false);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || nav.isNavigationBarHidden()),
            "the pop applies the root page's hidden intent"
        );
    }

    /// `set_page` on a page under the top records intent without
    /// writing the bar — a re-rendered root page never touches the
    /// chrome the pushed page shows.
    fn a_buried_pages_set_page_leaves_the_bar_alone() {
        let (_leaf, _window, nav) = hidden_root_stack();
        nav.pushViewController_animated(&pushed_page(false), false);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || !nav.isNavigationBarHidden()),
            "precondition: the pushed page shows the bar"
        );

        let root = nav
            .viewControllers()
            .objectAtIndex(0)
            .downcast::<NavContentController>()
            .expect("the root page is a NavContentController");
        root.set_page(&NavPage {
            hidden: true,
            ..NavPage::default()
        });
        assert!(
            !pump_main_until(0.5, || nav.isNavigationBarHidden()),
            "a buried page's set_page never reaches the bar"
        );

        // The re-run still recorded the intent: popping back applies it.
        nav.popViewControllerAnimated(false);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || nav.isNavigationBarHidden()),
            "the re-rendered root's intent applies when it becomes top"
        );
    }

    /// Pumps the main queue until `page` is `nav`'s top, the bar reads
    /// `hidden`, and the transition coordinator has torn down — the
    /// three flushes arrive unordered: `topViewController` can re-point
    /// either before or after `willShow` applies the incoming page's
    /// intent, and a bar write issued while the coordinator is still
    /// live is swallowed with it.
    fn settles_with_bar(
        nav: &UINavigationController,
        page: &UIViewController,
        hidden: bool,
    ) -> bool {
        pump_main_until(MAIN_QUEUE_DEADLINE, || {
            nav.topViewController()
                .is_some_and(|top| core::ptr::eq(Retained::as_ptr(&top), page))
                && nav.isNavigationBarHidden() == hidden
                && nav.transitionCoordinator().is_none()
        })
    }

    /// The top page's reactive `hidden` signal writes the bar at once
    /// — and what it records is what a later round trip re-applies:
    /// show reactively on top, hide by pushing, and the pop back shows
    /// the bar — the record the reactive path kept, not the
    /// mount-time flag.
    ///
    /// The reactive write goes first: a harness-driven pop reports a
    /// model pop that drops the popped `Entry` — and the binding
    /// watcher with it — so no reactive write can follow one.
    ///
    /// A cancelled interactive pop cannot be driven here: starting one
    /// needs the edge pan's `UITouch` stream, and `UITouch` has no
    /// public initializer the harness can construct.
    fn the_top_pages_reactive_hidden_writes_and_rides_transitions() {
        let visible = binding(false);
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Root", text("root")).navigation_bar_visibility(visible.clone()),
        ));
        let _window = super::mount_and_order_front(leaf.view());
        let nav =
            nav_controller_in(leaf.view()).expect("the stack mounts a UINavigationController");
        assert!(
            nav.isNavigationBarHidden(),
            "precondition: the root page hides the bar"
        );

        // The top page's reactive change writes the bar at once.
        visible.set(true);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || !nav.isNavigationBarHidden()),
            "the top page's reactive show reaches the bar"
        );

        // Push a hiding page, then pop: the willShow intent the pop
        // re-applies is the reactively-kept record — the bar shows.
        let pushed = pushed_page(true);
        nav.pushViewController_animated(&pushed, false);
        assert!(
            settles_with_bar(&nav, &pushed, true),
            "precondition: the push settles on the pushed page hiding the bar"
        );
        nav.popViewControllerAnimated(false);
        let root = nav.viewControllers().objectAtIndex(0);
        assert!(
            settles_with_bar(&nav, &root, false),
            "the top page's reactive intent survives the push/pop round trip"
        );
    }

    /// A buried page's reactive `hidden` change stays recorded-only
    /// while another page is top — the bar keeps the pushed page's
    /// shown intent — and the record applies when the pop shows the
    /// page again.
    ///
    /// A cancelled interactive pop cannot be driven here: starting one
    /// needs the edge pan's `UITouch` stream, and `UITouch` has no
    /// public initializer the harness can construct.
    fn a_buried_pages_reactive_hidden_leaves_the_bar_alone() {
        let visible = binding(true);
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Root", text("root")).navigation_bar_visibility(visible.clone()),
        ));
        let _window = super::mount_and_order_front(leaf.view());
        let nav =
            nav_controller_in(leaf.view()).expect("the stack mounts a UINavigationController");
        assert!(
            !nav.isNavigationBarHidden(),
            "precondition: the root page shows the bar"
        );

        let pushed = pushed_page(false);
        nav.pushViewController_animated(&pushed, false);
        assert!(
            settles_with_bar(&nav, &pushed, false),
            "precondition: the push settles on the pushed page showing the bar"
        );

        // The buried root asks for a hidden bar — intent only: the
        // pushed page's chrome stays up.
        visible.set(false);
        assert!(
            !pump_main_until(0.5, || nav.isNavigationBarHidden()),
            "a buried page's reactive change leaves the bar alone"
        );

        nav.popViewControllerAnimated(false);
        let root = nav.viewControllers().objectAtIndex(0);
        assert!(
            settles_with_bar(&nav, &root, true),
            "the buried page's recorded intent applies when it shows again"
        );
    }
}
