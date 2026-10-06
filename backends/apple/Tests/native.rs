//! Native tests for the Rust `AppKit`/`UIKit` backend — real platform
//! objects, no visible windows, no application run loop.
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
    libtest_mimic::run(&args, trials()).exit();
}

fn trials() -> Vec<Trial> {
    let tests = vec![
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
            "picture::a_laid_out_picture_rasterizes_at_its_bounds",
            || {
                picture::a_laid_out_picture_rasterizes_at_its_bounds();
                Ok(())
            },
        ),
        Trial::test(
            "owner_lifetimes::a_dropped_mounted_hierarchy_releases_views_and_remounts",
            || {
                owner_lifetimes::a_dropped_mounted_hierarchy_releases_views_and_remounts();
                Ok(())
            },
        ),
    ];
    #[cfg(all(target_os = "macos", feature = "native-test"))]
    let tests = {
        let mut tests = tests;
        tests.extend([
            Trial::test("window::manager_installs_into_the_environment", || {
                window::manager_installs_into_the_environment(mtm());
                Ok(())
            }),
            Trial::test("window::bind_root_window_wires_a_live_window", || {
                window::bind_root_window_wires_a_live_window(mtm());
                Ok(())
            }),
        ]);
        #[cfg(feature = "gpu_surface")]
        tests.extend(gpu_surface::trials());
        tests
    };
    #[cfg(target_os = "ios")]
    let tests = {
        let mut tests = tests;
        tests.extend(tabs::trials());
        tests.extend(controller_bounds::trials());
        tests
    };
    let mut tests = tests;
    tests.extend(migration::trials());
    tests.extend(scroll::trials());
    tests.extend(list_scroll::trials());
    tests
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
/// There is deliberately no `UIKit` `Default` arm: the libtest harness
/// mounts a window with no scene and no `UIApplication`, where the
/// platform's animated scroll never advances the offset, so nothing
/// asserts. The explicit curves write the offset themselves every frame
/// clock tick, so they observe on both kits.
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
        window.addSubview(content);
        // Size the content to the window's bounds the way
        // `set_content_view` does on macOS — `addSubview` alone leaves it
        // at its zero frame, so surfaces that read their viewport (lazy
        // containers especially) would see an empty window.
        cocoa_ui::view::set_frame(content, cocoa_ui::view::bounds(&window));
        window
    }
}

/// View → leaf mapping through `dispatch::render` — the typed entry point
/// a host reaches. A view nobody claims panics (there is no foreign caller
/// to hand it back to), so a spurious empty render could not masquerade as
/// a pass.
mod resolve {
    use waterui::filter::Opacity;
    use waterui::layout::Spacer;
    use waterui::reactive::{SignalExt, binding};
    use waterui_apple::contract::NativeLeaf;
    use waterui_backend_core::{AnyView, Environment, View};
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

    /// The minimum environment a real render needs: `dispatch::install`
    /// performs the backend's half of the embedding contract (dispatcher,
    /// window manager, realizations); the theme slots text resolves
    /// through are the framework's.
    pub fn env() -> Environment {
        use waterui::graphics::color::WorkingColor;
        use waterui::text::font::{Body, Caption, FontSlot, Subheadline};

        let mut env = Environment::new();
        waterui_apple::dispatch::install(&mut env);
        waterui::theme::install_color_scheme(
            &mut env,
            binding(waterui::theme::ColorScheme::Light).computed(),
        );
        let black = || binding(WorkingColor::BLACK).computed();
        waterui::theme::install_color_signal::<waterui::theme::color::Foreground>(
            &mut env,
            black(),
        );
        // The richer fixtures (list rows, stacked text) resolve muted and
        // accent roles plus the caption/subheadline slots — install them so
        // a theme miss can't masquerade as a render failure.
        waterui::theme::install_color_signal::<waterui::theme::color::MutedForeground>(
            &mut env,
            black(),
        );
        waterui::theme::install_color_signal::<waterui::theme::color::Accent>(&mut env, black());
        waterui::theme::install_font_signal::<Body>(&mut env, binding(Body::DEFAULT).computed());
        waterui::theme::install_font_signal::<Caption>(
            &mut env,
            binding(Caption::DEFAULT).computed(),
        );
        waterui::theme::install_font_signal::<Subheadline>(
            &mut env,
            binding(Subheadline::DEFAULT).computed(),
        );
        env
    }

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

/// GPU-surface ownership regression coverage (#1725): a real mounted
/// `SceneView` — production `build_surface`, `SurfaceState`,
/// `SceneRenderer`/`SceneEngine` — driven through the failure drain and
/// the completion settlement seam. The ordering the GPU's own timing
/// cannot pin down is delivered by a direct call of the actual
/// `settle_frame_completion` body — reported as a deterministic seam
/// call, not a physical cadence or real GPU submission claim.
#[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
mod gpu_surface {
    use libtest_mimic::Trial;
    use waterui_apple::native_test_support::{gpu_surface::MountedSceneSurface, pump_main_until};

    use super::mtm;

    /// The registered trials — the completion/failure seam coverage for
    /// the mounted-surface ownership contract.
    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "gpu_surface::routed_failure_drains_idle_owner_once",
                routed_failure_drains_idle_owner_once,
            ),
            Trial::test(
                "gpu_surface::stale_completion_releases_only_its_lease",
                stale_completion_releases_only_its_lease,
            ),
        ]
    }

    /// A routed shared-generation failure reaches an owner that is idle
    /// — never attached, never presented, with a readiness waiter
    /// registered — through the production path: `ScenePart::note_failure`
    /// stores it and `RedrawHandle::request_redraw` enqueues
    /// `handle_redraw_request`, which drains it before the visibility,
    /// in-flight and external gates and settles through `settle_failed`:
    /// the owner is marked failed, the readiness waiter fires exactly
    /// once, the frame is owed, and the recovery watch on the failed
    /// generation arms. Re-producing the sealed generation answers the
    /// retained failure unchanged.
    pub fn routed_failure_drains_idle_owner_once() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted
            .install_scene_renderer()
            .map_err(|error| format!("the production renderer install: {error}"))?;
        let probe = mounted.readiness_probe();
        assert!(!mounted.owner_failed() && !mounted.frame_owed());

        // Route the failure: the generation seals, the participant's
        // owner wake lands on the main queue — the callback itself.
        mounted.fail_scene_generation();
        assert!(mounted.scene_generation_sealed());
        assert!(
            !mounted.owner_failed(),
            "the routed failure sits queued until the owner drains it"
        );

        assert!(
            pump_main_until(2.0, || mounted.owner_failed()),
            "the enqueued handle_redraw_request never drained the routed failure"
        );
        assert!(mounted.frame_owed(), "the failed frame stays owed");
        assert_eq!(
            probe.wakes(),
            1,
            "the registered readiness waiter settles exactly once"
        );
        assert!(
            mounted.context_watch_armed(),
            "the recovery watch arms on the failed generation"
        );
        assert!(
            mounted.sealed_produce_is_cached_error(),
            "the sealed generation never re-produces"
        );
        Ok(())
    }

    /// A stale completion after a genuinely newer publication: the
    /// retained old-generation submission releases only its own lease —
    /// `frame_owed`, `frame_in_flight` — and neither presents nor touches
    /// the newer epoch's readiness, failure flag or watch. The current
    /// generation still settles a completion normally.
    ///
    /// The seam is called directly for deterministic ordering — the real
    /// `settle_frame_completion` body, not a simulated path; no physical
    /// GPU submission/cadence is claimed.
    pub fn stale_completion_releases_only_its_lease() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted
            .install_scene_renderer()
            .map_err(|error| format!("the production renderer install: {error}"))?;
        let probe = mounted.readiness_probe();
        assert!(
            mounted.begin_in_flight_frame(),
            "the surface ring yields a real PendingFrame"
        );

        assert!(
            pollster::block_on(mounted.publish_newer_context()),
            "the runtime must publish a genuinely newer context generation"
        );

        // The stale submission's completion runs the real seam: obsolete
        // first — it releases its lease and owes the work, nothing more.
        mounted.settle_submitted_completion();
        assert!(
            !mounted.frame_in_flight(),
            "the stale completion released its PendingFrame lease"
        );
        assert!(mounted.frame_owed(), "the work is owed on the live epoch");
        assert!(
            !mounted.owner_failed(),
            "a stale completion never marks the newer epoch failed"
        );
        assert_eq!(
            probe.wakes(),
            0,
            "a stale completion never settles the newer epoch's readiness"
        );
        assert!(
            !mounted.context_watch_armed(),
            "a stale completion never installs a watch"
        );
        assert!(
            !mounted.frame_presented(),
            "a stale completion never presents"
        );

        // The current epoch still completes through the same seam.
        assert!(
            mounted.begin_in_flight_frame(),
            "the ring yields a fresh frame for the live epoch"
        );
        mounted.settle_current_completion();
        assert!(
            mounted.frame_presented(),
            "the live epoch presents and reports readiness"
        );
        assert_eq!(probe.wakes(), 1, "readiness resolves once");
        assert!(!mounted.owner_failed());
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

/// Mounted-owner lifetimes (#1575): a real dispatcher-mounted hierarchy —
/// `.size`/`padding`/`hstack`/`zstack` containers plus a lazy `ForEach`
/// membership — must release its owners, child guards and host views when
/// the mounted tree drops. Weak handles into the tree's views die after
/// queued work drains, post-owner writes cease naturally, and a remount
/// of the same view still measures, binds and recycles membership.
mod owner_lifetimes {
    use waterui::Identifiable;
    use waterui::component::lazy::Lazy;
    use waterui::prelude::*;
    use waterui::reactive::binding;
    use waterui::reactive::collection::List as ReactiveList;
    use waterui::views::ForEach;
    use waterui_core::layout::ProposalSize;

    use cocoa_ui::objc2_foundation::{NSDate, NSRunLoop};

    use super::{HostView, Label, MainThreadMarker, PlatformView, Retained, leaf, mtm, resolve};

    /// A stable-id row for the `ForEach` membership.
    #[derive(Clone, Copy, Identifiable)]
    struct Row {
        #[id]
        id: u64,
    }

    /// Drains queued main-queue work — binding flushes and enqueued drops
    /// alike land at a real run-loop boundary.
    fn pump() {
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.2));
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
}

/// `ScrollController` requests against the kit's real scroll surfaces —
/// the water-rs/waterui#1901 contract: a bare request jumps to its
/// target, `Animation::Default` plays the platform's smooth scroll, an
/// explicit animation moves the offset along its timing, and every
/// flight lands where the jump would have. A request issued mid-flight
/// takes over.
mod scroll {
    use std::time::Duration;

    use objc2::Message;
    use waterui::animation::Animation;
    use waterui::layout::frame::Frame;
    use waterui::layout::scroll::{ScrollController, scroll};
    use waterui::prelude::text;
    use waterui::reactive::{Binding, Signal, binding};
    use waterui_apple::contract::NativeLeaf;
    use waterui_core::layout::Point;

    use super::{
        AttachedWindow, Retained, animation_cases, assert_offset_eq, distinct_offsets,
        mount_and_order_front, pump_flight, resolve::render, trial_each,
    };

    #[cfg(target_os = "macos")]
    use cocoa_ui::appkit::ScrollView as ScrollSurface;
    #[cfg(target_os = "ios")]
    use cocoa_ui::uikit::ScrollView as ScrollSurface;

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
        let view = scroll(Frame::new(text("document")).height(2000.0))
            .scroll_controller(&controller)
            .report_offset(&offset);
        let leaf = render(view);
        let surface = leaf
            .view()
            .downcast_ref::<ScrollSurface>()
            .expect("a ScrollView renders the kit's scroll surface")
            .retain();
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

    /// An animated request moves the reported offset through
    /// intermediate values — every write lands on the model, so
    /// `report_offset` publishes the ramp — and ends on the target a
    /// jump would have taken.
    fn an_animation_scrolls_to_the_target(name: &'static str, animation: Animation) {
        let fixture = mounted();
        fixture
            .controller
            .animate_to(Point::new(0.0, 800.0), animation);
        assert!(
            fixture.surface.scroll_animation_in_flight(),
            "the {name} flight must be in progress after the request"
        );
        let samples = pump_flight(
            || fixture.surface.scroll_animation_in_flight(),
            || reported_y(&fixture),
        );
        assert!(
            distinct_offsets(&samples) >= 3,
            "the {name} flight must move the reported offset through intermediate values: {samples:?}"
        );
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
    /// target re-clamped to the final geometry.
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
            raw_target_y(&fixture, 1000.0).min(end_offset(&fixture)),
            "the flight lands on its re-clamped target",
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

    use objc2::Message;
    use waterui::ViewExt;
    use waterui::animation::Animation;
    use waterui::component::list::{List, ListItem};
    use waterui::layout::scroll::ScrollController;
    use waterui::prelude::text;
    use waterui_apple::contract::NativeLeaf;

    use super::{
        AttachedWindow, Retained, animation_cases, assert_offset_eq, distinct_offsets,
        mount_and_order_front, pump_flight, resolve::render, trial_each,
    };

    #[cfg(target_os = "macos")]
    use cocoa_ui::appkit::ListTableView as TableSurface;
    #[cfg(target_os = "ios")]
    use cocoa_ui::uikit::TableView as TableSurface;

    /// A list taller than the window by far — row targets past the fold.
    const ROWS: usize = 200;
    /// The row the cases aim at.
    const TARGET: usize = 40;

    /// A mounted list of `ROWS` rows, the controller that drives it, and
    /// the leaf and window that keep the wiring alive.
    struct Fixture {
        table: Retained<TableSurface>,
        controller: ScrollController<usize>,
        _leaf: NativeLeaf,
        _window: AttachedWindow,
    }

    /// The clip offset the jump's family lands `row` on — `rectOfRow`'s
    /// origin on `AppKit`; `rectForRowAtIndexPath`'s origin minus the
    /// adjusted top inset on `UIKit`. `TARGET` sits far from either end,
    /// where no clamp applies.
    fn row_top_offset(fixture: &Fixture, row: usize) -> f64 {
        #[cfg(target_os = "macos")]
        {
            fixture.table.rect_of_row(row).origin.y
        }
        #[cfg(target_os = "ios")]
        {
            use cocoa_ui::objc2_ui_kit::NSIndexPathUIKitAdditions;
            let index = cocoa_ui::objc2_foundation::NSIndexPath::indexPathForRow_inSection(
                row.cast_signed(),
                0,
            );
            fixture.table.rectForRowAtIndexPath(&index).origin.y
                - fixture.table.adjustedContentInset().top
        }
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

    /// The row body — identical one-line rows; the cases assert
    /// positions, not labels.
    fn row_item() -> ListItem {
        ListItem::new(text("row"))
    }

    /// A row taller than the rest — mixed heights keep the last row's
    /// offset an honest measure instead of a multiple of the first.
    fn tall_row_item() -> ListItem {
        ListItem::new(text("row").padding_with(24.0))
    }

    /// Mounts `rows` in a list driven by a fresh `usize` controller.
    fn mounted_with(rows: Vec<fn() -> ListItem>) -> Fixture {
        let controller = ScrollController::new(0usize);
        let view = List::content(rows).scroll_controller(&controller);
        let leaf = render(view);
        let table = leaf
            .view()
            .downcast_ref::<TableSurface>()
            .expect("a List renders the kit's table surface")
            .retain();
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

    /// An animated request moves the clip through intermediate offsets —
    /// the model follows each tick, so lazily built rows materialize as
    /// the flight passes them — and ends exactly where the jump lands.
    fn an_animation_scrolls_the_row_to_the_top(name: &'static str, animation: Animation) {
        let fixture = mounted();
        fixture.controller.animate_to(TARGET, animation);
        assert!(
            fixture.table.scroll_animation_in_flight(),
            "the {name} row flight must be in progress after the request"
        );
        let samples = pump_flight(
            || fixture.table.scroll_animation_in_flight(),
            || offset_y(&fixture),
        );
        assert!(
            distinct_offsets(&samples) >= 3,
            "the {name} row flight must move through intermediate offsets: {samples:?}"
        );
        assert_offset_eq(
            offset_y(&fixture),
            row_top_offset(&fixture, TARGET),
            "the row flight lands on the jump's position",
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
        /// Slack for sub-point rounding: `UIKit`'s own row scroll lands a
        /// fraction of a thousandth of a point short of the end its
        /// `contentSize` implies.
        const EPSILON: f64 = 1.0e-3;

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
    /// top re-clamped to the final geometry.
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
            row_top_offset(&fixture, TARGET).min(end_offset(&fixture)),
            "the row flight lands on its re-clamped target",
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
