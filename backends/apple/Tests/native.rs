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

// On Mac Catalyst the binary is spawned without a bundle: its embedded
// `Info.plist` declares `LSUIElement`, so the process arrives at the
// `Prohibited` activation policy and `UIKit`'s Mac backgrounding
// controller leaves the trials' scene-less windows alone — the same
// contract `preview_info_plist!` gives the preview binary. The iOS
// targets spawn with no `Info.plist` and keep it that way, so the
// section is emitted on Catalyst only.
#[cfg(catalyst)]
#[used]
#[unsafe(link_section = "__TEXT,__info_plist")]
static NATIVE_SPAWN_INFO_PLIST: [u8; include_bytes!("native.plist").len()] =
    *include_bytes!("native.plist");

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, Label};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HostView, Label};

fn main() {
    let mut args = Arguments::from_args();
    // `AppKit`/`UIKit` objects may only be built on the real main thread;
    // `run` executes sequentially in the calling thread at one thread.
    args.test_threads = Some(1);
    // The precondition the GPU suites rely on for their whole run: an
    // awake display — a sleeping one presents nothing (#3529). `--list`
    // mounts nothing and skips it.
    #[cfg(target_os = "macos")]
    let _display_awake = (!args.list).then(cocoa_ui::native_test::DisplayAwake::hold);
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
    // A Catalyst spawn has no `UIApplication`, no scene and no
    // `NSApplication`. A scene-less `UIWindow`'s unassociated scene reads
    // the `AppKit` delegate the moment the window is created, and its
    // `UINSApplicationDelegate` raises when no `NSApplication` exists —
    // so the process's `AppKit` side comes up at the `Prohibited` policy
    // before any trial mounts a window, the same bring-up the preview
    // entry performs. `--list` mounts nothing and skips it.
    #[cfg(catalyst)]
    let _background_application =
        (!args.list).then(|| cocoa_ui::uikit::catalyst::BackgroundApplication::bring_up(mtm()));
    libtest_mimic::run(&args, trials()).exit();
}

/// The `embedding::` mount-ownership trials — kept beside `trials` so the
/// registrars stay one-per-line.
#[cfg(all(target_os = "macos", feature = "native-test"))]
fn embedding_trials() -> Vec<Trial> {
    vec![
        Trial::test(
            "embedding::an_embedded_mount_leaves_the_hosts_window_and_menu_bar_alone",
            || {
                embedding::an_embedded_mount_leaves_the_hosts_window_and_menu_bar_alone();
                Ok(())
            },
        ),
        Trial::test(
            "embedding::each_host_views_appearance_drives_its_own_mounted_scheme",
            || {
                embedding::each_host_views_appearance_drives_its_own_mounted_scheme();
                Ok(())
            },
        ),
        Trial::test(
            "embedding::the_embedded_root_follows_the_host_bounds_and_detaches_on_drop",
            || {
                embedding::the_embedded_root_follows_the_host_bounds_and_detaches_on_drop();
                Ok(())
            },
        ),
        Trial::test(
            "embedding::a_swiftui_host_measures_and_updates_its_environment_without_remounting",
            || {
                embedding::a_swiftui_host_measures_and_updates_its_environment_without_remounting();
                Ok(())
            },
        ),
    ]
}

/// The macOS `navigation::` window-chrome trials — kept beside `trials`
/// so the registrars stay one-per-line.
#[cfg(all(target_os = "macos", feature = "native-test"))]
fn navigation_trials() -> Vec<Trial> {
    use waterui_apple::native_test_support::toolbar_reach;
    vec![
        Trial::test(
            "navigation::a_searchable_page_rests_its_field_in_the_titlebar_row",
            || {
                navigation::a_searchable_page_rests_its_field_in_the_titlebar_row();
                Ok(())
            },
        ),
        Trial::test(
            "navigation::a_hidden_tab_split_releases_the_toolbar_sidebar_space",
            || {
                navigation::a_hidden_tab_split_releases_the_toolbar_sidebar_space();
                Ok(())
            },
        ),
        Trial::test(
            "navigation::a_toolbar_tab_picker_uses_the_extra_large_metrics",
            || {
                navigation::a_toolbar_tab_picker_uses_the_extra_large_metrics();
                Ok(())
            },
        ),
        Trial::test(
            "navigation::toolbar_labels_default_to_icons_and_preserve_explicit_titles",
            || {
                navigation::toolbar_labels_default_to_icons_and_preserve_explicit_titles();
                Ok(())
            },
        ),
        Trial::test(
            "navigation::toolbar_reach::a_stack_page_list_runs_under_the_toolbar",
            || {
                toolbar_reach::a_stack_page_list_runs_under_the_toolbar(mtm());
                Ok(())
            },
        ),
        Trial::test(
            "navigation::toolbar_reach::a_row_scrolled_to_the_top_sits_below_the_titlebar",
            || {
                toolbar_reach::a_row_scrolled_to_the_top_sits_below_the_titlebar(mtm());
                Ok(())
            },
        ),
        Trial::test(
            "navigation::toolbar_reach::a_stack_page_text_stays_clear_of_the_toolbar",
            || {
                toolbar_reach::a_stack_page_text_stays_clear_of_the_toolbar(mtm());
                Ok(())
            },
        ),
        Trial::test(
            "navigation::toolbar_reach::a_tabbed_stack_page_list_runs_under_the_toolbar",
            || {
                toolbar_reach::a_tabbed_stack_page_list_runs_under_the_toolbar(mtm());
                Ok(())
            },
        ),
        Trial::test(
            "navigation::toolbar_reach::a_tabbed_stack_page_text_stays_clear_of_the_toolbar",
            || {
                toolbar_reach::a_tabbed_stack_page_text_stays_clear_of_the_toolbar(mtm());
                Ok(())
            },
        ),
        Trial::test(
            "navigation::toolbar_reach::the_example_inbox_list_runs_under_the_toolbar",
            || {
                toolbar_reach::the_example_inbox_list_runs_under_the_toolbar(mtm());
                Ok(())
            },
        ),
        Trial::test(
            "navigation::toolbar_reach::the_example_inbox_text_stays_clear_of_the_toolbar",
            || {
                toolbar_reach::the_example_inbox_text_stays_clear_of_the_toolbar(mtm());
                Ok(())
            },
        ),
    ]
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
            Trial::test("window::a_declared_window_realizes_its_declaration", || {
                window::a_declared_window_realizes_its_declaration(mtm());
                Ok(())
            }),
        ]);
        tests.extend(embedding_trials());
        tests.extend(navigation_trials());
        tests.extend([
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
            #[cfg(feature = "text_field")]
            Trial::test(
                "text_selection::a_selection_menu_command_edits_the_live_selection",
                || {
                    text_selection::a_selection_menu_command_edits_the_live_selection();
                    Ok(())
                },
            ),
            Trial::test("split::a_sidebar_list_takes_the_source_list_chrome", || {
                split::a_sidebar_list_takes_the_source_list_chrome();
                Ok(())
            }),
            Trial::test("split::a_plain_list_keeps_the_regular_chrome", || {
                split::a_plain_list_keeps_the_regular_chrome();
                Ok(())
            }),
        ]);
        tests.extend(list_ax::trials());
        tests.extend(gesture_ax::trials());
        tests.extend(a11y_extent::trials());
        tests.extend(gesture_events::trials());
        #[cfg(all(feature = "scale", feature = "rotation"))]
        tests.extend(transform_modifiers::trials());
        #[cfg(feature = "gpu_surface")]
        {
            tests.extend(gpu_surface::trials());
            tests.extend(filtered::trials());
            tests.extend(capture_colour::trials());
            tests.extend(capture_order::trials());
        }
    }
    #[cfg(target_os = "ios")]
    {
        tests.extend(tabs::trials());
        tests.extend(navigation::trials());
        tests.extend(controller_bounds::trials());
        tests.extend(safe_area::trials());
        tests.extend(offset::trials());
    }
    tests.extend(migration::trials());
    tests.extend(transition::trials());
    tests.extend(owner_lifetimes::trials());
    tests.extend(scroll::trials());
    tests.extend(list_scroll::trials());
    #[cfg(feature = "webview")]
    tests.extend(webview::trials());
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
        Trial::test("resolve::toggle_leaves_follow_the_stretch_rule", || {
            resolve::toggle_leaves_follow_the_stretch_rule();
            Ok(())
        }),
        Trial::test("resolve::stepper_leaves_follow_the_stretch_rule", || {
            resolve::stepper_leaves_follow_the_stretch_rule();
            Ok(())
        }),
        Trial::test("resolve::picker_leaves_follow_the_stretch_rule", || {
            resolve::picker_leaves_follow_the_stretch_rule();
            Ok(())
        }),
        Trial::test("resolve::toggle_rows_place_the_control_by_kind", || {
            resolve::toggle_rows_place_the_control_by_kind();
            Ok(())
        }),
        Trial::test("resolve::a_label_leaves_8pt_between_icon_and_title", || {
            resolve::a_label_leaves_8pt_between_icon_and_title();
            Ok(())
        }),
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
    use std::time::Duration;

    use cocoa_ui::display_link::FrameClock;
    use cocoa_ui::native_test::ManualScrollClock;
    use cocoa_ui::notification::{NotificationName, NotificationObserver, observe_object};
    use cocoa_ui::objc2_app_kit::{
        NSEvent, NSScrollView, NSScrollViewDidLiveScrollNotification,
        NSStandardKeyBindingResponding, NSView, NSWindow,
    };
    use cocoa_ui::objc2_foundation::{NSPoint, NSRect, NSSize};
    use cocoa_ui::{MainThreadMarker, Retained};
    use objc2::runtime::AnyObject;
    use objc2::{MainThreadOnly, define_class, msg_send};
    use objc2_core_graphics::{CGEvent, CGScrollEventUnit};
    use waterui_apple::native_test_support::{
        MAIN_QUEUE_DEADLINE, drain_main_queue, pump_main_until,
    };

    /// Sends `action` down `window`'s responder chain from its first
    /// responder, the delivery `interpretKeyEvents` performs once a key
    /// resolves to its binding. A key-down needs a key window to resolve,
    /// and macOS's cooperative activation never makes a non-interactively
    /// launched harness the active application, so the case starts at the
    /// action: from there the scroll is `AppKit`'s own.
    pub fn command(window: &NSWindow, action: objc2::runtime::Sel) {
        let responder = window
            .firstResponder()
            .expect("the window has a first responder");
        // SAFETY: `action` is a no-argument binding action; the chain's
        // `doCommandBySelector:` forwarding is the mechanism
        // `interpretKeyEvents` uses for resolved bindings.
        unsafe { responder.doCommandBySelector(action) };
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
    /// keep running through each. Each correction follows a stepped frame
    /// of the surface's manual clock, and the final frame reaches the
    /// schedule's end and lands the flight.
    pub fn assert_flight_survives_corrections(
        window: &NSWindow,
        clock: &ManualScrollClock,
        target: f64,
        in_flight: impl Fn() -> bool,
        offset: impl Fn() -> f64,
        grow_document: impl FnOnce(),
    ) {
        // The 2 s linear flight, stepped a frame at a time: a quarter-second
        // frame before each of the three corrections and one after the last,
        // all mid-schedule; the final frame reaches the schedule's end and
        // lands. Every instant is a dyadic fraction, so the elapsed time is
        // exact and the landing frame is the last one by construction.
        const STEP: Duration = Duration::from_millis(250);
        let content = window.contentView().expect("the window has a content view");
        let size = content.frame().size;
        let step = |when: &str| {
            let previous = offset().to_bits();
            clock.advance(STEP);
            assert!(
                in_flight(),
                "the flight must still be running {when}: offset {}",
                offset()
            );
            assert!(
                offset().to_bits() != previous,
                "the frame {when} must move the flight toward {target}: offset {}",
                offset()
            );
        };
        let settle = |what: &str| {
            content.layoutSubtreeIfNeeded();
            assert!(
                drain_main_queue(super::mtm()),
                "the main queue must drain after {what}"
            );
            assert!(
                in_flight(),
                "{what} must not cancel the flight: offset {}",
                offset()
            );
        };
        step("before the document grows");
        grow_document();
        settle("the document growing");
        step("before a larger window");
        window.setContentSize(NSSize::new(size.width + 120.0, size.height + 160.0));
        settle("a larger window");
        step("before a smaller window");
        window.setContentSize(NSSize::new(size.width - 120.0, size.height - 160.0));
        settle("a smaller window");
        step("after the last correction");
        clock.advance(Duration::from_secs(1));
        assert!(
            !in_flight(),
            "the frame at the schedule's end must land the flight: offset {}",
            offset()
        );
        assert!(!clock.is_running(), "the landed flight must stop its clock");
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
    use waterui::component::form::picker::{PickerItem, PickerStyle, picker};
    use waterui::component::form::secure::{Secure, SecureField};
    use waterui::component::label::label;
    use waterui::component::slider::slider;
    use waterui::component::stepper::stepper;
    use waterui::component::text_field::TextField;
    use waterui::component::toggle::Toggle;
    use waterui::filter::Opacity;
    use waterui::icon::SystemIcon;
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

    /// `docs/layout-spec.md` §3 and §6 over the style × label-visibility
    /// matrix: only a switch with a visible label is `Horizontal` and
    /// answers the proposed width; a checkbox and a hidden label answer the
    /// row's intrinsic size. `Automatic` is the platform's default first —
    /// a switch on iOS, a checkbox on macOS.
    pub fn toggle_leaves_follow_the_stretch_rule() {
        let on = binding(false);
        #[cfg(target_os = "ios")]
        let automatic = StretchAxis::Horizontal;
        #[cfg(target_os = "macos")]
        let automatic = StretchAxis::None;
        let cases = [
            (
                "switch",
                Toggle::new("Wi-Fi", &on).switch(),
                StretchAxis::Horizontal,
            ),
            (
                "hidden-label switch",
                Toggle::new("Wi-Fi", &on).switch().hide_label(),
                StretchAxis::None,
            ),
            (
                "checkbox",
                Toggle::new("Wi-Fi", &on).checkbox(),
                StretchAxis::None,
            ),
            (
                "hidden-label checkbox",
                Toggle::new("Wi-Fi", &on).checkbox().hide_label(),
                StretchAxis::None,
            ),
            ("automatic", Toggle::new("Wi-Fi", &on), automatic),
            (
                "hidden-label automatic",
                Toggle::new("Wi-Fi", &on).hide_label(),
                StretchAxis::None,
            ),
        ];
        for (name, toggle, axis) in cases {
            let leaf = render(toggle);
            assert_eq!(leaf.layout().stretch_axis(), axis, "{name}: stretch axis");
            let intrinsic = leaf.layout().measure(ProposalSize::UNSPECIFIED).size;
            assert!(
                intrinsic.width < 400.0,
                "{name}: intrinsic width {} must be narrower than the offer",
                intrinsic.width
            );
            let offered = leaf
                .layout()
                .measure(ProposalSize::new(Some(400.0), None))
                .size;
            let expected = if axis == StretchAxis::Horizontal {
                400.0_f32
            } else {
                intrinsic.width
            };
            assert_eq!(
                offered.width.to_bits(),
                expected.to_bits(),
                "{name}: width under a 400 pt offer"
            );
            assert_eq!(
                offered.height.to_bits(),
                intrinsic.height.to_bits(),
                "{name}: height stays intrinsic"
            );
        }
    }

    /// `docs/layout-spec.md` §3 and §6 over label visibility: a stepper is
    /// `Horizontal` and answers the proposed width only while its label is
    /// visible; a hidden label answers the buttons' intrinsic size
    /// (water-rs/waterui#2364).
    pub fn stepper_leaves_follow_the_stretch_rule() {
        let quantity = binding(3_i32);
        let cases = [
            (
                "visible label",
                stepper("Quantity", &quantity),
                StretchAxis::Horizontal,
            ),
            (
                "hidden label",
                stepper("Quantity", &quantity).hide_label(),
                StretchAxis::None,
            ),
        ];
        for (name, stepper, axis) in cases {
            let leaf = render(stepper);
            assert_eq!(leaf.layout().stretch_axis(), axis, "{name}: stretch axis");
            let intrinsic = leaf.layout().measure(ProposalSize::UNSPECIFIED).size;
            assert!(
                intrinsic.width < 400.0,
                "{name}: intrinsic width {} must be narrower than the offer",
                intrinsic.width
            );
            let offered = leaf
                .layout()
                .measure(ProposalSize::new(Some(400.0), None))
                .size;
            let expected = if axis == StretchAxis::Horizontal {
                400.0_f32
            } else {
                intrinsic.width
            };
            assert_eq!(
                offered.width.to_bits(),
                expected.to_bits(),
                "{name}: width under a 400 pt offer"
            );
            assert_eq!(
                offered.height.to_bits(),
                intrinsic.height.to_bits(),
                "{name}: height stays intrinsic"
            );
        }
    }

    /// `docs/layout-spec.md` §3 and §6: a segmented picker is `Horizontal`
    /// and answers the proposed width; every other style answers its
    /// intrinsic size.
    pub fn picker_leaves_follow_the_stretch_rule() {
        let selection = binding(0_i32);
        let items = || -> Vec<PickerItem<i32>> {
            vec![
                waterui::text!("One").tag(0),
                waterui::text!("Two").tag(1),
                waterui::text!("Three").tag(2),
            ]
        };
        let cases = [
            (
                "segmented",
                picker("Mode", items(), &selection).segmented(),
                StretchAxis::Horizontal,
            ),
            (
                "menu",
                picker("Mode", items(), &selection).style(PickerStyle::Menu),
                StretchAxis::None,
            ),
        ];
        for (name, picker, axis) in cases {
            let leaf = render(picker);
            assert_eq!(leaf.layout().stretch_axis(), axis, "{name}: stretch axis");
            let intrinsic = leaf.layout().measure(ProposalSize::UNSPECIFIED).size;
            assert!(
                intrinsic.width < 300.0,
                "{name}: intrinsic width {} must be narrower than the offer",
                intrinsic.width
            );
            let offered = leaf
                .layout()
                .measure(ProposalSize::new(Some(300.0), None))
                .size;
            let expected = if axis == StretchAxis::Horizontal {
                300.0_f32
            } else {
                intrinsic.width
            };
            assert_eq!(
                offered.width.to_bits(),
                expected.to_bits(),
                "{name}: width under a 300 pt offer"
            );
            assert_eq!(
                offered.height.to_bits(),
                intrinsic.height.to_bits(),
                "{name}: height stays intrinsic"
            );
        }
    }

    /// A switch row puts the label at the leading edge and the switch at
    /// the trailing edge of the full width; a checkbox row reads box first,
    /// then the label `LABEL_SPACING` after it.
    pub fn toggle_rows_place_the_control_by_kind() {
        #[cfg(target_os = "macos")]
        use cocoa_ui::appkit::toggle::LABEL_SPACING;
        #[cfg(target_os = "ios")]
        use cocoa_ui::uikit::toggle::LABEL_SPACING;

        // Auto Layout snaps frames to the backing scale's pixel grid.
        fn assert_near(actual: f64, expected: f64, what: &str, name: &str) {
            assert!(
                (actual - expected).abs() < 0.5,
                "{name}: {what} — expected {expected}, got {actual}"
            );
        }

        let on = binding(false);
        for (name, toggle, switch) in [
            ("switch", Toggle::new("Wi-Fi", &on).switch(), true),
            ("checkbox", Toggle::new("Wi-Fi", &on).checkbox(), false),
        ] {
            let leaf = render(toggle);
            let height = f64::from(leaf.layout().measure(ProposalSize::UNSPECIFIED).size.height);
            let row = leaf.view();
            cocoa_ui::view::set_frame(row, cocoa_ui::Rect::new(0.0, 0.0, 400.0, height));
            cocoa_ui::view::layout_immediately(row);
            let subviews = cocoa_ui::view::subviews(row);
            assert_eq!(
                subviews.len(),
                2,
                "{name}: the row holds the label and the control"
            );
            let label = cocoa_ui::view::frame(&subviews[0]);
            // The constraints pin the control's alignment rect, not its
            // frame: a UISwitch frame overshoots the pinned trailing edge
            // by 2 pt, so the edge Auto Layout placed is the alignment
            // rect's.
            let control = cocoa_ui::view::alignment_frame(&subviews[1]);
            let control_trailing = control.origin.x + control.size.width;
            if switch {
                assert_near(label.origin.x, 0.0_f64, "label leading", name);
                assert_near(
                    control_trailing,
                    400.0_f64,
                    "switch at the trailing edge",
                    name,
                );
            } else {
                assert_near(control.origin.x, 0.0_f64, "box leading", name);
                assert_near(
                    label.origin.x,
                    control_trailing + LABEL_SPACING,
                    "label after the box",
                    name,
                );
            }
        }
    }

    /// water-rs/waterui#1101: a semantic label leaves 8 pt between its
    /// icon and its title — the `SwiftUI` `Label` and Material 3 default
    /// the composition reproduces on every backend.
    pub fn a_label_leaves_8pt_between_icon_and_title() {
        let leaf = render(label("Play").icon(SystemIcon::new("play.fill")));
        let size = leaf.layout().measure(ProposalSize::UNSPECIFIED).size;
        let row = leaf.view();
        cocoa_ui::view::set_frame(
            row,
            cocoa_ui::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
        );
        cocoa_ui::view::layout_immediately(row);
        // The leaf's host view wraps the hstack's own host view, whose
        // subviews are the icon and the title in one coordinate space.
        let stack = cocoa_ui::view::subviews(row);
        assert_eq!(stack.len(), 1, "the label leaf hosts one stack view");
        let subviews = cocoa_ui::view::subviews(&stack[0]);
        assert_eq!(
            subviews.len(),
            2,
            "an icon label holds the icon and the title"
        );
        let icon = cocoa_ui::view::frame(&subviews[0]);
        let title = cocoa_ui::view::frame(&subviews[1]);
        // Auto Layout snaps frames to the backing scale's pixel grid.
        let gap = title.origin.x - (icon.origin.x + icon.size.width);
        assert!(
            (gap - 8.0).abs() < 0.5,
            "icon-to-title gap — expected 8 pt, got {gap}"
        );
    }
}

/// `Metadata<Offset>` on `UIKit`: the content moves visually through its
/// `transform`, and the wrapper's layout pass positions its bounds and
/// center. `UIKit` resolves `frame`/`center` writes under a non-identity
/// transform through the autoresizing-mask constraints it synthesizes for
/// a translated view — a re-layout can absorb part of the translation and
/// the content lands short of its declared offset (#2713).
#[cfg(target_os = "ios")]
mod offset {
    use cocoa_ui::view;
    use waterui::style::Offset;
    use waterui_apple::native_test_support::{mount_uikit, render_environment};
    use waterui_backend_core::AnyView;
    use waterui_core::Metadata;

    use super::mtm;

    fn t(name: &'static str, body: fn()) -> libtest_mimic::Trial {
        libtest_mimic::Trial::test(format!("offset::{name}"), move || {
            body();
            Ok(())
        })
    }

    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![t(
            "relayout_keeps_the_full_translation",
            relayout_keeps_the_full_translation,
        )]
    }

    /// Two layout passes must leave the declared translation whole and the
    /// content centered in the wrapper's untransformed space — the state the
    /// offset contract resolves to, whatever path the passes took there.
    fn relayout_keeps_the_full_translation() {
        let mount = mount_uikit(
            mtm(),
            AnyView::new(Metadata::new("x", Offset::new(-120.0, -160.0))),
            &render_environment(),
        );
        for _ in 0..2 {
            mount.window.layoutIfNeeded();
        }
        let wrapper = view::retain_base(mount.content.view());
        let content = &view::subviews(&wrapper)[0];
        let bounds = view::bounds(&wrapper);

        // The anchor sits at the wrapper's midpoint; the transform carries
        // the offset on top of it.
        let position = content.layer().position();
        assert_eq!(position.x, bounds.size.width / 2.0);
        assert_eq!(position.y, bounds.size.height / 2.0);

        let transform = content.transform();
        assert_eq!(transform.tx, -120.0);
        assert_eq!(transform.ty, -160.0);

        // A manually positioned, transform-moved child must not let the
        // autoresizing mask pin its frame — the pinned frame is the channel
        // the absorption came through.
        assert!(!content.translatesAutoresizingMaskIntoConstraints());
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
    ) -> (waterui::reactive::Binding<usize>, NativeLeaf) {
        let pane = binding(0usize);
        let leaf = resolve::render(
            Tabs::new(
                &pane,
                vec![
                    Tab::container(label("One"), || text("pane one")),
                    Tab::container(label("Two"), || text("pane two")),
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
        pane.set(1usize);
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
            let pane = binding(0usize);
            let leaf = resolve::render(
                Tabs::new(&pane, vec![Tab::container(label("One"), || text("one"))])
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
        let pane = binding(0usize);
        let leaf = resolve::render(Tabs::new(
            &pane,
            vec![
                Tab::container(label("One"), || {
                    scroll(vstack((text("one"), text("two"), text("three"))))
                }),
                Tab::container(label("Two"), || {
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
        let pane = binding(0usize);
        let leaf = resolve::render(Tabs::new(
            &pane,
            vec![
                Tab::container(label("One"), || scroll(text("scrollable"))),
                Tab::container(label("Two"), || text("static")),
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
        let pane = binding(0usize);
        // Render inside a bounded pool: the leaf's own `Retained`/`Rc`
        // ownership survives, while every autoreleased temporary the
        // mount produced drains now — the weak read later must not meet
        // a render-time retainer in the harness's outer pool.
        let leaf = autoreleasepool(|_| {
            resolve::render(Tabs::new(
                &pane,
                vec![Tab::container(label("One"), move || {
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
        let pane = binding(0usize);
        let leaf = resolve::render(Tabs::new(
            &pane,
            vec![Tab::container(label("One"), || {
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
        a_declared_window_realizes_its_declaration, manager_installs_into_the_environment,
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

/// Navigation-chrome trials on `AppKit` against a real titled `NSWindow`: a
/// `.searchable` page rests its field in the titlebar's bottom accessory
/// row at the platform's metrics, a `NavigationSplitView` in a hidden tab
/// releases the toolbar space its sidebar claims, and the toolbar tab
/// picker uses the system extra-large metrics — the same `navigation::`
/// namespace the `UIKit` cases own on iOS.
#[cfg(all(target_os = "macos", feature = "native-test"))]
mod navigation {
    use cocoa_ui::appkit::{Segment, SegmentedControl};
    use cocoa_ui::objc2_app_kit::{
        NSControlSize, NSLayoutAttribute, NSSearchField, NSSegmentDistribution, NSSegmentStyle,
        NSSegmentedControl, NSSplitView, NSSplitViewController, NSTrackingSeparatorToolbarItem,
    };
    use cocoa_ui::{PlatformView, Retained, view};
    use objc2::ClassType as _;
    use objc2::runtime::NSObjectProtocol as _;
    use waterui::Str;
    use waterui::navigation::{
        NavigationSplitView, NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement,
        NavigationView, Tab, Tabs,
    };
    use waterui::prelude::text;
    use waterui::reactive::binding;
    use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};

    use super::{leaf, mtm, resolve};

    /// The platform's `.searchable` on a titled window without a sidebar
    /// installs a bottom titlebar accessory row 38 pt tall, the field 28 pt
    /// tall at 41 % of the window's width, centred, drawn in the 11 pt
    /// system font. The assertions read the row's live `AppKit` objects so
    /// each metric — attribute, height, font, size, centring — is checked
    /// the way the platform's row was measured.
    pub fn a_searchable_page_rests_its_field_in_the_titlebar_row() {
        let mtm = mtm();
        let query = binding(Str::from(""));
        let page = resolve::render(
            NavigationView::new("Title", text("body")).searchable(&query, "Search mail"),
        );
        let window = leaf::attach(mtm, page.view());
        // The platform's reference window: the metrics were measured at an
        // 800 pt window.
        window.set_content_rect(cocoa_ui::Rect::new(0.0, 0.0, 800.0, 600.0), false);
        window.order_front();
        let native = window.native();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                native.titlebarAccessoryViewControllers().count() == 1
            }),
            "the searchable page must install exactly one titlebar accessory"
        );
        let accessory = native.titlebarAccessoryViewControllers().objectAtIndex(0);
        assert_eq!(
            accessory.layoutAttribute(),
            NSLayoutAttribute::Bottom,
            "the platform pins the search row to the titlebar's bottom"
        );
        accessory.view().layoutSubtreeIfNeeded();
        let row = accessory.view().frame();
        assert_eq!(
            row.size.height.to_bits(),
            38.0_f64.to_bits(),
            "the platform's search row is 38 pt tall"
        );
        let field = accessory
            .view()
            .subviews()
            .iter()
            .find_map(|sub| sub.downcast::<NSSearchField>().ok())
            .expect("the accessory row hosts an NSSearchField");
        assert_eq!(
            field
                .font()
                .expect("the field carries a font")
                .pointSize()
                .to_bits(),
            11.0_f64.to_bits(),
            "the platform's .searchable row draws the field at 11 pt"
        );
        let frame = field.frame();
        assert!(
            (frame.size.height - 28.0).abs() <= 1.0,
            "the platform's field is 28 pt tall, got {}",
            frame.size.height
        );
        let window_width = native.frame().size.width;
        let expected_width = 0.41 * window_width;
        assert!(
            (frame.size.width - expected_width).abs() <= 1.0,
            "the platform's field spans 41 % of the window's width, got {}",
            frame.size.width
        );
        let in_window = field.convertRect_toView(field.bounds(), None);
        let field_center_x = in_window.origin.x + in_window.size.width / 2.0;
        let window_center_x = window_width / 2.0;
        assert!(
            (field_center_x - window_center_x).abs() <= 1.0,
            "the platform centres the field in the window: field at {field_center_x}, window at {window_center_x}"
        );
    }

    /// The `NSSplitView` mounted anywhere under `root`, depth-first — the
    /// fixture mounts exactly one.
    fn split_view_in(root: &PlatformView) -> Option<Retained<NSSplitView>> {
        for sub in view::subviews(root) {
            if let Ok(split) = sub.clone().downcast::<NSSplitView>() {
                return Some(split);
            }
            if let Some(found) = split_view_in(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// The `NSSplitViewController` that owns `split_view`, found by walking
    /// the responder chain up from the view, the way `AppKit` dispatches
    /// the controller's own actions such as `toggleSidebar:`.
    fn split_controller(split_view: &NSSplitView) -> Retained<NSSplitViewController> {
        let mut chain = Vec::new();
        // SAFETY: `nextResponder` is an ordinary responder-chain read on
        // the main thread.
        let mut responder = unsafe { split_view.nextResponder() };
        while let Some(current) = responder {
            match current.downcast::<NSSplitViewController>() {
                Ok(controller) => return controller,
                Err(current) => {
                    chain.push(current.class().name().to_string_lossy().into_owned());
                    // SAFETY: as above.
                    responder = unsafe { current.nextResponder() };
                }
            }
        }
        panic!("no NSSplitViewController in the split view's responder chain: {chain:?}");
    }

    /// Whether the window toolbar carries the split's tracking separator —
    /// the sidebar chrome a visible split pane offers the window.
    fn has_tracking_separator(window: &cocoa_ui::appkit::Window) -> bool {
        window.native().toolbar().is_some_and(|toolbar| {
            toolbar
                .items()
                .iter()
                .any(|item| item.isKindOfClass(NSTrackingSeparatorToolbarItem::class()))
        })
    }

    /// Whether the window toolbar exists and has laid out at least one
    /// view-backed item — the point where item frames read real positions.
    fn toolbar_laid_out(window: &cocoa_ui::appkit::Window) -> bool {
        window.native().toolbar().is_some_and(|toolbar| {
            toolbar
                .items()
                .iter()
                .filter_map(|item| item.view())
                .any(|view| view.frame().size.width > 0.0)
        })
    }

    /// A `Tabs` whose selected pane is a `NavigationView` page carrying a
    /// `TopBarLeading` item, while the unselected pane holds a
    /// `NavigationSplitView`. An expanded `allowsFullHeightLayout` sidebar
    /// item claims the toolbar's leading region even while its view is
    /// hidden, so the pane collapses the item to release that space,
    /// expands it when the pane shows, and — after a user collapse —
    /// restores the user's own collapse state instead of forcing the
    /// sidebar open.
    pub fn a_hidden_tab_split_releases_the_toolbar_sidebar_space() {
        let tab = binding(0usize);
        let detail = binding(Some(1_i32));
        let leaf = resolve::render(Tabs::new(
            &tab,
            vec![
                Tab::new("Inbox", move || {
                    NavigationView::new("Inbox", text("inbox")).navigation_toolbar(
                        NavigationToolbar::new(vec![NavigationToolbarItem::action(
                            NavigationToolbarPlacement::TopBarLeading,
                            "Edit",
                            || {},
                        )]),
                    )
                }),
                Tab::container("Split", move || {
                    NavigationSplitView::new(&detail, text("Sidebar"), |id: i32| {
                        NavigationView::new(format!("Detail {id}"), text("detail"))
                    })
                }),
            ],
        ));
        let window = super::mount_and_order_front(leaf.view());
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || toolbar_laid_out(&window)),
            "the window toolbar laid out its items"
        );
        let content = window
            .native()
            .contentView()
            .expect("a titled window has a content view");
        let split_view = split_view_in(&content).expect("the split pane mounts an NSSplitView");
        let split = split_controller(&split_view);
        let sidebar = split
            .splitViewItems()
            .firstObject()
            .expect("the split's first item is the sidebar");

        // A hidden pane contributes no sidebar chrome and no hidden claim
        // on the titlebar either — collapsing the item is what releases
        // the space the expanded one reserves.
        assert!(
            !has_tracking_separator(&window),
            "a hidden split's toolbar carries no tracking separator"
        );
        assert!(
            sidebar.isCollapsed(),
            "the hidden split's sidebar item collapses to release its titlebar claim"
        );

        // The page's leading item hugs the window's leading edge: with the
        // claim left standing it sat right of the sidebar — the declared
        // default sidebar is 240-480 pt wide, so a pushed item lands past
        // 150.
        let item_view = window
            .native()
            .toolbar()
            .expect("the window toolbar")
            .items()
            .iter()
            .find_map(|item| item.view())
            .expect("the leading item hosts a view");
        let min_x = view::convert_rect(&item_view, view::bounds(&item_view), None)
            .origin
            .x;
        assert!(
            min_x < 150.0,
            "the leading toolbar item sits at the window's leading edge — minX {min_x}"
        );

        // Selecting the split's tab brings its chrome back: the sidebar
        // expands and the toolbar aligns to it with the tracking
        // separator.
        tab.set(1usize);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                !sidebar.isCollapsed() && has_tracking_separator(&window)
            }),
            "showing the pane expands the sidebar and aligns the toolbar to it"
        );

        // Collapse the sidebar the way the toolbar's own toggle does, then
        // cycle the pane away and back. The user's collapse is the state
        // to restore — the forced collapse while hidden must not
        // overwrite it, and the show must not force the sidebar open.
        // SAFETY: `toggleSidebar:` is the controller's documented sidebar
        // toggle; a nil sender on the main thread.
        unsafe { split.toggleSidebar(None) };
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || sidebar.isCollapsed()),
            "the user's collapse lands on the sidebar item"
        );
        tab.set(0usize);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || !has_tracking_separator(&window)),
            "hiding the pane withdraws the sidebar chrome again"
        );
        tab.set(1usize);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                sidebar.isCollapsed() && has_tracking_separator(&window)
            }),
            "the restored pane keeps the user's collapsed sidebar"
        );
    }

    /// The toolbar tab picker is the system one: an extra-large segmented
    /// control whose segments fill equally. Joining the unified toolbar
    /// narrows that control below the size it reports while free, and the
    /// item follows the intrinsic size rather than freezing the free one.
    /// The window stays unordered; the metrics were read that way.
    pub fn a_toolbar_tab_picker_uses_the_extra_large_metrics() {
        let mtm = mtm();
        let pane: waterui::Binding<usize> = binding(0usize);
        let labels = ["Inbox", "Library", "Gallery", "Settings"];
        let leaf = resolve::render(Tabs::new(
            &pane,
            labels
                .iter()
                .map(|label| {
                    let title = (*label).to_string();
                    Tab::new(*label, move || {
                        NavigationView::new(title.clone(), text(title.clone()))
                    })
                })
                .collect::<Vec<_>>(),
        ));
        let window = leaf::attach(mtm, leaf.view());
        window.set_content_rect(cocoa_ui::Rect::new(0.0, 0.0, 700.0, 520.0), false);

        let free = SegmentedControl::new(mtm);
        free.set_segments(
            &labels
                .iter()
                .map(|label| Segment {
                    label: label.to_string(),
                    enabled: true,
                    ..Segment::default()
                })
                .collect::<Vec<_>>(),
        );
        let free_size = free.fitting_size();

        let laid_out = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            toolbar_segmented(&window).is_some_and(|control| {
                let bounds = control.bounds().size;
                bounds.height > free_size.height - 0.5 && bounds.width + 8.0 < free_size.width
            })
        });
        assert!(
            laid_out,
            "the toolbar tab picker adopts the unified-toolbar metrics"
        );

        let control = toolbar_segmented(&window).expect("the toolbar hosts the tab picker");
        assert_eq!(control.controlSize(), NSControlSize::ExtraLarge);
        assert_eq!(
            control.segmentDistribution(),
            NSSegmentDistribution::FillEqually
        );
        assert_eq!(control.segmentStyle(), NSSegmentStyle::Automatic);
        let bounds = control.bounds().size;
        let intrinsic = control.intrinsicContentSize();
        assert!(
            (bounds.width - intrinsic.width).abs() < 0.5,
            "the item follows the intrinsic width {} rather than a frozen fitting size {}",
            intrinsic.width,
            bounds.width
        );
        assert!(
            (bounds.height - intrinsic.height).abs() < 0.5
                && (bounds.height - free_size.height).abs() < 0.5,
            "the picker is the extra-large height"
        );

        let frame = view::convert_rect(&control, view::bounds(&control), None);
        let center = frame.origin.x + frame.size.width / 2.0;
        assert!(
            (center - 350.0).abs() < 4.0,
            "the picker stays centred in the 700 pt window, centre {center}"
        );
    }

    /// A navigation page's toolbar labels present icon-only by default, so
    /// an icon action's hosted item is exactly its icon wide and the
    /// toolbar draws its capsule around the icon alone — while an explicit
    /// title-and-icon preference keeps its title.
    ///
    /// The icon is the one the item hosts: a macOS button label resolves
    /// `Body` at its control size's system font, so the same symbol rendered
    /// outside a button is drawn at a different size.
    pub fn toolbar_labels_default_to_icons_and_preserve_explicit_titles() {
        use cocoa_ui::appkit::first_symbol_view;
        use waterui::component::{ButtonStyle, button, label};
        use waterui::icon::SystemIcon;
        use waterui::navigation::NavigationStack;

        const LEADING: &str = "dev.cocoaui.navigation.leading";
        const TRAILING: &str = "dev.cocoaui.navigation.trailing";

        let icon = || SystemIcon::new("plus");
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Inbox", text("root")).navigation_toolbar(NavigationToolbar::new(
                vec![
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::TopBarLeading,
                        button(label("").icon(icon())).style(ButtonStyle::Borderless),
                    ),
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::TopBarTrailing,
                        button(label("Keep title").icon(icon()).title_and_icon())
                            .style(ButtonStyle::Borderless),
                    ),
                ],
            )),
        ));
        let window = leaf::attach(mtm(), leaf.view());
        let hosted_view = |identifier: &str| {
            window.native().toolbar().and_then(|toolbar| {
                toolbar
                    .items()
                    .iter()
                    .find(|item| item.itemIdentifier().to_string() == identifier)
                    .and_then(|item| item.view())
            })
        };
        let hosted_width =
            |identifier: &str| hosted_view(identifier).map(|view| view.frame().size.width);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                hosted_width(LEADING).is_some() && hosted_width(TRAILING).is_some()
            }),
            "the page offers both actions to the window toolbar"
        );
        let icon_width = hosted_view(LEADING)
            .and_then(|view| first_symbol_view(&view))
            .and_then(|symbol| symbol.image_size())
            .expect("the default action hosts its symbol")
            .width;
        let automatic = hosted_width(LEADING).expect("the default action is hosted");
        assert!(
            (automatic - icon_width).abs() < 0.5,
            "automatic toolbar label width {automatic} must equal icon width {icon_width} without empty-title spacing"
        );
        let explicit = hosted_width(TRAILING).expect("the explicit action is hosted");
        assert!(
            explicit > icon_width + 8.0,
            "an explicit title-and-icon preference keeps its title, got {explicit}"
        );
    }

    /// The segmented control the window toolbar is hosting, when it has one.
    fn toolbar_segmented(
        window: &cocoa_ui::appkit::Window,
    ) -> Option<Retained<NSSegmentedControl>> {
        window.native().toolbar().and_then(|toolbar| {
            toolbar.items().iter().find_map(|item| {
                item.view()
                    .and_then(|view| view.downcast::<NSSegmentedControl>().ok())
            })
        })
    }
}

/// A `NavigationSplitView` sidebar column is marked `SidebarContent`: the
/// lists inside take the source-list chrome — clear background over the
/// split's material and the unemphasised selection pill — while a list
/// outside the sidebar keeps the regular fill (water-rs/waterui#1495).
#[cfg(target_os = "macos")]
mod split {
    use cocoa_ui::appkit::ListTableView;
    use cocoa_ui::objc2_app_kit::NSTableViewStyle;
    use cocoa_ui::{PlatformView, view};
    use waterui::component::list::{List, ListItem};
    use waterui::navigation::{NavigationSplitView, NavigationView};
    use waterui::prelude::text;
    use waterui::reactive::binding;

    use super::resolve;

    /// The `ListTableView` inside `view`'s subtree, depth-first.
    fn sidebar_table(view: &PlatformView) -> Option<cocoa_ui::Retained<ListTableView>> {
        let mut stack = vec![view::retain_base(view)];
        while let Some(view) = stack.pop() {
            stack.extend(view::subviews(&view));
            if let Ok(table) = view.downcast::<ListTableView>() {
                return Some(table);
            }
        }
        None
    }

    /// The split's sidebar list is the one `set_sidebar` marks — the
    /// source-list style is what draws the grey selection pill.
    pub fn a_sidebar_list_takes_the_source_list_chrome() {
        let selection = binding(Some(0_i32));
        let leaf = resolve::render(NavigationSplitView::new(
            &selection,
            move || {
                List::content((|| ListItem::new(text("row")),)).selection(&binding(Some(0_usize)))
            },
            |_id: i32| NavigationView::new("Detail", text("detail")),
        ));
        let table = sidebar_table(leaf.view()).expect("the sidebar mounts a list");
        assert!(
            table.is_sidebar(),
            "the sidebar list draws source-list chrome"
        );
        assert_eq!(
            table.table_view().style(),
            NSTableViewStyle::SourceList,
            "the sidebar list takes the source-list style"
        );
    }

    /// The same list outside a split keeps the regular table chrome.
    pub fn a_plain_list_keeps_the_regular_chrome() {
        let selection = binding(Some(0_usize));
        let leaf =
            resolve::render(List::content((|| ListItem::new(text("row")),)).selection(&selection));
        let table = sidebar_table(leaf.view()).expect("the list mounts a table");
        assert!(
            !table.is_sidebar(),
            "a list outside a sidebar keeps the regular fill"
        );
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

    fn an_owed_first_frame_pauses_when_hidden() -> Result<(), libtest_mimic::Failed> {
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm()))
            .map_err(|error| format!("mount: {error}"))?;
        mounted.ensure_attached();
        let _ready = mounted.readiness_probe();
        assert!(!mounted.frame_presented(), "the first frame is still owed");
        assert!(
            !mounted.link_paused(),
            "the visible first frame is scheduled"
        );
        cocoa_ui::view::set_hidden(mounted.leaf.view(), true);
        assert!(mounted.link_paused(), "hiding pauses an owed first frame");
        cocoa_ui::view::set_hidden(mounted.leaf.view(), false);
        assert!(!mounted.link_paused(), "showing resumes the owed frame");
        Ok(())
    }

    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "gpu_surface::an_owed_first_frame_pauses_when_hidden",
                an_owed_first_frame_pauses_when_hidden,
            ),
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
                "gpu_surface::a_scope_end_after_teardown_settles_quietly",
                a_scope_end_after_teardown_settles_quietly,
            ),
            Trial::test(
                "gpu_surface::a_scope_end_beyond_the_orphaned_count_still_panics",
                a_scope_end_beyond_the_orphaned_count_still_panics,
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

    /// A capture's registration can owe the surface an `end` after the
    /// mount is gone — a frame still in flight settles on the main
    /// queue whenever the GPU finishes with it, which may follow the
    /// owner's teardown. Two open scopes deepen the count teardown
    /// orphans; both late ends then settle quietly instead of tripping
    /// the balance assert a live surface answers.
    pub fn a_scope_end_after_teardown_settles_quietly() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        mounted.begin_capture(Rc::new(|| {}));
        mounted.begin_capture(Rc::new(|| {}));
        // Unmounts mid-scope and delivers both owed ends — a panic here
        // is the pre-fix crash a late-settling registration produced.
        mounted.unmount_then_end_captures(2);
        Ok(())
    }

    /// The orphaned count keeps the balance honest: an end beyond what
    /// teardown recorded is still the unbalanced scope the assert
    /// names, not a quiet drop.
    pub fn a_scope_end_beyond_the_orphaned_count_still_panics() -> Result<(), libtest_mimic::Failed>
    {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        mounted.begin_capture(Rc::new(|| {}));
        // One scope was open at teardown; the second late end is owed
        // by nobody.
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                mounted.unmount_then_end_captures(2);
            }))
            .is_err(),
            "a scope end beyond the orphaned count still panics"
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
        use waterui_core::view_renderer::{RenderError, RenderRequest, ViewRenderer};
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
                RenderRequest::new(
                    waterui_core::layout::Size::new(64.0, 64.0),
                    std::time::Duration::from_secs(10),
                ),
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
        use waterui_core::view_renderer::{RenderRequest, ViewRenderer};

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
                RenderRequest::new(
                    waterui_core::layout::Size::new(64.0, 64.0),
                    std::time::Duration::from_secs(10),
                ),
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
    use cocoa_ui::objc2::rc::autoreleasepool;
    use libtest_mimic::Trial;
    use waterui_apple::native_test_support::ErrorLog;
    use waterui_apple::native_test_support::filtered::MountedFilteredSurface;
    use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};
    use waterui_graphics::gpu::runtime::HostedLayerError;

    use super::mtm;

    fn an_owed_first_frame_pauses_when_hidden() -> Result<(), libtest_mimic::Failed> {
        // The gated setup keeps the link paused while the window orders in,
        // so the real display link cannot take the owed first frame before
        // the trial samples it.
        let (mounted, gate) = pollster::block_on(MountedFilteredSurface::mount_gated(mtm()))
            .map_err(|error| format!("mount: {error}"))?;
        mounted.ensure_attached();
        let _ready = mounted.readiness_probe();
        assert!(
            mounted.link_paused(),
            "the pending effect setup holds the link"
        );
        gate.release();
        // The setup's main-queue landing opens the link; the link's first
        // update is a later run-loop source, so the pump samples the open
        // link before any delivery.
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || !mounted.link_paused()),
            "the visible first frame is scheduled once the setup lands"
        );
        assert!(!mounted.frame_presented(), "the first frame is still owed");
        cocoa_ui::view::set_hidden(mounted.leaf.view(), true);
        assert!(mounted.link_paused(), "hiding pauses an owed first frame");
        assert!(mounted.deliver_frame(1.0), "a queued drawable is available");
        assert!(
            !mounted.render_in_flight(),
            "hidden delivery starts no capture"
        );
        cocoa_ui::view::set_hidden(mounted.leaf.view(), false);
        assert!(!mounted.link_paused(), "showing resumes the owed frame");
        Ok(())
    }

    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "filtered::an_owed_first_frame_pauses_when_hidden",
                an_owed_first_frame_pauses_when_hidden,
            ),
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
            Trial::test(
                "filtered::unmount_releases_the_state_and_view",
                unmount_releases_the_state_and_view,
            ),
        ]
    }

    /// water-rs/waterui#1567: dropping the mounted leaf releases the
    /// `FilteredState` and its `HostView` while the shared runtime lives
    /// on — the layout, window, superview and backing handlers and the
    /// intrinsic measure the mount installed hold neither once pending
    /// submitted work has settled — and a remount on the same runtime
    /// presents again.
    ///
    /// The trials run inside one `applicationDidFinishLaunching:` callback
    /// whose autorelease pool never drains, so the mount, its presented
    /// frame and the unmount live inside a bounded `autoreleasepool` and
    /// the release pump reads its probes inside one too — only the `Weak`
    /// probes and the runtime escape, so a survivor names a real retainer,
    /// never a pooled temporary.
    pub fn unmount_releases_the_state_and_view() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let ((state, view), runtime) = autoreleasepool(|_| {
            let mounted = pollster::block_on(MountedFilteredSurface::mount(mtm))
                .map_err(|error| format!("a mounted filtered surface: {error}"))?;
            mounted.ensure_attached();
            assert!(
                pump_main_until(MAIN_QUEUE_DEADLINE, || mounted.frame_presented()),
                "the filtered view presents its first frame"
            );
            let probes = mounted.release_probes();
            Ok::<_, String>((probes, mounted.unmount()))
        })?;

        let released = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            autoreleasepool(|_| state.upgrade().is_none() && view.load().is_none())
        });
        assert!(
            released,
            "the unmounted leaf's state and host view release with the shared runtime alive \
             (FilteredState strong count {}, HostView alive: {})",
            state.strong_count(),
            autoreleasepool(|_| view.load().is_some()),
        );

        autoreleasepool(|_| {
            let remounted = MountedFilteredSurface::mount_shared(mtm, runtime)
                .map_err(|error| format!("a remounted filtered surface: {error}"))?;
            remounted.ensure_attached();
            assert!(
                pump_main_until(MAIN_QUEUE_DEADLINE, || remounted.frame_presented()),
                "a remounted filtered view presents again on the shared runtime"
            );
            Ok::<_, String>(())
        })?;
        Ok(())
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
        // A redraw the scene child requested during the flight legitimately
        // resumes the link for one recapture through the `Deferred` arm —
        // the contract is the settled state, not the first sampled instant.
        // `paused && nothing in flight` is unreachable inside that resumed
        // window, and unreachable entirely if the link recaptured at display
        // rate: under that bug the pump times out instead of passing.
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.link_paused()
                && !filtered.render_in_flight()),
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

/// #3528: a `FilteredView` whose chain declines the native realizations
/// composites through `capture.rs` — the compositor once stored every
/// source's sampled values unconverted, so an identity chain's half-float
/// linear-P3 output read back darker and hue-shifted. The trial captures
/// the same column rendered plain and under the identity chain and
/// measures the difference with the oracle's LDR-FLIP.
#[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
mod capture_colour {
    use std::rc::Rc;

    use cocoa_ui::objc2::rc::autoreleasepool;
    use cocoa_ui::view;
    use libtest_mimic::Trial;
    use waterui_apple::native_test_support::{capture_view_rgba, pump_main_until};

    use super::transition::{gpu_leaf, gpu_picture};
    use super::{HostView, PlatformView, Retained, mtm};

    /// The columns' pinned edge: `200×200` points each.
    const COLUMN_EDGE: f64 = 200.0;

    /// The gap between the two columns' leading edges.
    const COLUMN_GAP: f64 = 20.0;

    /// The `capture_colour::` trials.
    pub fn trials() -> Vec<Trial> {
        vec![Trial::test(
            "capture_colour::a_capture_realized_identity_filter_preserves_colour",
            || {
                a_capture_realized_identity_filter_preserves_colour();
                Ok(())
            },
        )]
    }

    /// The asymmetric marker rectangle, (40,20)–(140,120) in the
    /// column's flipped `200×200` space.
    fn marker_rect() -> waterui::layout::PinConstraints {
        waterui::layout::PinConstraints::new()
            .leading(40.0)
            .top(20.0)
            .width(100.0)
            .height(100.0)
    }

    /// One `200×200` column: a `MeshGradient` GPU surface at the marker
    /// rectangle, a half-transparent magenta native rectangle over it,
    /// an opaque black notch at the marker's top-left corner and an
    /// asymmetric label — GPU content, translucent and opaque native
    /// raster, and text in one column, so the comparison covers every
    /// source kind the capture compositor converts.
    fn column_subtree() -> impl waterui_backend_core::View {
        use waterui::Color;
        use waterui::ViewExt;
        use waterui::graphics::color::Srgb;
        use waterui::layout::{PinConstraints, PositionExt, absolute};
        use waterui::prelude::text;
        absolute((
            gpu_picture().pin(marker_rect()),
            Color::new(Srgb::new(1.0, 0.0, 1.0))
                .opacity(0.5)
                .pin(marker_rect()),
            Color::new(Srgb::new(0.0, 0.0, 0.0)).pin(
                PinConstraints::new()
                    .leading(40.0)
                    .top(20.0)
                    .width(20.0)
                    .height(20.0),
            ),
            text("Fg 3528 \u{2191}").pin(PinConstraints::new().leading(10.0).top(140.0)),
        ))
    }

    /// The capturable surfaces resolved at `root`'s first depth, with
    /// the retained views that carry them — the GPU child's kit
    /// `SurfaceView` and the `HostView` the filtered output resolves on.
    pub fn capturables(
        root: &PlatformView,
    ) -> Vec<(
        Retained<PlatformView>,
        Rc<dyn cocoa_ui::capture::CapturableSurface>,
    )> {
        let mut entries = Vec::new();
        cocoa_ui::capture::for_each_capturable(root, &mut |surface_view, surface| {
            entries.push((view::retain_base(surface_view), Rc::clone(surface)));
        });
        entries
    }

    /// The outermost `200×200` views of `root`'s subtree ordered by their
    /// leading edge in `root`'s coordinates: `index` 0 is the uncaptured
    /// column, 1 the filtered one. Only size and order identify the
    /// columns — their absolute origin is the window's — and `f64` frame
    /// values compare inside a half-point band.
    pub fn column(root: &PlatformView, index: usize) -> Retained<PlatformView> {
        let mut columns = Vec::new();
        let mut queue = std::collections::VecDeque::from([view::retain_base(root)]);
        while let Some(candidate) = queue.pop_front() {
            let frame = view::convert_rect(&candidate, view::bounds(&candidate), Some(root));
            if (frame.size.width - COLUMN_EDGE).abs() < 0.5
                && (frame.size.height - COLUMN_EDGE).abs() < 0.5
            {
                columns.push((frame.origin.x, candidate));
            } else {
                queue.extend(view::subviews(&candidate));
            }
        }
        columns.sort_by(|a, b| a.0.total_cmp(&b.0));
        columns
            .into_iter()
            .nth(index)
            .unwrap_or_else(|| panic!("no 200×200 column {index}"))
            .1
    }

    /// A capture's premultiplied sRGB-encoded RGBA8 texels as the
    /// oracle's premultiplied linear-P3 interchange image.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the oracle's f32 interchange format deliberately narrows its f64 colour maths"
    )]
    pub fn linear_p3(texels: &[u8], width: u32, height: u32) -> cherenkov_oracle::F32Image {
        use cherenkov_oracle::color::{linear_srgb_to_linear_p3, srgb_decode};
        let pixels = texels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let alpha = f64::from(p[3]) / 255.0;
                if alpha == 0.0 {
                    return [0.0; 4];
                }
                let [r, g, b] = linear_srgb_to_linear_p3(
                    [p[0], p[1], p[2]].map(|c| srgb_decode(f64::from(c) / 255.0 / alpha)),
                );
                [r * alpha, g * alpha, b * alpha, alpha].map(|c| c as f32)
            })
            .collect();
        cherenkov_oracle::F32Image {
            width,
            height,
            pixels,
        }
    }

    /// `a_capture_realized_identity_filter_preserves_colour` — #3528's
    /// reproduction: the same `200×200` column uncaptured at x=0 and under
    /// a capture-realized identity chain (`Brightness(0.0)` with a
    /// declared `OutputSize::MatchInput`, which `native_filter::plan`
    /// declines) at x=220. Both columns read back through
    /// `capture_view_rgba` — the `capture_rgba` path the issue's numbers
    /// came from — and must match under the oracle's same-pixels limits.
    pub fn a_capture_realized_identity_filter_preserves_colour() {
        use waterui::graphics::filtrate::filters::Brightness;
        use waterui::graphics::{AnyEffect, FilteredView, OutputSize, ParamGuards};
        use waterui::layout::{PinConstraints, PositionExt, absolute};
        use waterui_backend_core::AnyView;

        let _mtm = mtm();
        let column_pin = |leading: f64| {
            PinConstraints::new()
                .leading(leading)
                .top(0.0)
                .width(COLUMN_EDGE)
                .height(COLUMN_EDGE)
        };
        let filtered = FilteredView {
            content: AnyView::new(column_subtree()),
            effect: AnyEffect::filter(Brightness(0.0_f32)),
            guards: ParamGuards::default(),
        }
        .output_size(OutputSize::MatchInput);
        let leaf = gpu_leaf(absolute((
            column_subtree().pin(column_pin(0.0)),
            filtered.pin(column_pin(COLUMN_EDGE + COLUMN_GAP)),
        )));
        let _window = super::mount_and_order_front(leaf.view());
        let root = leaf.view();

        assert!(
            pump_main_until(10.0, || {
                autoreleasepool(|_| {
                    let entries = capturables(root);
                    entries.len() == 2
                        && entries
                            .iter()
                            .all(|(_, surface)| surface.has_presented_frame())
                })
            }),
            "the reference GPU surface and the filtered output both present"
        );

        // Realization check: the capture realization hides its unfiltered
        // content view and never arms `layerUsesCoreImageFilters` — a
        // `CIFilter` realization keeps the content visible on a Core
        // Image-filtered layer, so these assertions pin the case to the
        // path under test.
        let filtered_host = capturables(root)
            .into_iter()
            .find_map(|(host, _)| host.downcast_ref::<HostView>().is_some().then_some(host))
            .expect("the filtered output resolves on its host view");
        assert!(
            view::subviews(&filtered_host)
                .iter()
                .any(|child| view::is_hidden(child)),
            "the capture realization hides its unfiltered content"
        );
        assert!(
            !filtered_host.layerUsesCoreImageFilters(),
            "the identity chain took the capture realization, not the CIFilter one"
        );

        assert_columns_match(root);
    }

    /// Captures both `200×200` columns of `root` through
    /// `capture_view_rgba` and asserts the oracle's same-pixels equivalence
    /// between them. The unconverted composite #3528 fixed scored a mean
    /// FLIP of 0.34 here; the converting one scores 0.0002.
    fn assert_columns_match(root: &PlatformView) {
        let [reference, filtered] = [0, 1].map(|index| {
            let capture = capture_view_rgba(&column(root, index));
            linear_p3(&capture.pixels, capture.width, capture.height)
        });
        cherenkov_oracle::metrics::assert_images_match(&reference, &filtered);
    }
}

/// #1771: a GPU surface keeps its place in the native tree inside a
/// capture. The compositor once drew every resolved surface first and the
/// whole native raster over it, so an opaque native ancestor background
/// hid the GPU child and a translucent ancestor's group opacity never
/// reached it. Each trial captures the same `200×200` column twice — at
/// x=0 with the GPU child replaced by a native view of the same solid
/// colour, at x=220 with the real GPU child — and asserts the oracle's
/// same-pixels equivalence between the two captures.
#[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
mod capture_order {
    use cocoa_ui::objc2::rc::autoreleasepool;
    use libtest_mimic::Trial;
    use waterui::Color;
    use waterui::ViewExt;
    use waterui::graphics::color::{Srgb, Working, WorkingColor};
    use waterui::layout::{PinConstraints, PositionExt, absolute};
    use waterui::prelude::text;
    use waterui_apple::native_test_support::{capture_view_rgba, pump_main_until};
    use waterui_backend_core::{AnyView, View};

    use super::capture_colour::{capturables, column, linear_p3};
    use super::mtm;
    use super::transition::gpu_leaf;

    /// The columns' pinned edge: `200×200` points each.
    const COLUMN_EDGE: f64 = 200.0;

    /// The gap between the two columns' leading edges.
    const COLUMN_GAP: f64 = 20.0;

    /// The GPU child's solid fill — the working colour both the GPU
    /// surface and its native stand-in paint.
    const CHILD: WorkingColor = WorkingColor::new([0.05, 0.25, 0.9, 1.0]);

    /// The `capture_order::` trials.
    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "capture_order::an_opaque_ancestor_background_keeps_its_gpu_child_and_foreground",
                || {
                    an_opaque_ancestor_background_keeps_its_gpu_child_and_foreground();
                    Ok(())
                },
            ),
            Trial::test(
                "capture_order::a_translucent_ancestor_composites_its_gpu_child_in_order",
                || {
                    a_translucent_ancestor_composites_its_gpu_child_in_order();
                    Ok(())
                },
            ),
            Trial::test(
                "capture_order::a_clipped_scaled_gpu_child_keeps_its_place",
                || {
                    a_clipped_scaled_gpu_child_keeps_its_place();
                    Ok(())
                },
            ),
            Trial::test(
                "capture_order::a_nested_filter_keeps_its_gpu_child_in_order",
                || {
                    a_nested_filter_keeps_its_gpu_child_in_order();
                    Ok(())
                },
            ),
        ]
    }

    /// The GPU child: a `MeshGradient` whose four corners are all
    /// [`CHILD`] — a GPU surface of one solid colour.
    fn gpu_child() -> AnyView {
        use waterui::graphics::gradient::MeshGradient;
        use waterui::reactive::constant;
        AnyView::new(MeshGradient::new(2, 2, constant(vec![CHILD; 4])))
    }

    /// The GPU child's native stand-in: a [`CHILD`] colour view.
    fn native_child() -> AnyView {
        AnyView::new(Color::new(Working(CHILD)))
    }

    /// An opaque orange — the ancestor background that hid the child.
    fn orange() -> Color {
        Color::new(Srgb::new(1.0, 0.55, 0.0))
    }

    /// `child` under native foreground inside the column: the child at
    /// (40,30)–(160,130), a half-transparent magenta strip across its
    /// lower part and past both its sides, and an asymmetric label over
    /// its top edge.
    fn layered(child: AnyView) -> impl View {
        absolute((
            child.pin(
                PinConstraints::new()
                    .leading(40.0)
                    .top(30.0)
                    .width(120.0)
                    .height(100.0),
            ),
            Color::new(Srgb::new(1.0, 0.0, 1.0)).opacity(0.5).pin(
                PinConstraints::new()
                    .leading(20.0)
                    .top(90.0)
                    .width(160.0)
                    .height(30.0),
            ),
            text("GPU 1771 \u{2191}").pin(PinConstraints::new().leading(50.0).top(40.0)),
        ))
    }

    /// Mounts the all-native `reference` column at x=0 and the GPU
    /// `subject` column at x=220, waits until the `surfaces` capturable
    /// surfaces the tree resolves have presented, captures both columns
    /// through `capture_view_rgba` and asserts the oracle's same-pixels
    /// equivalence of the subject to the reference.
    fn assert_gpu_column_matches_native(reference: impl View, subject: impl View, surfaces: usize) {
        let _mtm = mtm();
        let pin = |leading: f64| {
            PinConstraints::new()
                .leading(leading)
                .top(0.0)
                .width(COLUMN_EDGE)
                .height(COLUMN_EDGE)
        };
        let leaf = gpu_leaf(absolute((
            reference.pin(pin(0.0)),
            subject.pin(pin(COLUMN_EDGE + COLUMN_GAP)),
        )));
        let _window = super::mount_and_order_front(leaf.view());
        let root = leaf.view();
        assert!(
            pump_main_until(10.0, || {
                autoreleasepool(|_| {
                    let entries = capturables(root);
                    entries.len() == surfaces
                        && entries
                            .iter()
                            .all(|(_, surface)| surface.has_presented_frame())
                })
            }),
            "the GPU column's {surfaces} capturable surface(s) present"
        );
        let [reference, subject] = [0, 1].map(|index| {
            let capture = capture_view_rgba(&column(root, index));
            linear_p3(&capture.pixels, capture.width, capture.height)
        });
        cherenkov_oracle::metrics::assert_images_match(&reference, &subject);
    }

    /// An opaque ancestor background under the GPU child, native
    /// foreground over it: the child shows between the two.
    pub fn an_opaque_ancestor_background_keeps_its_gpu_child_and_foreground() {
        let column = |child| layered(child).background(orange());
        assert_gpu_column_matches_native(column(native_child()), column(gpu_child()), 1);
    }

    /// A half-transparent ancestor background under the GPU child, inside
    /// an ancestor at 0.7 group opacity: the child takes the group's
    /// opacity and covers the background.
    pub fn a_translucent_ancestor_composites_its_gpu_child_in_order() {
        let column = |child| {
            layered(child)
                .background(orange().opacity(0.5))
                .opacity(0.7)
        };
        assert_gpu_column_matches_native(column(native_child()), column(gpu_child()), 1);
    }

    /// The opaque-background column scaled 1.6× about its centre — a
    /// sublayer transform on `AppKit`, which view geometry does not see —
    /// and clipped to a rounded rectangle: the child spans
    /// (4,-12)–(196,148), so the column's top edge and rounded top-left
    /// corner cut it.
    pub fn a_clipped_scaled_gpu_child_keeps_its_place() {
        use waterui::shape::RoundedRectangle;
        let column = |child| {
            layered(child)
                .background(orange())
                .scale(1.6, 1.6)
                .clip(RoundedRectangle::new(0.2))
        };
        assert_gpu_column_matches_native(column(native_child()), column(gpu_child()), 1);
    }

    /// The opaque-background column at 0.8 group opacity inside a
    /// capture-realized identity filter (`Brightness(0.0)` with a declared
    /// `OutputSize::MatchInput`, which `native_filter::plan` declines):
    /// the filter's own capture composites the GPU child it owns, and the
    /// outer capture composites the filter's output. The reference column
    /// is the same native column unfiltered.
    pub fn a_nested_filter_keeps_its_gpu_child_in_order() {
        use waterui::graphics::filtrate::filters::Brightness;
        use waterui::graphics::{AnyEffect, FilteredView, OutputSize, ParamGuards};
        let content = |child| layered(child).background(orange()).opacity(0.8);
        // The reference column is unfiltered: the identity chain
        // reproduces it (#3528), so only the GPU column runs the nested
        // capture — the filtered output is its one resolved surface.
        let filtered = FilteredView {
            content: AnyView::new(content(gpu_child())),
            effect: AnyEffect::filter(Brightness(0.0_f32)),
            guards: ParamGuards::default(),
        }
        .output_size(OutputSize::MatchInput);
        assert_gpu_column_matches_native(content(native_child()), filtered, 1);
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
        UIKeyboardWillShowNotification, UILabel, UINavigationBar, UINavigationController,
        UIScrollView, UITabBar, UITextView, UIView,
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
            t(
                "a_stacks_only_window_seeds_its_region_and_replaces_above_the_band",
                a_stacks_only_window_seeds_its_region_and_replaces_above_the_band,
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

    /// The first `UIKit`-owned label showing `text` — how a title bar
    /// item is found: the bar draws the item's word itself, in a private
    /// button inside the floating bar container under the host, so the
    /// label carrying it is the item's only public surface. `WaterUI`'s own
    /// `Label`s are skipped so a hosted copy of the word never matches.
    fn find_bar_title(view: &PlatformView, text: &str) -> Option<Retained<PlatformView>> {
        for sub in view::subviews(view) {
            if sub.downcast_ref::<Label>().is_none()
                && let Some(label) = sub.downcast_ref::<UILabel>()
                && label.text().map(|t| t.to_string()).as_deref() == Some(text)
            {
                return Some(sub);
            }
            if let Some(found) = find_bar_title(&sub, text) {
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
        // The wrapper's own pass re-derived its fill: the moved
        // container's bottom edge now sits inside the keyboard band, so
        // the fill releases through it to the window's bottom edge — a
        // stale fill would still end at the old container bottom.
        assert!(
            (window_bottom(&fill) - mount.window.bounds().size.height).abs() <= TOLERANCE,
            "the wrapper's fill re-extended to the window's bottom edge — {:?}",
            window_frame(&fill),
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

    /// A window with no kit scroll surface still seeds its keyboard
    /// region: a `window_keyboard` read resolves the guide — here, the
    /// frame a resolved guide would report — and a non-zero seed marks
    /// the window's readers once, so the next pass re-places the
    /// stacks-only content above the band. A spawned simulator
    /// window's guide never resolves at all, so the trial asserts the
    /// unresolved contract on the real reads and feeds the resolved
    /// frame through the same function the read applies.
    fn a_stacks_only_window_seeds_its_region_and_replaces_above_the_band() {
        let draft = binding(Str::from(""));
        // Stacks only — a spacer over a composer row; no scroll
        // surface anywhere in the tree.
        let mount = mount(vstack((
            spacer(),
            hstack((
                field("Message", &draft),
                button("Send").action(move || draft.set(Str::from(""))),
            ))
            .padding_with(12.0),
        )));
        let field = find_view::<cocoa_ui::uikit::TextField>(&mount.host)
            .expect("the composer mounts a text field");
        let resting = window_bottom(&field);
        assert!(
            resting > 700.0,
            "precondition: the composer sits at the window's bottom — {resting}",
        );

        // The real guide read: a spawned simulator window's guide
        // never resolves — every `window_keyboard` read logs
        // `resolved = false` — so the region must stay unseeded. A
        // real app window's guide does resolve (the device capture):
        // that branch asserts its rest position seeded no keyboard.
        let guide_frame = mount.window.keyboardLayoutGuide().layoutFrame();
        if guide_frame.size.width <= 0.0 {
            assert!(
                !keyboard::seeded(&mount.window),
                "an unresolved guide must not seed",
            );
        } else {
            assert!(
                keyboard::seeded(&mount.window),
                "a resolved guide seeds on a mount pass",
            );
        }
        assert_eq!(
            region_frame(&mount.host),
            CGRect::ZERO,
            "no docked keyboard seeds nothing",
        );

        // The frame the guide would report for a docked keyboard.
        let keyboard = keyboard_end(&mount.window);
        keyboard::feed_guide_frame(&mount.window, keyboard);
        assert_eq!(
            region_frame(&mount.host),
            keyboard,
            "the feed seeds the window's region",
        );
        let band_top = keyboard.origin.y;
        let above = pump_main_until(MAIN_QUEUE_DEADLINE, || {
            mount.window.layoutIfNeeded();
            window_bottom(&field) <= band_top + TOLERANCE
        });
        assert!(
            above,
            "the seeded region re-placed the stacks content above the band — field {:?}",
            window_frame(&field),
        );
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
        let selection = binding(0usize);
        let mount = mount(Tabs::new(
            &selection,
            vec![Tab::new("Messages", move || {
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
        // not a `UIToolbar`, and a text action is the bar's own title
        // item — the word the bar draws is the item's proof.
        let item = find_bar_title(&mount.host, "New").expect("the page mounts its bar item");
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
        let selection = binding(0usize);
        let mount = mount(Tabs::new(
            &selection,
            vec![Tab::container("Home", move || {
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
        // not a `UIToolbar`, and a text action is the bar's own title
        // item — the word the bar draws is the item's proof.
        let item = find_bar_title(&mount.host, "New").expect("the page mounts its bar item");
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
        let selection = binding(0usize);
        let inner_selection = binding(0usize);
        let mount = mount(Tabs::new(
            &selection,
            vec![
                Tab::new("Outer", move || {
                    NavigationView::new(
                        "Outer",
                        vstack((
                            Tabs::new(
                                &inner_selection,
                                vec![
                                    Tab::new("Inner A", move || {
                                        NavigationView::new("A", text("a"))
                                    }),
                                    Tab::new("Inner B", move || {
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
                Tab::new("Other", move || NavigationView::new("Other", text("other"))),
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
        let mut items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }]);
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
    /// animation retires, and stepping the flight's clock past its
    /// original schedule proves the retired flight silent — the offset
    /// the wheel produced stands.
    #[cfg(target_os = "macos")]
    fn a_users_wheel_scroll_takes_over_an_in_flight_animation() {
        use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};

        let fixture = mounted();
        let clock = fixture.surface.install_manual_scroll_clock();
        let (_observer, live_scrolls) =
            super::appkit_input::count_live_scrolls(super::mtm(), &fixture.surface);
        fixture.controller.animate_to(
            Point::new(0.0, 1000.0),
            Animation::linear(Duration::from_secs(2)),
        );
        clock.advance(Duration::from_millis(500));
        assert!(
            fixture.surface.scroll_animation_in_flight(),
            "the flight must be in progress when the user's scroll starts"
        );
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
        assert!(
            !clock.is_running(),
            "the retired flight must stop its clock"
        );
        let wheeled = fixture.surface.content_offset().y;
        clock.advance(Duration::from_secs(2));
        assert_offset_eq(
            fixture.surface.content_offset().y,
            wheeled,
            "the retired flight writes nothing past its original schedule",
        );
        let settled = fixture.surface.content_offset().y;
        assert!(
            settled > before && (settled - 1000.0).abs() > 1.0,
            "the wheel's scroll stands — the retired flight never rewrites the model: \
             before {before}, settled {settled}"
        );
    }

    /// The keyboard wins mid-flight: page-down's `pageDown:`, sent up the
    /// responder chain from a focused view in the document, reaches the
    /// scroll view, whose clip write is not the flight's own — the
    /// animation retires synchronously, and stepping the flight's clock
    /// past its original schedule proves the retired flight silent.
    #[cfg(target_os = "macos")]
    fn a_page_down_takes_over_an_in_flight_animation() {
        use cocoa_ui::objc2_app_kit::NSResponder;
        use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};

        let fixture = mounted();
        let clock = fixture.surface.install_manual_scroll_clock();
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
        clock.advance(Duration::from_millis(500));
        let before = fixture.surface.content_offset().y;
        assert!(
            fixture.surface.scroll_animation_in_flight() && before > 0.0,
            "the flight must be mid-way when page-down is pressed: offset {before}"
        );
        super::appkit_input::command(&window, objc2::sel!(pageDown:));
        assert!(
            !fixture.surface.scroll_animation_in_flight(),
            "the keyboard's page-down must retire the flight"
        );
        assert!(
            !clock.is_running(),
            "the retired flight must stop its clock"
        );
        let pressed = fixture.surface.content_offset().y;
        clock.advance(Duration::from_secs(2));
        assert_offset_eq(
            fixture.surface.content_offset().y,
            pressed,
            "the retired flight writes nothing past its original schedule",
        );
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || {
                fixture.surface.content_offset().y > before
            }),
            "AppKit's animated page scroll must move the document past {before}"
        );
        let paged = fixture.surface.content_offset().y;
        assert!(
            (paged - 1000.0).abs() > 1.0,
            "the retired flight never landed: {paged}"
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
        let clock = fixture.surface.install_manual_scroll_clock();
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
            &clock,
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

    /// The keyboard wins mid-flight: the key's `action`, sent up the
    /// responder chain from the focused table, scrolls the clip through a write that is not the
    /// flight's own — `pageDown:` on the scroll view, or the table's own
    /// `scrollToEndOfDocument:`, which posts no live-scroll notification —
    /// and the flight retires synchronously; stepping the flight's clock
    /// past its original schedule proves the retired flight silent.
    #[cfg(target_os = "macos")]
    fn the_keyboard_takes_over_an_in_flight_animation(
        name: &'static str,
        action: objc2::runtime::Sel,
    ) {
        use cocoa_ui::objc2_app_kit::NSResponder;
        use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};

        let fixture = mounted();
        let clock = fixture.table.install_manual_scroll_clock();
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
        clock.advance(Duration::from_millis(500));
        assert!(
            fixture.table.scroll_animation_in_flight(),
            "the row flight must be in progress when {name} is pressed"
        );
        let before = offset_y(&fixture);
        super::appkit_input::command(&window, action);
        assert!(
            !fixture.table.scroll_animation_in_flight(),
            "{name} must retire the flight"
        );
        assert!(
            !clock.is_running(),
            "the retired flight must stop its clock"
        );
        let pressed = offset_y(&fixture);
        clock.advance(Duration::from_secs(2));
        assert_offset_eq(
            offset_y(&fixture),
            pressed,
            "the retired flight writes nothing past its original schedule",
        );
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || offset_y(&fixture) > before),
            "{name} must scroll the list past {before}"
        );
        let keyed = offset_y(&fixture);
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
        let clock = fixture.table.install_manual_scroll_clock();
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
            &clock,
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
                        objc2::sel!(pageDown:),
                    );
                }) as Box<dyn FnOnce() + Send>,
            ),
            (
                "list_scroll::the_end_key_takes_over_an_in_flight_animation".to_owned(),
                Box::new(|| {
                    the_keyboard_takes_over_an_in_flight_animation(
                        "End",
                        objc2::sel!(scrollToEndOfDocument:),
                    );
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
    use cocoa_ui::objc2_ui_kit::{
        UIBarButtonItem, UINavigationController, UIView, UIViewController,
    };
    use cocoa_ui::uikit::view_controller::owning_controller;
    use cocoa_ui::uikit::{NavContentController, NavPage};
    use cocoa_ui::{PlatformView, Retained, view};
    use waterui::Str;
    use waterui::ViewExt as _;
    use waterui::component::toggle::toggle;
    use waterui::component::{ButtonStyle, button};
    use waterui::icon::SystemIcon;
    use waterui::navigation::{
        NavigationStack, NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement,
        NavigationView, ToolbarItemIcon,
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
            libtest_mimic::Trial::test(
                "navigation::toolbar_labels_default_to_icons_and_preserve_explicit_titles",
                || {
                    toolbar_labels_default_to_icons_and_preserve_explicit_titles();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test("navigation::a_text_toolbar_button_draws_its_title", || {
                a_text_toolbar_button_draws_its_title();
                Ok(())
            }),
            libtest_mimic::Trial::test(
                "navigation::a_text_toolbar_item_follows_its_button_disabled_state",
                || {
                    a_text_toolbar_item_follows_its_button_disabled_state();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "navigation::a_symbol_toolbar_item_follows_its_button_disabled_state",
                || {
                    a_symbol_toolbar_item_follows_its_button_disabled_state();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "navigation::a_checkbox_toolbar_item_follows_its_disabled_state",
                || {
                    a_checkbox_toolbar_item_follows_its_disabled_state();
                    Ok(())
                },
            ),
        ]
    }

    /// The bar item's title as a plain string.
    fn bar_item_title(item: &UIBarButtonItem) -> String {
        item.title()
            .map(|text| text.to_string())
            .unwrap_or_default()
    }

    /// Toolbar context hides automatic titles without overriding explicit label style.
    fn toolbar_labels_default_to_icons_and_preserve_explicit_titles() {
        use waterui::component::label;
        use waterui_core::layout::ProposalSize;

        let icon = || SystemIcon::new("plus");
        let icon_leaf = resolve::render(icon());
        let icon_width = f64::from(
            icon_leaf
                .layout()
                .measure(ProposalSize::UNSPECIFIED)
                .size
                .width,
        );
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Inbox", text("root")).navigation_toolbar(NavigationToolbar::new(
                vec![
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::TopBarLeading,
                        button(label("").icon(icon())).style(ButtonStyle::Borderless),
                    ),
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::TopBarTrailing,
                        button(label("Keep title").icon(icon()).title_and_icon())
                            .style(ButtonStyle::Borderless),
                    ),
                ],
            )),
        ));
        let nav = nav_controller_in(leaf.view()).expect("the stack mounts a navigation controller");
        let item = nav
            .topViewController()
            .expect("the root page")
            .navigationItem();
        let leading = item
            .leftBarButtonItems()
            .expect("the default action")
            .objectAtIndex(0);
        let automatic = leading.customView().expect("the icon label remains hosted");
        let actual = view::bounds(&automatic).size.width;
        assert!(
            (actual - icon_width).abs() < 0.5,
            "automatic toolbar label width {actual} must equal icon width {icon_width} without empty-title spacing"
        );
        let trailing = item
            .rightBarButtonItems()
            .expect("the explicit action")
            .objectAtIndex(0);
        let explicit = trailing
            .customView()
            .expect("the explicit title-and-icon remains hosted");
        assert!(
            view::bounds(&explicit).size.width > icon_width + 8.0,
            "an explicit title-and-icon preference keeps its title"
        );
    }

    /// A leading text button is the bar's own title item, and the title
    /// follows the label. Hosting the button would hand the bar a `UIButton`
    /// whose configuration carries no title, which the bar draws as an empty
    /// capsule. Plain text beside it stays a hosted view.
    fn a_text_toolbar_button_draws_its_title() {
        let title = binding(Str::from("Edit"));
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Inbox", text("root")).navigation_toolbar(NavigationToolbar::new(
                vec![
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::TopBarLeading,
                        button(waterui::text!("{title}")).style(ButtonStyle::Plain),
                    ),
                    NavigationToolbarItem::new(
                        NavigationToolbarPlacement::TopBarTrailing,
                        text("12 unread"),
                    ),
                ],
            )),
        ));
        let nav =
            nav_controller_in(leaf.view()).expect("the stack mounts a UINavigationController");
        let top = nav.topViewController().expect("the stack has a root page");
        let leading = top
            .navigationItem()
            .leftBarButtonItems()
            .expect("the leading text button is a bar item");
        assert_eq!(leading.count(), 1, "the root page has one leading item");
        let leading = leading.objectAtIndex(0);
        assert_eq!(bar_item_title(&leading), "Edit");
        assert!(
            leading.customView().is_none(),
            "the word is the bar item's title, not a hosted view"
        );

        let trailing = top
            .navigationItem()
            .rightBarButtonItems()
            .expect("the trailing caption is a bar item");
        assert!(
            trailing.objectAtIndex(0).customView().is_some(),
            "plain text stays a hosted view"
        );

        title.set(Str::from("Done"));
        assert_eq!(bar_item_title(&leading), "Done");
    }

    /// A leading text button's bar item follows the button's `disabled`
    /// state: `sendActions` ignores `isEnabled`, so the bar item is what
    /// the bar disables — it must start disabled and re-enable with the
    /// button.
    fn a_text_toolbar_item_follows_its_button_disabled_state() {
        let disabled = binding(true);
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Inbox", text("root")).navigation_toolbar(NavigationToolbar::new(
                vec![NavigationToolbarItem::new(
                    NavigationToolbarPlacement::TopBarLeading,
                    button("Save")
                        .style(ButtonStyle::Plain)
                        .disabled(disabled.clone()),
                )],
            )),
        ));
        let nav =
            nav_controller_in(leaf.view()).expect("the stack mounts a UINavigationController");
        let top = nav.topViewController().expect("the stack has a root page");
        let leading = top
            .navigationItem()
            .leftBarButtonItems()
            .expect("the leading text button is a bar item")
            .objectAtIndex(0);
        assert!(
            !leading.isEnabled(),
            "the bar item starts disabled with the button"
        );
        disabled.set(false);
        assert!(
            leading.isEnabled(),
            "the bar item follows the button back to enabled"
        );
    }

    /// A trailing symbol item follows the same rule: the bar-drawn item
    /// tracks the content button's `disabled` state.
    fn a_symbol_toolbar_item_follows_its_button_disabled_state() {
        let disabled = binding(true);
        let mut item = NavigationToolbarItem::new(
            NavigationToolbarPlacement::TopBarTrailing,
            button("Save")
                .style(ButtonStyle::Plain)
                .disabled(disabled.clone()),
        );
        item.icon = Some(ToolbarItemIcon::System(SystemIcon::new("checkmark")));
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Inbox", text("root"))
                .navigation_toolbar(NavigationToolbar::new(vec![item])),
        ));
        let nav =
            nav_controller_in(leaf.view()).expect("the stack mounts a UINavigationController");
        let top = nav.topViewController().expect("the stack has a root page");
        let trailing = top
            .navigationItem()
            .rightBarButtonItems()
            .expect("the trailing symbol item is a bar item")
            .objectAtIndex(0);
        assert!(
            !trailing.isEnabled(),
            "the bar item starts disabled with the button"
        );
        disabled.set(false);
        assert!(
            trailing.isEnabled(),
            "the bar item follows the button back to enabled"
        );
    }

    /// The enabled mirror does not depend on the content being a `button`:
    /// a checkbox toggle's control is a `UIButton` in the leaf's subtree,
    /// so the item's icon follows its `disabled` state like any other.
    fn a_checkbox_toolbar_item_follows_its_disabled_state() {
        let disabled = binding(true);
        let checked = binding(false);
        let mut item = NavigationToolbarItem::new(
            NavigationToolbarPlacement::TopBarTrailing,
            toggle("Done", &checked)
                .checkbox()
                .disabled(disabled.clone()),
        );
        item.icon = Some(ToolbarItemIcon::System(SystemIcon::new("checkmark")));
        let leaf = resolve::render(NavigationStack::new(
            NavigationView::new("Inbox", text("root"))
                .navigation_toolbar(NavigationToolbar::new(vec![item])),
        ));
        let nav =
            nav_controller_in(leaf.view()).expect("the stack mounts a UINavigationController");
        let top = nav.topViewController().expect("the stack has a root page");
        let trailing = top
            .navigationItem()
            .rightBarButtonItems()
            .expect("the trailing icon item is a bar item")
            .objectAtIndex(0);
        assert!(
            !trailing.isEnabled(),
            "the bar item starts disabled with the checkbox"
        );
        disabled.set(false);
        assert!(
            trailing.isEnabled(),
            "the bar item follows the checkbox back to enabled"
        );
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

/// The web view leaf's real-engine cases.
///
/// Bridge admission is decided inside `WebKit` at navigation time, so only
/// a genuine `WKWebView` proves a non-admitted document never receives the
/// bridge — a mock of the handle's own bookkeeping would merely restate it.
#[cfg(feature = "webview")]
mod webview {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use libtest_mimic::Trial;
    use waterui_apple::native_test_support::{block_on_main, pump_main_until};
    use waterui_webview::{
        AnyWebViewHandle, BackendEvent, BridgeOrigins, JsReply, OriginPolicy, ScriptInjectionTime,
        Url, UserScript, WatcherGuard, WebViewConfig, WebViewEvent,
    };

    use super::mtm;

    /// The bound every wait on the real engine gets: a navigation, an
    /// evaluation, or a whole conformance scenario.
    ///
    /// Each of these crosses `WebKit`'s web content, GPU and network
    /// processes, and a view's first navigation launches them. On a freshly
    /// created simulator — which CI provisions for every run — that launch
    /// competes with the device's own first-boot work and with the suite's
    /// other test processes, so it takes seconds to tens of seconds — not
    /// the few run-loop turns the main-queue drain bound
    /// [`MAIN_QUEUE_DEADLINE`](waterui_apple::native_test_support::MAIN_QUEUE_DEADLINE)
    /// gives one deferred piece of work. Every wait still ends on the
    /// engine's own completion signal; this only bounds how long a stuck
    /// engine is given.
    const ENGINE_DEADLINE: f64 = 180.0;

    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "webview::a_non_admitted_document_gets_no_bridge_globals",
                || {
                    a_non_admitted_document_gets_no_bridge_globals();
                    Ok(())
                },
            ),
            Trial::test("webview::raw_evaluation_answers_json", || {
                raw_evaluation_answers_json();
                Ok(())
            }),
            Trial::test(
                "webview::mirrored_state_reaches_only_admitted_documents",
                || {
                    mirrored_state_reaches_only_admitted_documents();
                    Ok(())
                },
            ),
        ]
    }

    /// What a view reported about its loads: every event, in order, and how
    /// many loads finished. The guard keeps the record subscribed.
    struct Loads {
        finished: Rc<Cell<u32>>,
        events: Rc<RefCell<Vec<String>>>,
        _guard: WatcherGuard,
    }

    impl Loads {
        fn watch(handle: &AnyWebViewHandle) -> Self {
            let finished = Rc::new(Cell::new(0_u32));
            let events = Rc::new(RefCell::new(Vec::new()));
            let guard = handle.watch({
                let finished = Rc::clone(&finished);
                let events = Rc::clone(&events);
                move |event| {
                    if matches!(event, BackendEvent::Event(WebViewEvent::Loaded)) {
                        finished.set(finished.get() + 1);
                    }
                    events.borrow_mut().push(format!("{event:?}"));
                }
            });
            Self {
                finished,
                events,
                _guard: guard,
            }
        }
    }

    /// Navigates to `destination` and waits for the engine to report the
    /// load finished — delegate callbacks arrive on the main run loop,
    /// which `pump_main_until` turns. A load that fails, or is cancelled by
    /// a navigation policy, reports an error event; that ends the wait, and
    /// the failure names every event the view reported.
    fn navigate(handle: &AnyWebViewHandle, destination: &str, loads: &Loads) {
        let before = loads.finished.get();
        let reported = loads.events.borrow().len();
        let failed = || {
            loads.events.borrow()[reported..]
                .iter()
                .any(|event| event.contains("Error"))
        };
        let url: Url = destination
            .parse()
            .unwrap_or_else(|error| panic!("{destination} must parse: {error}"));
        handle.go_to(&url);
        let settled = pump_main_until(ENGINE_DEADLINE, || {
            loads.finished.get() > before || failed()
        });
        assert!(
            settled && !failed(),
            "the engine never finished loading {destination}; it reported {:?}",
            &loads.events.borrow()[reported..]
        );
    }

    /// The `typeof` of every global the bridge installs, evaluated in the
    /// document currently loaded — `waterui`, the `__wateruiEval` wrapper
    /// and the `__wateruiState` store from the document-start script, plus
    /// the `__wateruiProbe` the case itself injects, riding the creation-time
    /// scripts the mirrored-state seed is one of. The probe answers an
    /// array, whose reply is its JSON encoding on every engine.
    fn bridge_globals(handle: &AnyWebViewHandle) -> Vec<String> {
        let globals = block_on_main(
            ENGINE_DEADLINE,
            handle.run_javascript(
                "[typeof waterui, typeof __wateruiEval, typeof __wateruiState, typeof __wateruiProbe]",
            ),
        )
        .unwrap_or_else(|error| panic!("the globals probe must evaluate: {error}"));
        serde_json::from_str(&globals).unwrap_or_else(|error| {
            panic!("the globals probe answered {globals}, not a string array: {error}")
        })
    }

    /// A view created under `origins` with the probe script and a handler,
    /// and the record of its loads.
    fn probed_view(origins: BridgeOrigins) -> (AnyWebViewHandle, Loads) {
        let initial: Url = "https://waterui.dev".parse().expect("parses");
        let webview = waterui_apple::native_test_support::webview::controller(mtm()).open_with(
            WebViewConfig::default()
                .with_bridge_origins(OriginPolicy::new(origins, &initial))
                .with_scripts(vec![UserScript::new(
                    "waterui:test-probe",
                    "globalThis.__wateruiProbe = true;",
                    ScriptInjectionTime::DocumentStart,
                )]),
        );
        let handle = webview.handle().clone();
        let loads = Loads::watch(&handle);

        handle.add_handler(
            "noop",
            Box::new(|_| -> waterui_webview::HandlerFuture {
                Box::pin(async { Ok(JsReply::Json(b"null".to_vec())) })
            }),
        );
        (handle, loads)
    }

    /// A document outside the admission policy must receive none of the
    /// bridge: no `waterui` object, no `__wateruiEval`, no `__wateruiState`,
    /// and none of the creation-time scripts the state seed is one of.
    ///
    /// The first view is created under `BridgeOrigins::Any` so the globals
    /// are expected present — without it, a build that simply never
    /// installed anything would pass too. Both views load `data:` documents,
    /// so the policy is the only difference between them. Neither is
    /// `about:blank`: a fresh view's first `about:blank` navigation reuses
    /// the initial empty document, which runs no document-start script.
    fn a_non_admitted_document_gets_no_bridge_globals() {
        let (admitting, loads) = probed_view(BridgeOrigins::Any);
        navigate(&admitting, "data:text/html,admitted", &loads);
        assert_eq!(
            bridge_globals(&admitting),
            ["object", "function", "object", "boolean"],
            "an admitted document must receive the whole bridge"
        );

        // The default policy — only the origin the view was opened at —
        // admits nothing a `data:` document could report, so the document
        // must be left with no bridge at all.
        let (refusing, loads) = probed_view(BridgeOrigins::Initial);
        navigate(&refusing, "data:text/plain,elsewhere", &loads);
        assert_eq!(
            bridge_globals(&refusing),
            ["undefined", "undefined", "undefined", "undefined"],
            "a document outside the admission policy must get no bridge globals"
        );
    }

    /// `run_javascript` answers the JSON encoding of the evaluated value —
    /// a string quoted, a boolean `true`/`false`, `undefined` as `null` —
    /// checked against a real `WKWebView` through the shared conformance
    /// case, on a document that has loaded.
    fn raw_evaluation_answers_json() {
        let webview = waterui_apple::native_test_support::webview::open(mtm());
        let handle = webview.handle().clone();
        let loads = Loads::watch(&handle);
        navigate(&handle, "data:text/html,conformance", &loads);
        block_on_main(
            ENGINE_DEADLINE,
            waterui_webview::conformance::raw_evaluation_answers_json(async |script| {
                handle.run_javascript(script).await
            }),
        );
    }

    /// Mirrored state and bridge replies reach only the documents the
    /// admission policy admits — the shared conformance case, run on two
    /// real `WKWebView`s opened through the leaf's own controller.
    ///
    /// The case's handler futures and mirrored-state flushes run on the
    /// local executor, which on this backend lands on the main dispatch
    /// queue; `block_on_main` keeps the run loop turning so they advance
    /// while it drives the case's future.
    fn mirrored_state_reaches_only_admitted_documents() {
        // The local executor is process-global: `initialize_process` installs
        // a monitored one when the GPU-surface fixtures run, and this install
        // is a no-op then; otherwise the plain main-queue executor answers
        // the same contract.
        let _ = executor_core::try_init_local_executor(
            native_executor::NativeMainExecutor::new()
                .expect("the native suite runs on the process's main thread"),
        );
        let controller = waterui_apple::native_test_support::webview::controller(mtm());
        block_on_main(
            ENGINE_DEADLINE,
            waterui_webview::conformance::mirrored_state_reaches_only_admitted_documents(
                &controller,
            ),
        );
    }
}

/// `AppKit` accessibility coverage for the list — regression tests for
/// water-rs/waterui#1489. The table vends an `NSTableRow` proxy for every
/// row, and an unrealized row's cell child materializes the row when an
/// assistive client asks it for children, so the rows the table exposes
/// through the generic tree must stay bounded to the realized ones.
#[cfg(target_os = "macos")]
mod list_ax {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSAccessibilityChildrenAttribute, NSTableView};
    use objc2_foundation::{NSArray, NSIndexSet};
    use waterui::component::list::ListItem;
    use waterui::layout::scroll::ScrollController;
    use waterui_apple::native_test_support::{row_item, row_list};

    use super::{AttachedWindow, Retained, leaf, mtm, trial_each};

    /// `examples/edge_list`'s row count — the list the issue's tree walk
    /// timed out on.
    const ROWS: usize = 200_000;

    /// The slack the table's non-row children take: `AppKit` adds a column
    /// element and, where present, header chrome beside the rows.
    const NON_ROW_CHILDREN: usize = 16;

    /// The rows the table has materialized so far — `rowViewAtRow:` with
    /// `makeIfNecessary:NO` reads the realized set without touching it.
    fn realized_rows(table: &NSTableView) -> usize {
        (0..table.numberOfRows())
            .filter(|row| table.rowViewAtRow_makeIfNecessary(*row, false).is_some())
            .count()
    }

    /// The row elements `accessibilityRows` answers.
    fn ax_rows(table: &NSTableView) -> Retained<NSArray> {
        // SAFETY: the query answers an object array.
        let rows: Option<Retained<NSArray>> = unsafe { msg_send![table, accessibilityRows] };
        rows.unwrap_or_else(|| NSArray::from_retained_slice(&[]))
    }

    /// The elements `accessibilityChildren` answers.
    fn ax_children(table: &NSTableView) -> Retained<NSArray> {
        // SAFETY: the query answers an object array.
        let children: Option<Retained<NSArray>> =
            unsafe { msg_send![table, accessibilityChildren] };
        children.unwrap_or_else(|| NSArray::from_retained_slice(&[]))
    }

    /// The elements `accessibilitySelectedRows` answers.
    fn ax_selected_rows(table: &NSTableView) -> Retained<NSArray> {
        // SAFETY: the query answers an object array.
        let rows: Option<Retained<NSArray>> =
            unsafe { msg_send![table, accessibilitySelectedRows] };
        rows.unwrap_or_else(|| NSArray::from_retained_slice(&[]))
    }

    /// A laid-out 200k-row list, its leaf and window alive but never
    /// ordered in.
    struct Fixture {
        table: Retained<NSTableView>,
        _leaf: waterui_apple::contract::NativeLeaf,
        _window: AttachedWindow,
    }

    fn mounted() -> Fixture {
        let controller = ScrollController::new(0usize);
        let (leaf, surface) = row_list(vec![row_item as fn() -> ListItem; ROWS], &controller);
        let window = leaf::attach(mtm(), leaf.view());
        leaf.view().setNeedsLayout(true);
        leaf.view().layoutSubtreeIfNeeded();
        Fixture {
            table: surface.table_view(),
            _leaf: leaf,
            _window: window,
        }
    }

    /// A laid-out 200k-row list must expose only realized rows through the
    /// generic children tree, and walking the exposed elements must
    /// materialize nothing: before the fix the walk reached every row's
    /// cell mock, which realized its row — O(rows) materialization.
    fn a_large_list_walk_materializes_no_rows() {
        let fixture = mounted();
        let table = &*fixture.table;
        assert_eq!(table.numberOfRows(), ROWS.cast_signed());
        let realized = realized_rows(table);
        assert!(
            realized < ROWS / 10,
            "a freshly laid-out list must virtualize, not realize all {ROWS} rows"
        );

        let rows = ax_rows(table);
        assert_eq!(
            rows.count(),
            realized,
            "accessibilityRows must expose only the realized rows"
        );
        let children = ax_children(table);
        assert!(
            children.count() <= realized + NON_ROW_CHILDREN,
            "accessibilityChildren must expose only realized rows and the table's chrome"
        );

        // The walk an assistive client performs: each exposed element's
        // children. No cell mock for an unrealized row can be reached, so
        // the realized set must not grow.
        for element in &children {
            // SAFETY: the generic attribute getter answers an object.
            let _: Option<Retained<AnyObject>> = unsafe {
                msg_send![&*element, accessibilityAttributeValue: NSAccessibilityChildrenAttribute]
            };
        }
        assert_eq!(
            realized_rows(table),
            realized,
            "walking the exposed children must not materialize more rows"
        );
    }

    /// A selected-but-unrealized row stays in the row model —
    /// `accessibilityRows` and `accessibilitySelectedRows` — without
    /// joining the walkable children.
    fn a_selected_row_stays_reachable_without_joining_the_walk() {
        let fixture = mounted();
        let table = &*fixture.table;
        let selected = 150_000usize;
        table
            .selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(selected), false);

        let realized = realized_rows(table);
        let rows = ax_rows(table);
        assert_eq!(
            rows.count(),
            realized + 1,
            "the selected row must stay in the row model"
        );
        let selected_rows = ax_selected_rows(table);
        assert_eq!(
            selected_rows.count(),
            1,
            "the unrealized selection must remain reportable"
        );
        let children = ax_children(table);
        assert!(
            children.count() <= realized + NON_ROW_CHILDREN,
            "the generic children must not gain the unrealized selected row"
        );
    }

    /// The `list_ax::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        trial_each(vec![
            (
                "list_ax::a_large_list_walk_materializes_no_rows".to_owned(),
                Box::new(a_large_list_walk_materializes_no_rows),
            ),
            (
                "list_ax::a_selected_row_stays_reachable_without_joining_the_walk".to_owned(),
                Box::new(a_selected_row_stays_reachable_without_joining_the_walk),
            ),
        ])
    }
}

/// `AppKit` accessibility coverage for gestures — regression tests for
/// water-rs/waterui#1490. `NSClickGestureRecognizer` gives an `on_tap`
/// view no assistive affordance, so the wrapper the gesture renders must
/// itself be an element answering `accessibilityPerformPress` — the
/// `AXPress` action `AppKit` puts on an element whose class overrides it.
#[cfg(target_os = "macos")]
mod gesture_ax {
    use std::cell::Cell;
    use std::rc::Rc;

    use cocoa_ui::PlatformView;
    use cocoa_ui::objc2::runtime::ProtocolObject;
    use cocoa_ui::objc2::{msg_send, sel};
    use cocoa_ui::objc2_app_kit::NSAccessibility;
    use waterui::ViewExt as _;
    use waterui::prelude::text;

    use super::{resolve, trial_each};

    /// What an assistive client gets for `press` — the answer
    /// `accessibilityPerformPress` returns.
    fn perform_press(view: &PlatformView) -> bool {
        // SAFETY: `accessibilityPerformPress` answers a `BOOL`.
        unsafe { msg_send![view, accessibilityPerformPress] }
    }

    /// Whether the element offers `AXPress` to an assistive client.
    fn press_allowed(view: &PlatformView) -> bool {
        // SAFETY: `isAccessibilitySelectorAllowed:` takes a selector and
        // answers a `BOOL`.
        unsafe { msg_send![view, isAccessibilitySelectorAllowed: sel!(accessibilityPerformPress)] }
    }

    /// The gesture's `HostView` is an element whose press runs the tap's
    /// action.
    fn a_tap_gesture_answers_accessibility_perform_press() {
        let pressed = Rc::new(Cell::new(0u32));
        let leaf = resolve::render({
            let pressed = Rc::clone(&pressed);
            text("tap").on_tap(move || pressed.set(pressed.get() + 1))
        });
        let host = leaf.view();
        let element = ProtocolObject::<dyn NSAccessibility>::from_ref(host);
        assert!(
            element.isAccessibilityElement(),
            "the tap host must sit in the accessibility tree"
        );
        assert!(press_allowed(host), "the tap host must offer AXPress");
        assert!(perform_press(host), "the tap host must handle a press");
        assert_eq!(pressed.get(), 1, "the press must run the tap's action");
    }

    /// A long-press host claims no `AXPress`: a press does not stand in
    /// for a hold.
    fn a_long_press_gesture_keeps_the_default_press_answer() {
        let pressed = Rc::new(Cell::new(0u32));
        let leaf = resolve::render({
            let pressed = Rc::clone(&pressed);
            text("hold").on_long_press_gesture(500, move || {
                pressed.set(pressed.get() + 1);
            })
        });
        let host = leaf.view();
        assert!(
            !press_allowed(host),
            "a long-press host must not offer AXPress"
        );
        assert!(
            !perform_press(host),
            "a long-press host must not claim a press"
        );
        assert_eq!(pressed.get(), 0);
    }

    /// The `gesture_ax::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        trial_each(vec![
            (
                "gesture_ax::a_tap_gesture_answers_accessibility_perform_press".to_owned(),
                Box::new(a_tap_gesture_answers_accessibility_perform_press),
            ),
            (
                "gesture_ax::a_long_press_gesture_keeps_the_default_press_answer".to_owned(),
                Box::new(a_long_press_gesture_keeps_the_default_press_answer),
            ),
        ])
    }
}

/// `AppKit` accessibility coverage for the labelled-container frame —
/// water-rs/waterui#1482: a labelled layout container announces the
/// region its content resolved to, not the frame its parent assigned it.
#[cfg(target_os = "macos")]
mod a11y_extent {
    use cocoa_ui::PlatformView;
    use cocoa_ui::objc2::runtime::ProtocolObject;
    use cocoa_ui::objc2_app_kit::{NSAccessibility, NSAccessibilityFrameInView};
    use cocoa_ui::objc2_foundation::{NSRect, NSSize};
    use waterui::ViewExt as _;
    use waterui::prelude::text;
    use waterui_backend_core::AnyView;

    use super::{HostView, leaf, mtm, resolve, trial_each};

    /// The frame an assistive client reads for `view`.
    fn ax_frame(view: &PlatformView) -> NSRect {
        ProtocolObject::<dyn NSAccessibility>::from_ref(view).accessibilityFrame()
    }

    /// `view`'s own bounds reported in the screen space
    /// `accessibilityFrame` answers.
    fn own_frame(view: &PlatformView) -> NSRect {
        NSAccessibilityFrameInView(view, view.bounds())
    }

    /// #1482: a `view.size(8, 8).a11y_label("Sized")` under the window's
    /// `Overlay` is assigned the whole window while its content is placed
    /// in the resolved 8×8 — the labelled element must report the placed
    /// region, not the window.
    fn a_labelled_container_reports_its_content_extent() {
        let leaf = resolve::render(
            AnyView::new(AnyView::new(text("x")).size(8.0, 8.0))
                .size(8.0, 8.0)
                .a11y_label("Sized"),
        );
        let view = leaf.view();
        let _window = leaf::attach(mtm(), view);
        view.setNeedsLayout(true);
        view.layoutSubtreeIfNeeded();

        let content = view
            .subviews()
            .iter()
            .next()
            .expect("the frame container mounts its content as a subview");
        let reported = ax_frame(view);
        assert_eq!(
            reported.size,
            NSSize::new(8.0, 8.0),
            "the labelled element reports the resolved 8x8, not the window"
        );
        assert_eq!(
            reported,
            NSAccessibilityFrameInView(view, content.frame()),
            "the reported frame is where the content was placed"
        );
        assert_ne!(
            reported,
            own_frame(view),
            "the reported frame is not the assigned bounds"
        );
    }

    /// A labelled leaf that is no `HostView` keeps reporting its own
    /// frame — the flag never lands on it.
    fn a_labelled_plain_leaf_keeps_its_own_frame() {
        let on = waterui::reactive::binding(false);
        let leaf = resolve::render(
            waterui::component::toggle::Toggle::new("Wi-Fi", &on).a11y_label("Plain"),
        );
        let view = leaf.view();
        assert!(
            view.downcast_ref::<HostView>().is_none(),
            "a switch leaf is no host view"
        );
        let _window = leaf::attach(mtm(), view);
        view.setNeedsLayout(true);
        view.layoutSubtreeIfNeeded();
        assert_eq!(ax_frame(view), own_frame(view));
    }

    /// A host view without the flag reports what `NSView` reports; with
    /// the flag it reports the union of its non-hidden children — or its
    /// own bounds when it has none.
    fn a_host_view_reports_content_extent_only_when_marked() {
        let mtm = mtm();
        let host = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let _window = leaf::attach(mtm, &host);
        assert_eq!(
            ax_frame(&host),
            own_frame(&host),
            "an unmarked host reports `super`'s frame"
        );

        host.set_reports_content_extent(true);
        assert_eq!(
            ax_frame(&host),
            own_frame(&host),
            "with no children the extent falls back to the view's bounds"
        );

        let child = HostView::new(mtm, cocoa_ui::Rect::new(10.0, 20.0, 30.0, 40.0));
        let hidden = HostView::new(mtm, cocoa_ui::Rect::new(100.0, 100.0, 50.0, 50.0));
        cocoa_ui::view::set_hidden(&hidden, true);
        host.set_layout_handler(|_| {});
        cocoa_ui::view::add_subview(&host, &hidden);
        cocoa_ui::view::add_subview(&host, &child);
        let expected = NSAccessibilityFrameInView(&host, child.frame());
        assert_eq!(
            ax_frame(&host),
            expected,
            "the extent is the union of the non-hidden children's frames"
        );
    }

    /// The `a11y_extent::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        trial_each(vec![
            (
                "a11y_extent::a_labelled_container_reports_its_content_extent".to_owned(),
                Box::new(a_labelled_container_reports_its_content_extent),
            ),
            (
                "a11y_extent::a_labelled_plain_leaf_keeps_its_own_frame".to_owned(),
                Box::new(a_labelled_plain_leaf_keeps_its_own_frame),
            ),
            (
                "a11y_extent::a_host_view_reports_content_extent_only_when_marked".to_owned(),
                Box::new(a_host_view_reports_content_extent_only_when_marked),
            ),
        ])
    }
}

/// `embedding::mount` leaves everything the embedder owns alone: the host
/// window's title, frame, style and resizability, the close gate the host
/// armed, and the process's main menu. The embedded root follows the host
/// view's bounds and detaches when the mount drops.
#[cfg(all(target_os = "macos", feature = "native-test"))]
mod embedding {
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::rc::Rc;

    use waterui_apple::embedding::{
        HostEnvironment, waterui_apple_measure, waterui_apple_update_environment,
    };
    use waterui_core::layout::{
        Layout, LayoutDirection, Point, ProposalSize, Rect, Size, SubView, SubviewPlacement,
    };
    use waterui_locale::locale_binding;

    use cocoa_ui::objc2::MainThreadOnly;
    use cocoa_ui::objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
        NSApplication, NSMenu, NSView, NSWindowStyleMask,
    };
    use cocoa_ui::objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use cocoa_ui::{MainThreadMarker, Retained, view};
    use waterui::app::App;
    use waterui::component::menu::{Command, Menu, MenuItem};
    use waterui::component::text::Text;
    use waterui::graphics::color::ColorScheme;
    use waterui::layout::container::FixedContainer;
    use waterui::reactive::{Computed, binding};
    use waterui::regional;
    use waterui::text::font::ResolvedFont;
    use waterui::window::WindowState;
    use waterui::{Environment, Signal};

    use crate::mtm;

    /// A live mount plus the test runtime it borrows — `mount` requires the
    /// runtime to outlive it.
    struct Embed {
        mount: *mut c_void,
        _runtime: Box<waterui_apple::embedding::Runtime>,
    }

    impl Embed {
        fn new(host: &NSView, app: impl FnOnce(Environment) -> App) -> Self {
            Self::new_with_environment(host, None, app)
        }

        /// `environment` is the scoped host snapshot a `SwiftUI` host would
        /// declare; `None` mounts under the ambient platform.
        fn new_with_environment(
            host: &NSView,
            environment: Option<waterui_apple::embedding::HostEnvironment>,
            app: impl FnOnce(Environment) -> App,
        ) -> Self {
            let runtime = Box::new(waterui_apple::embedding::Runtime::for_test());
            // SAFETY: `_runtime` keeps the runtime box alive past the mount's
            // drop; `host` is a live view and the paths are NUL-terminated.
            let mount = unsafe {
                waterui_apple::embedding::mount(
                    (&raw const *runtime).cast::<c_void>(),
                    std::ptr::from_ref(host).cast_mut().cast(),
                    c"/tmp/waterui-embed-test-assets".as_ptr(),
                    c"/tmp/waterui-embed-test-fonts".as_ptr(),
                    environment,
                    app,
                )
            };
            Self {
                mount,
                _runtime: runtime,
            }
        }
    }

    impl Drop for Embed {
        fn drop(&mut self) {
            // SAFETY: `mount` was returned by `embedding::mount` and dropped
            // once on the main thread.
            unsafe { waterui_apple::embedding::waterui_apple_mount_drop(self.mount) };
        }
    }

    /// The named appearance — the constants are `AppKit` strings exported
    /// for the process's lifetime.
    fn appearance(name: &'static NSString) -> Retained<NSAppearance> {
        NSAppearance::appearanceNamed(name).expect("the named appearance exists")
    }

    /// `(dark, aqua)` — the pair the trials flip between.
    fn appearances() -> (&'static NSString, &'static NSString) {
        // SAFETY: reads string constants `AppKit` owns for the process's
        // lifetime.
        unsafe { (NSAppearanceNameDarkAqua, NSAppearanceNameAqua) }
    }

    /// A plain `NSView`, never touched by our kit — the embedder's own.
    fn plain_view(mtm: MainThreadMarker) -> Retained<NSView> {
        // SAFETY: `initWithFrame:` is `NSView`'s documented initializer.
        NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(120.0, 80.0)),
        )
    }

    /// The mount must not reach across the boundary: a declared window
    /// title/style/state and a declared menu bar change nothing on the
    /// host's window or the application's menu.
    pub fn an_embedded_mount_leaves_the_hosts_window_and_menu_bar_alone() {
        let mtm = mtm();
        let window = cocoa_ui::appkit::Window::new(
            mtm,
            cocoa_ui::Rect::new(200.0, 200.0, 640.0, 480.0),
            cocoa_ui::appkit::WindowStyle::TITLED,
        );
        let native = window.native();
        native.setTitle(&NSString::from_str("Host Title"));
        native.setStyleMask(
            native.styleMask() | NSWindowStyleMask::Resizable | NSWindowStyleMask::Closable,
        );
        let host_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("HostMenu"));
        let application = NSApplication::sharedApplication(mtm);
        let saved_menu = application.mainMenu();
        application.setMainMenu(Some(&host_menu));
        let close_called = Rc::new(RefCell::new(false));
        window.on_should_close({
            let close_called = Rc::clone(&close_called);
            move || {
                *close_called.borrow_mut() = true;
                false
            }
        });
        let content_view = native
            .contentView()
            .expect("a titled window has a content view");
        let host = plain_view(mtm);
        content_view.addSubview(&host);
        let host_frame = native.frame();
        let host_mask = native.styleMask();

        let embed = Embed::new(&host, |env| {
            App::new_with_windows(
                vec![waterui::window::Window::new(
                    "Declared Title",
                    binding(WindowState::Normal),
                    || Text::new("embedded"),
                )],
                env,
            )
            .menu_bar(Menu::new(
                "Embed",
                vec![MenuItem::Command(
                    Command::builder("DeclaredItem").action(|| {}),
                )],
            ))
        });

        assert_eq!(native.title().to_string(), "Host Title");
        assert_eq!(native.styleMask(), host_mask);
        assert!(native.isResizable());
        assert_eq!(native.frame(), host_frame);
        assert_eq!(
            application
                .mainMenu()
                .expect("the host menu stays")
                .title()
                .to_string(),
            "HostMenu"
        );
        // The host's close gate still answers — the mount armed no veto of
        // its own.
        native.performClose(None);
        assert!(*close_called.borrow());

        // Detach and remount on the same host.
        drop(embed);
        assert_eq!(view::subviews(&host).len(), 0, "drop detaches the root");
        let _remounted = Embed::new(&host, |env| App::new(|| Text::new("again"), env));
        assert_eq!(view::subviews(&host).len(), 1);
        application.setMainMenu(saved_menu.as_deref());
    }

    /// Each host view's `effectiveAppearance` drives its own mounted tree's
    /// color scheme; a change on one never reaches the other, and a mount
    /// dropped stops hearing its host entirely.
    pub fn each_host_views_appearance_drives_its_own_mounted_scheme() {
        let mtm = mtm();
        let host_a = plain_view(mtm);
        let host_b = plain_view(mtm);
        let (dark, aqua) = appearances();
        host_a.setAppearance(Some(&appearance(dark)));
        host_b.setAppearance(Some(&appearance(aqua)));

        let scheme_a = Rc::new(RefCell::new(None::<Computed<ColorScheme>>));
        let scheme_b = Rc::new(RefCell::new(None::<Computed<ColorScheme>>));
        let capture = |slot: &Rc<RefCell<Option<Computed<ColorScheme>>>>| {
            let slot = Rc::clone(slot);
            move |env: Environment| {
                *slot.borrow_mut() = Some(
                    env.query::<ColorScheme, Computed<ColorScheme>>()
                        .cloned()
                        .expect("the mount installs a color scheme"),
                );
                App::new(|| Text::new("x"), env)
            }
        };
        let embed_a = Embed::new(&host_a, capture(&scheme_a));
        let _embed_b = Embed::new(&host_b, capture(&scheme_b));

        let scheme_a = scheme_a
            .borrow()
            .clone()
            .expect("mount A installed a scheme");
        let scheme_b = scheme_b
            .borrow()
            .clone()
            .expect("mount B installed a scheme");
        assert_eq!(scheme_a.snapshot(), ColorScheme::Dark);
        assert_eq!(scheme_b.snapshot(), ColorScheme::Light);

        // Changing one host's appearance reaches only its own mount.
        host_a.setAppearance(Some(&appearance(aqua)));
        assert_eq!(scheme_a.snapshot(), ColorScheme::Light);
        assert_eq!(scheme_b.snapshot(), ColorScheme::Light);

        host_b.setAppearance(Some(&appearance(dark)));
        assert_eq!(scheme_a.snapshot(), ColorScheme::Light);
        assert_eq!(scheme_b.snapshot(), ColorScheme::Dark);

        // A dropped mount stops hearing its host: mutating it changes
        // nothing and fires nothing (a leaked observer would fire into the
        // freed signal graph).
        drop(embed_a);
        host_a.setAppearance(Some(&appearance(dark)));
        assert_eq!(scheme_a.snapshot(), ColorScheme::Light);
        assert_eq!(scheme_b.snapshot(), ColorScheme::Dark);
    }

    /// The embedded root tracks the host view's bounds through autoresizing
    /// and `drop` removes it without touching the host.
    pub fn the_embedded_root_follows_the_host_bounds_and_detaches_on_drop() {
        let mtm = mtm();
        let host = plain_view(mtm);
        let embed = Embed::new(&host, |env| App::new(|| Text::new("x"), env));

        let root = view::subviews(&host)
            .into_iter()
            .next()
            .expect("the mount attaches a root");
        assert_eq!(root.frame().size, host.bounds().size);

        host.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(300.0, 200.0)));
        assert_eq!(
            root.frame().size,
            NSSize::new(300.0, 200.0),
            "the root autoresizes with the host"
        );

        drop(embed);
        assert_eq!(view::subviews(&host).len(), 0);
        assert_eq!(host.frame().size, NSSize::new(300.0, 200.0));
    }

    /// TEST ONLY: synthetic host metrics — every style differs, so a
    /// style-order mixup in the ABI shows up as a wrong slot, not a
    /// coincidentally equal one.
    fn host_metrics(base: f64) -> [cocoa_ui::system_font::FontMetrics; 6] {
        std::array::from_fn(|index| {
            let index = f64::from(u32::try_from(index).expect("six host font styles"));
            cocoa_ui::system_font::FontMetrics {
                size: base + index,
                weight: 0.01 * (index + 1.0),
                line_height: base + 3.0 + index,
            }
        })
    }
    fn flat_metrics(metrics: &[cocoa_ui::system_font::FontMetrics; 6]) -> Vec<f64> {
        metrics
            .iter()
            .flat_map(|font| [font.size, font.weight, font.line_height])
            .collect()
    }
    fn test_locale(tag: &str) -> waterui::locale::Locale {
        tag.parse().expect("the test tags are valid BCP-47")
    }

    /// Records each proposal the measure ABI hands the mounted tree.
    #[derive(Debug)]
    struct ProbeLayout {
        seen: Rc<RefCell<Vec<ProposalSize>>>,
    }
    impl Layout for ProbeLayout {
        fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
            self.seen.borrow_mut().push(proposal);
            Size::new(17.0, 23.0)
        }
        fn place(
            &self,
            _bounds: Rect,
            proposal: ProposalSize,
            children: &[&dyn SubView],
        ) -> Vec<SubviewPlacement> {
            children
                .iter()
                .map(|_| {
                    SubviewPlacement::new(
                        Rect::new(Point::new(0.0, 0.0), Size::new(17.0, 23.0)),
                        proposal,
                    )
                })
                .collect()
        }
    }

    /// What the app body captures from the scoped environment: the scheme
    /// and direction `Computed`s, the three font slots the assertions use
    /// (Body 0, Headline 2, Footnote 5 — distinct metrics catch an ABI
    /// style-order mixup), the locale binding, and the body call count.
    struct SwiftUiCapture {
        bodies: Rc<Cell<usize>>,
        scheme: Rc<RefCell<Option<Computed<ColorScheme>>>>,
        fonts: Rc<RefCell<Vec<Computed<ResolvedFont>>>>,
        direction: Rc<RefCell<Option<Computed<LayoutDirection>>>>,
        locale: Rc<RefCell<Option<waterui::reactive::Binding<waterui::locale::Locale>>>>,
    }
    impl SwiftUiCapture {
        fn new() -> Self {
            Self {
                bodies: Rc::new(Cell::new(0)),
                scheme: Rc::new(RefCell::new(None)),
                fonts: Rc::new(RefCell::new(Vec::new())),
                direction: Rc::new(RefCell::new(None)),
                locale: Rc::new(RefCell::new(None)),
            }
        }
        /// The app closure: records the env's captured signals, then mounts
        /// the probe container.
        fn app(
            &self,
            seen: Rc<RefCell<Vec<ProposalSize>>>,
        ) -> impl FnOnce(Environment) -> App + use<> {
            let (bodies, scheme, fonts, direction, locale) = (
                Rc::clone(&self.bodies),
                Rc::clone(&self.scheme),
                Rc::clone(&self.fonts),
                Rc::clone(&self.direction),
                Rc::clone(&self.locale),
            );
            move |env: Environment| {
                bodies.set(bodies.get() + 1);
                *scheme.borrow_mut() = env.query::<ColorScheme, Computed<ColorScheme>>().cloned();
                fonts.borrow_mut().extend([
                    env.query::<waterui::text::font::Body, Computed<ResolvedFont>>()
                        .cloned()
                        .expect("a scoped mount installs the Body font slot"),
                    env.query::<waterui::text::font::Headline, Computed<ResolvedFont>>()
                        .cloned()
                        .expect("a scoped mount installs the Headline font slot"),
                    env.query::<waterui::text::font::Footnote, Computed<ResolvedFont>>()
                        .cloned()
                        .expect("a scoped mount installs the Footnote font slot"),
                ]);
                *direction.borrow_mut() = Some(waterui::layout::layout_direction(&env));
                let binding = locale_binding(&env);
                assert_eq!(
                    env.get::<waterui::locale::Locale>()
                        .expect("a scoped mount inserts the host's static Locale"),
                    &binding.snapshot()
                );
                *locale.borrow_mut() = Some(binding);
                App::new(
                    move || {
                        FixedContainer::new(
                            ProbeLayout {
                                seen: Rc::clone(&seen),
                            },
                            (Text::new("probe"),),
                        )
                    },
                    env,
                )
            }
        }
        fn scheme_now(&self) -> ColorScheme {
            self.scheme.borrow().as_ref().unwrap().snapshot()
        }
        fn direction_now(&self) -> LayoutDirection {
            self.direction.borrow().as_ref().unwrap().snapshot()
        }
        fn locale_now(&self) -> String {
            self.locale
                .borrow()
                .as_ref()
                .unwrap()
                .snapshot()
                .canonical_tag()
        }
        fn font_now(&self, slot: usize) -> ResolvedFont {
            self.fonts.borrow()[slot].snapshot()
        }
    }

    /// The initial assertions: the mount captured the declared scheme,
    /// direction, locale and per-style metrics; the body ran once.
    fn assert_scoped_snapshot(
        cap: &SwiftUiCapture,
        scheme: ColorScheme,
        direction: LayoutDirection,
        locale_tag: &str,
        base: f64,
    ) {
        assert_eq!(cap.bodies.get(), 1, "the mount built its body once");
        assert_eq!(cap.scheme_now(), scheme);
        assert_eq!(cap.direction_now(), direction);
        assert_eq!(cap.locale_now(), locale_tag);
        assert_eq!(f64::from(cap.font_now(0).size), base, "Body size");
        assert_eq!(f64::from(cap.font_now(1).size), base + 2.0, "Headline size");
        assert_eq!(f64::from(cap.font_now(2).size), base + 5.0, "Footnote size");
        assert_eq!(
            cap.font_now(0).line_height.map(f64::from),
            Some(base + 3.0),
            "Body line height"
        );
        assert_eq!(
            cap.font_now(0).weight,
            waterui::text::font::FontWeight::Normal,
            "Body weight"
        );
    }

    /// The measure ABI delivers the host's proposal verbatim: `nil` axes
    /// are unbounded, `0`, finite and `.infinity` arrive as themselves,
    /// and the layout's own size comes back. The contract's own re-measure
    /// at the resolved size may append a second record — the first is the
    /// ABI's delivery.
    fn assert_measure_proposals(mount: *mut c_void, seen: &Rc<RefCell<Vec<ProposalSize>>>) {
        for (width, height, hw, hh, want) in [
            (0.0, 0.0, false, false, ProposalSize::new(None, None)),
            (0.0, 0.0, true, false, ProposalSize::new(Some(0.0), None)),
            (
                50.0,
                40.0,
                true,
                true,
                ProposalSize::new(Some(50.0), Some(40.0)),
            ),
            (
                f64::INFINITY,
                f64::INFINITY,
                true,
                true,
                ProposalSize::new(Some(f32::INFINITY), Some(f32::INFINITY)),
            ),
        ] {
            seen.borrow_mut().clear();
            let mut out = [0.0f64; 2];
            // SAFETY: `mount` is a live mount; `out` is a two-element f64.
            unsafe { waterui_apple_measure(mount, width, height, hw, hh, out.as_mut_ptr()) };
            assert_eq!(out, [17.0, 23.0], "the layout's own size comes back");
            assert_eq!(
                seen.borrow().first(),
                Some(&want),
                "the ABI delivers the proposal axes verbatim"
            );
        }
    }

    /// A `SwiftUI`-scoped mount reads its declared environment — scheme,
    /// direction, locale and the six resolved font metrics — updates in
    /// place through `waterui_apple_update_environment`, and measures under
    /// the host's proposal through `waterui_apple_measure`, never
    /// remounting.
    pub fn a_swiftui_host_measures_and_updates_its_environment_without_remounting() {
        let mtm = mtm();
        let host_a = plain_view(mtm);
        let host_b = plain_view(mtm);
        let seen_a = Rc::new(RefCell::new(Vec::new()));
        let seen_b = Rc::new(RefCell::new(Vec::new()));
        let cap_a = SwiftUiCapture::new();
        let cap_b = SwiftUiCapture::new();
        let global_tag_before = regional::current_settings().locale_tag().to_string();

        let embed_a = Embed::new_with_environment(
            &host_a,
            Some(HostEnvironment {
                scheme: cocoa_ui::ColorScheme::Light,
                direction: LayoutDirection::LeftToRight,
                locale: test_locale("en-US"),
                fonts: host_metrics(13.0),
            }),
            cap_a.app(Rc::clone(&seen_a)),
        );
        let _embed_b = Embed::new_with_environment(
            &host_b,
            Some(HostEnvironment {
                scheme: cocoa_ui::ColorScheme::Dark,
                direction: LayoutDirection::RightToLeft,
                locale: test_locale("ar-EG"),
                fonts: host_metrics(20.0),
            }),
            cap_b.app(Rc::clone(&seen_b)),
        );
        assert_scoped_snapshot(
            &cap_a,
            ColorScheme::Light,
            LayoutDirection::LeftToRight,
            "en-US",
            13.0,
        );
        assert_scoped_snapshot(
            &cap_b,
            ColorScheme::Dark,
            LayoutDirection::RightToLeft,
            "ar-EG",
            20.0,
        );
        assert_measure_proposals(embed_a.mount, &seen_a);

        // The same mount updates in place — the root view's identity is
        // the whole point of no-remount.
        let root_before = view::subviews(&host_a)
            .into_iter()
            .next()
            .expect("mount A has a root view");
        let updated = host_metrics(26.0);
        let updated_flat = flat_metrics(&updated);
        let ja = c"ja-JP";
        // SAFETY: `embed_a` is a live scoped mount; `ja` is NUL-terminated
        // and `updated_flat` holds eighteen doubles.
        unsafe {
            waterui_apple_update_environment(
                embed_a.mount,
                true,
                true,
                ja.as_ptr(),
                updated_flat.as_ptr(),
            );
        }
        assert!(
            waterui_apple::native_test_support::pump_main_until(
                waterui_apple::native_test_support::MAIN_QUEUE_DEADLINE,
                || (cap_a.font_now(0).size - 26.0).abs() < f32::EPSILON
            ),
            "the Body font slot lands the update within the pump deadline"
        );
        assert_eq!(cap_a.font_now(0).size, 26.0);
        assert_eq!(cap_a.font_now(2).size, 31.0, "A Footnote size");
        assert_eq!(
            cap_a.font_now(0).line_height,
            Some(29.0),
            "A Body line height"
        );
        assert_eq!(cap_a.scheme_now(), ColorScheme::Dark);
        // ja-JP is left-to-right in isolation — the direction binding is the
        // host's explicit declaration, not the locale's derived default.
        assert_eq!(cap_a.direction_now(), LayoutDirection::RightToLeft);
        assert_eq!(cap_a.locale_now(), "ja-JP");
        assert_eq!(
            cap_a.bodies.get(),
            1,
            "an environment update never remounts"
        );
        assert_eq!(
            view::subviews(&host_a)
                .into_iter()
                .next()
                .map(|view| &raw const *view),
            Some(&raw const *root_before),
            "the root view is the same object before and after the update"
        );

        // Nothing reaches the sibling mount or the process's own regional
        // settings.
        assert_eq!(cap_b.bodies.get(), 1);
        assert_eq!(cap_b.font_now(0).size, 20.0);
        assert_eq!(cap_b.locale_now(), "ar-EG");
        assert_eq!(
            regional::current_settings().locale_tag(),
            global_tag_before.as_str(),
            "the process locale is untouched by two scoped mounts and an update"
        );
    }
}

/// The shared gesture contract end to end on `AppKit`: a completed
/// gesture inserts the observed `Gesture` and its typed event —
/// `TapEvent`, `RotationEvent`, `MagnificationEvent` — into the action's
/// environment, so a `Use<T>` handler extracts the same value Hydrolysis
/// reports for the same input. These cases cover the observer's
/// state/getter mapping, not the OS's gesture recognition: they drive
/// the installed `CocoaUiGestureTarget` with seeded recognizer subclasses
/// (and one `accessibilityPerformPress` entry point) — each dispatch
/// lands on the same `on_recognized` path a real completion does. The
/// `UIKit` cases live in `native_app`'s `gesture_events`, inside the
/// real `UIApplication` scene.
#[cfg(target_os = "macos")]
mod gesture_events {
    use std::cell::RefCell;
    use std::rc::Rc;

    use cocoa_ui::objc2::msg_send;
    use cocoa_ui::objc2_app_kit::{
        NSClickGestureRecognizer, NSGestureRecognizerState, NSMagnificationGestureRecognizer,
        NSRotationGestureRecognizer,
    };
    use cocoa_ui::objc2_foundation::NSPoint;
    use waterui::ViewExt as _;
    use waterui::extract::Use;
    use waterui::gesture::{Gesture, GesturePhase};
    use waterui::gesture::{
        MagnificationEvent, MagnificationGesture, PointerButton, RotationEvent, RotationGesture,
        TapEvent, TapGesture,
    };
    use waterui::prelude::text;

    use super::{resolve, trial_each};

    /// `AppKit`: an assistive client's `AXPress` activates the tap the way
    /// a click's recognizer completion would — through the shared
    /// `on_recognized` dispatch; the action's environment carries the
    /// observed `Gesture` and a `TapEvent` naming the activation point in
    /// the host's local space, the tap count and the button.
    #[cfg(target_os = "macos")]
    fn a_tap_gesture_delivers_its_event() {
        let taps = Rc::new(RefCell::new(Vec::new()));
        let gestures = Rc::new(RefCell::new(Vec::new()));
        let leaf = resolve::render({
            let taps = Rc::clone(&taps);
            let gestures = Rc::clone(&gestures);
            text("tap").gesture(
                TapGesture::new(),
                move |Use(event): Use<TapEvent>, Use(gesture): Use<Gesture>| {
                    taps.borrow_mut().push(event);
                    gestures.borrow_mut().push(gesture);
                },
            )
        });
        let host = leaf.view();
        let _window = super::mount_and_order_front(host);
        cocoa_ui::view::set_frame(host, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        for (origin, expected) in [
            (NSPoint::new(0.0, 0.0), (100.0, 50.0)),
            (NSPoint::new(20.0, 30.0), (120.0, 80.0)),
        ] {
            host.setBoundsOrigin(origin);
            let before = taps.borrow().len();
            // SAFETY: `accessibilityPerformPress` answers a `BOOL`.
            let handled: bool = unsafe { msg_send![host, accessibilityPerformPress] };
            assert!(handled, "the tap host's AXPress runs the gesture's action");
            {
                let taps = taps.borrow();
                assert_eq!(
                    taps.len(),
                    before + 1,
                    "each press appends exactly one tap event"
                );
                let tap = &taps[before];
                assert_eq!(tap.count, 1, "an assistive press is a count-1 tap");
                assert_eq!(tap.button, PointerButton::Primary);
                assert_eq!(
                    tap.location,
                    waterui::gesture::GesturePoint::new(expected.0, expected.1),
                    "the event names the activation point — the bounds-space center"
                );
            }
            let gestures = gestures.borrow();
            assert_eq!(gestures.len(), before + 1);
            assert!(
                matches!(gestures[before], Gesture::Tap(_)),
                "the environment carries the observed gesture"
            );
        }
    }

    /// Seeded recognizers for the `AppKit` frontdoor: a headless process
    /// cannot feed a real recognizer an event stream, so these subclasses
    /// answer `state` and the measured value from cells while the
    /// observer's installed `CocoaUiGestureTarget` — read off the real
    /// attached recognizer — does the dispatch.
    #[cfg(target_os = "macos")]
    mod seeded {
        use std::cell::Cell;

        use cocoa_ui::objc2::rc::Retained;
        use cocoa_ui::objc2::runtime::{AnyObject, NSObjectProtocol};
        use cocoa_ui::objc2::{
            DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
        };
        use cocoa_ui::objc2_app_kit::{
            NSGestureRecognizerState, NSMagnificationGestureRecognizer,
            NSRotationGestureRecognizer, NSView,
        };
        use cocoa_ui::objc2_foundation::NSPoint;

        /// What the seeded recognizer's getters answer.
        #[derive(Debug)]
        pub struct SeededIvars {
            state: Cell<NSGestureRecognizerState>,
            value: Cell<f64>,
            location: Cell<NSPoint>,
        }

        macro_rules! seeded {
            ($name:ident, $super:ident, $getter:ident, $class:literal) => {
                define_class!(
                    // SAFETY: subclassing for seeded input only; every
                    // override answers an ivar and touches no `AppKit`
                    // internals.
                    #[unsafe(super($super))]
                    #[name = $class]
                    #[thread_kind = MainThreadOnly]
                    #[ivars = SeededIvars]
                    pub struct $name;

                    unsafe impl NSObjectProtocol for $name {}

                    impl $name {
                        // SAFETY: `state`'s signature; answers the seed.
                        #[unsafe(method(state))]
                        fn state(&self) -> NSGestureRecognizerState {
                            self.ivars().state.get()
                        }

                        // SAFETY: the superclass getter's signature;
                        // answers the seed.
                        #[unsafe(method($getter))]
                        fn value(&self) -> f64 {
                            self.ivars().value.get()
                        }

                        // SAFETY: `locationInView:`'s signature; answers
                        // the seed.
                        #[unsafe(method(locationInView:))]
                        fn location_in_view(&self, _view: Option<&NSView>) -> NSPoint {
                            self.ivars().location.get()
                        }
                    }
                );

                impl $name {
                    /// A seeded recognizer at `Possible`, zero value, the
                    /// host's center.
                    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
                        let ivars = SeededIvars {
                            state: Cell::new(NSGestureRecognizerState::Possible),
                            value: Cell::new(0.0),
                            location: Cell::new(NSPoint::new(0.0, 0.0)),
                        };
                        let this = Self::alloc(mtm).set_ivars(ivars);
                        // SAFETY: `initWithTarget:action:` accepts a null
                        // pair; the driver messages the installed target
                        // itself.
                        unsafe {
                            msg_send![
                                super(this),
                                initWithTarget: std::ptr::null::<AnyObject>(),
                                action: Option::<cocoa_ui::objc2::runtime::Sel>::None
                            ]
                        }
                    }

                    /// Re-seeds the getter answers.
                    pub fn seed(
                        &self,
                        state: NSGestureRecognizerState,
                        value: f64,
                        location: NSPoint,
                    ) {
                        self.ivars().state.set(state);
                        self.ivars().value.set(value);
                        self.ivars().location.set(location);
                    }
                }
            };
        }

        seeded!(
            SeededRotation,
            NSRotationGestureRecognizer,
            rotation,
            "WaterUiNativeTestSeededRotation"
        );
        seeded!(
            SeededMagnification,
            NSMagnificationGestureRecognizer,
            magnification,
            "WaterUiNativeTestSeededMagnification"
        );

        // A seeded click recognizer: `state`, `locationInView:` and a
        // `buttonMask` of the primary button.
        define_class!(
            // SAFETY: subclassing for seeded input only; every override
            // answers an ivar or a constant.
            #[unsafe(super(cocoa_ui::objc2_app_kit::NSClickGestureRecognizer))]
            #[name = "WaterUiNativeTestSeededClick"]
            #[thread_kind = MainThreadOnly]
            #[ivars = SeededIvars]
            pub struct SeededClick;

            unsafe impl NSObjectProtocol for SeededClick {}

            impl SeededClick {
                // SAFETY: `state`'s signature; answers the seed.
                #[unsafe(method(state))]
                fn state(&self) -> NSGestureRecognizerState {
                    self.ivars().state.get()
                }

                // SAFETY: `buttonMask`'s signature; answers the primary
                // button bit a real primary click carries.
                #[unsafe(method(buttonMask))]
                fn button_mask(&self) -> usize {
                    1
                }

                // SAFETY: `locationInView:`'s signature; answers the seed.
                #[unsafe(method(locationInView:))]
                fn location_in_view(&self, _view: Option<&NSView>) -> NSPoint {
                    self.ivars().location.get()
                }
            }
        );

        impl SeededClick {
            /// A seeded click at `Possible` at `location`.
            pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
                let ivars = SeededIvars {
                    state: Cell::new(NSGestureRecognizerState::Possible),
                    value: Cell::new(0.0),
                    location: Cell::new(NSPoint::new(0.0, 0.0)),
                };
                let this = Self::alloc(mtm).set_ivars(ivars);
                // SAFETY: `initWithTarget:action:` accepts a null pair.
                unsafe {
                    msg_send![
                        super(this),
                        initWithTarget: std::ptr::null::<AnyObject>(),
                        action: Option::<cocoa_ui::objc2::runtime::Sel>::None
                    ]
                }
            }

            /// Re-seeds `state` and `locationInView:`.
            pub fn seed(&self, state: NSGestureRecognizerState, location: NSPoint) {
                self.ivars().state.set(state);
                self.ivars().location.set(location);
            }
        }
    }

    /// Sends `sender` to the target the observer installed on the real
    /// `R` recognizer attached to `host` — the production `invoke:`
    /// dispatch reads the sender's state and getters.
    #[cfg(target_os = "macos")]
    fn drive<R>(
        host: &cocoa_ui::PlatformView,
        sender: &cocoa_ui::objc2_app_kit::NSGestureRecognizer,
    ) where
        R: cocoa_ui::objc2::DowncastTarget
            + std::ops::Deref<Target = cocoa_ui::objc2_app_kit::NSGestureRecognizer>,
    {
        let installed = host
            .gestureRecognizers()
            .iter()
            .find_map(|r| r.downcast::<R>().ok())
            .expect("the observer attaches a recognizer of this kind");
        let target = installed
            .target()
            .expect("the recognizer's target stays installed");
        // SAFETY: `invoke:` takes one `NSGestureRecognizer`; the target
        // the observer installed answers it.
        let _: () = unsafe { msg_send![&*target, invoke: sender] };
    }

    /// `AppKit`: `NSRotationGestureRecognizer.rotation` reports radians
    /// natively — the event's angle is the raw getter value, and each
    /// `Updated`'s velocity is the wall-clock delta rate, never a
    /// second conversion (#2757).
    #[cfg(target_os = "macos")]
    fn a_rotation_gesture_reports_native_radians() {
        let rotations = Rc::new(RefCell::new(Vec::new()));
        let leaf = resolve::render({
            let rotations = Rc::clone(&rotations);
            text("rotate").gesture(
                RotationGesture::new(0.0),
                move |Use(e): Use<RotationEvent>| {
                    rotations.borrow_mut().push(e);
                },
            )
        });
        let host = leaf.view();
        let _window = super::mount_and_order_front(host);
        let sender = seeded::SeededRotation::new(super::mtm());
        let here = NSPoint::new(100.0, 50.0);
        let fire = |state, value| {
            sender.seed(state, value, here);
            let before = rotations.borrow().len();
            drive::<NSRotationGestureRecognizer>(host, &sender);
            let events = rotations.borrow();
            assert_eq!(events.len(), before + 1, "one callback appends one event");
            events[before].clone()
        };

        let e = fire(NSGestureRecognizerState::Began, 0.0);
        assert_eq!(
            (e.phase, e.angle, e.velocity),
            (GesturePhase::Started, 0.0, 0.0)
        );
        let e = fire(NSGestureRecognizerState::Changed, 0.75);
        assert_eq!(e.phase, GesturePhase::Updated);
        assert!(
            (e.angle - 0.75).abs() < 1e-6,
            "angle must be the raw radians getter, saw {}",
            e.angle
        );
        assert!(
            e.velocity.is_finite() && e.velocity > 0.0,
            "rotation progressed: {}",
            e.velocity
        );
        let e = fire(NSGestureRecognizerState::Changed, 1.25);
        assert!((e.angle - 1.25).abs() < 1e-6);
        let e = fire(NSGestureRecognizerState::Ended, 1.25);
        assert_eq!((e.phase, e.velocity), (GesturePhase::Ended, 0.0));
        assert!((e.angle - 1.25).abs() < 1e-6);

        // A second gesture reseeds at zero; a negative update and a
        // cancel keep the last angle with zero velocity.
        let e = fire(NSGestureRecognizerState::Began, 0.0);
        assert_eq!(e.phase, GesturePhase::Started);
        let e = fire(NSGestureRecognizerState::Changed, -0.5);
        assert!((e.angle + 0.5).abs() < 1e-6);
        let e = fire(NSGestureRecognizerState::Cancelled, -0.5);
        assert_eq!((e.phase, e.velocity), (GesturePhase::Cancelled, 0.0));
        assert!((e.angle + 0.5).abs() < 1e-6);
    }

    /// `AppKit`: `NSMagnificationGestureRecognizer.magnification` is the
    /// gesture's current fractional magnification — `1 + m` is the factor
    /// since it began — so composing the reported factor's relative delta
    /// never multiplies a cumulative value in twice (#2757).
    #[cfg(target_os = "macos")]
    fn a_magnification_gesture_composes_the_native_factor() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let leaf = resolve::render({
            let events = Rc::clone(&events);
            text("pinch").gesture(
                MagnificationGesture::new(2.0),
                move |Use(e): Use<MagnificationEvent>| {
                    events.borrow_mut().push(e);
                },
            )
        });
        let host = leaf.view();
        let _window = super::mount_and_order_front(host);
        let sender = seeded::SeededMagnification::new(super::mtm());
        let here = NSPoint::new(100.0, 50.0);
        let fire = |state, value| {
            sender.seed(state, value, here);
            let before = events.borrow().len();
            drive::<NSMagnificationGestureRecognizer>(host, &sender);
            let events = events.borrow();
            assert_eq!(events.len(), before + 1, "one callback appends one event");
            events[before].clone()
        };

        let e = fire(NSGestureRecognizerState::Began, 0.0);
        assert_eq!(
            (e.phase, e.scale, e.velocity),
            (GesturePhase::Started, 2.0, 0.0)
        );
        let e = fire(NSGestureRecognizerState::Changed, 0.25);
        assert!(
            (e.scale - 2.5).abs() < 1e-5,
            "m .25 -> factor 1.25: {}",
            e.scale
        );
        assert!(e.velocity.is_finite() && e.velocity > 0.0);
        let e = fire(NSGestureRecognizerState::Changed, 0.5);
        assert!(
            (e.scale - 3.0).abs() < 1e-5,
            "m .5 -> factor 1.5: {}",
            e.scale
        );
        assert!(e.velocity.is_finite() && e.velocity > 0.0);
        let e = fire(NSGestureRecognizerState::Changed, 0.5);
        assert!(
            (e.scale - 3.0).abs() < 1e-5 && e.velocity == 0.0,
            "an unchanged sample adds nothing"
        );
        let e = fire(NSGestureRecognizerState::Ended, 0.5);
        assert_eq!(
            (e.phase, e.scale, e.velocity),
            (GesturePhase::Ended, 3.0, 0.0)
        );

        // The next gesture reseeds the baseline; a smaller factor zooms
        // out with a negative velocity and the cancel keeps the scale.
        let e = fire(NSGestureRecognizerState::Began, 0.0);
        assert_eq!((e.phase, e.scale), (GesturePhase::Started, 2.0));
        let e = fire(NSGestureRecognizerState::Changed, -0.25);
        assert!(
            (e.scale - 1.5).abs() < 1e-5,
            "m -.25 -> factor .75: {}",
            e.scale
        );
        assert!(e.velocity.is_finite() && e.velocity < 0.0);
        let e = fire(NSGestureRecognizerState::Cancelled, -0.25);
        assert_eq!((e.phase, e.velocity), (GesturePhase::Cancelled, 0.0));
        assert!((e.scale - 1.5).abs() < 1e-5);
    }

    /// `AppKit`: `Then(tap, rotation)` forwards the second gesture only
    /// after the first completes, end to end through the installed
    /// recognizer targets (#2757).
    #[cfg(target_os = "macos")]
    fn a_then_gesture_forwards_after_the_first_completes() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let gestures = Rc::new(RefCell::new(Vec::new()));
        let leaf = resolve::render({
            let events = Rc::clone(&events);
            let gestures = Rc::clone(&gestures);
            text("then").gesture(
                Gesture::Tap(TapGesture::new()).then(RotationGesture::new(0.0)),
                move |Use(event): Use<RotationEvent>, Use(gesture): Use<Gesture>| {
                    events.borrow_mut().push(event);
                    gestures.borrow_mut().push(gesture);
                },
            )
        });
        let host = leaf.view();
        let _window = super::mount_and_order_front(host);
        let sender = seeded::SeededRotation::new(super::mtm());
        let here = NSPoint::new(100.0, 50.0);

        // A rotation before any tap reaches the second recognizer but the
        // `Then` gate stays shut: no event.
        sender.seed(NSGestureRecognizerState::Began, 0.5, here);
        drive::<NSRotationGestureRecognizer>(host, &sender);
        assert!(events.borrow().is_empty(), "an unarmed Then emitted");

        // The installed click target completes the tap and arms the
        // rotation — still through the recognizer path, not AX.
        let click = seeded::SeededClick::new(super::mtm());
        click.seed(NSGestureRecognizerState::Ended, here);
        drive::<NSClickGestureRecognizer>(host, &click);
        assert!(events.borrow().is_empty(), "arming the Then emitted");

        // Now the armed rotation forwards.
        sender.seed(NSGestureRecognizerState::Began, 0.5, here);
        drive::<NSRotationGestureRecognizer>(host, &sender);
        {
            let events = events.borrow();
            let [event] = events.as_slice() else {
                panic!(
                    "the armed rotation emits exactly once, saw {}",
                    events.len()
                );
            };
            assert_eq!(event.phase, GesturePhase::Started);
            assert!((event.angle - 0.5).abs() < 1e-6);
        }
        assert!(
            matches!(gestures.borrow().as_slice(), [Gesture::Then(_)]),
            "the observed gesture is the Then composition"
        );

        // The armed slot was consumed: another callback without a new
        // first-gesture completion emits nothing.
        sender.seed(NSGestureRecognizerState::Changed, 1.0, here);
        drive::<NSRotationGestureRecognizer>(host, &sender);
        assert_eq!(events.borrow().len(), 1, "an unarmed update must not emit");
    }

    /// The `gesture_events::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        trial_each(vec![
            (
                "gesture_events::a_tap_gesture_delivers_its_event".to_owned(),
                Box::new(a_tap_gesture_delivers_its_event) as Box<dyn FnOnce() + Send>,
            ),
            #[cfg(target_os = "macos")]
            (
                "gesture_events::a_rotation_gesture_reports_native_radians".to_owned(),
                Box::new(a_rotation_gesture_reports_native_radians),
            ),
            #[cfg(target_os = "macos")]
            (
                "gesture_events::a_magnification_gesture_composes_the_native_factor".to_owned(),
                Box::new(a_magnification_gesture_composes_the_native_factor),
            ),
            #[cfg(target_os = "macos")]
            (
                "gesture_events::a_then_gesture_forwards_after_the_first_completes".to_owned(),
                Box::new(a_then_gesture_forwards_after_the_first_completes),
            ),
        ])
    }
}
/// `selection_menu` rows on the field editor's real context menu —
/// `textView:menu:forEvent:atIndex:` — resolved each request on an
/// environment carrying the live `TextSelection` (water-rs/waterui#3037).
#[cfg(all(target_os = "macos", feature = "native-test", feature = "text_field"))]
mod text_selection {
    use std::cell::RefCell;
    use std::rc::Rc;

    use cocoa_ui::PlatformView;
    use cocoa_ui::appkit::TextField as KitTextField;
    use cocoa_ui::objc2::DowncastTarget;
    use cocoa_ui::objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSMenu, NSTextView};
    use cocoa_ui::objc2_foundation::{NSPoint, NSRange, NSString};
    use cocoa_ui::{Retained, view};
    use waterui::Str;
    use waterui::component::menu::{Command, MenuItem};
    use waterui::component::text_field::TextField;
    use waterui::component::text_selection::TextSelection;
    use waterui::reactive::{Signal, binding};
    use waterui::text::StyledStr;

    use crate::{leaf, mtm, resolve};

    /// The first view of type `T` in a depth-first walk — the macOS
    /// counterpart of the iOS suite's `find_view`.
    fn find<T: DowncastTarget>(view: &PlatformView) -> Option<Retained<PlatformView>> {
        for sub in view::subviews(view) {
            if sub.downcast_ref::<T>().is_some() {
                return Some(sub);
            }
            if let Some(found) = find::<T>(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// The menu the field editor answers a right-click with — the path
    /// `textView:menu:forEvent:atIndex:` rides.
    fn edit_menu(window: &cocoa_ui::appkit::Window, editor: &NSTextView) -> Retained<NSMenu> {
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
        editor
            .menuForEvent(&event)
            .expect("the field editor produced a menu")
    }

    /// Fires the menu row titled `title` the way the platform does —
    /// `performActionForItemAtIndex:` sends the item's action at its target.
    fn perform(menu: &NSMenu, title: &str) {
        let item = menu
            .itemWithTitle(&NSString::from_str(title))
            .unwrap_or_else(|| panic!("no menu row titled {title}"));
        menu.performActionForItemAtIndex(menu.indexOfItem(&item));
    }

    /// A `selection_menu` command reads the selection the moment it fires —
    /// not the one the menu was built over — edits it through the native
    /// path, sees its write in the value binding, and lets the editor's
    /// undo restore both. `Option<TextSelection>` extracts `Some`, a
    /// disabled command renders disabled, and `styled_value` is the
    /// field's value binding.
    /// Undo restores the pre-command text exactly and the editor's
    /// `DidUndo` session observer reports it to the value binding; redo
    /// writes the replacement back through both legs. A synchronous
    /// external normalization inside the same write-back window still
    /// reaches the field: echo suppression skips only the exact echo.
    fn undo_and_normalization_roundtrip(
        field: &cocoa_ui::appkit::TextField,
        value: &waterui::reactive::Binding<Str>,
        editor: &NSTextView,
    ) {
        let undo = editor
            .undoManager()
            .expect("the field editor carries an undo manager");
        undo.undo();
        assert_eq!(field.string(), "hé😀b");
        assert_eq!(
            value.snapshot(),
            Str::from("hé😀b"),
            "undo reports the restored text to the value binding"
        );
        undo.redo();
        assert_eq!(field.string(), "hé《b");
        assert_eq!(
            value.snapshot(),
            Str::from("hé《b"),
            "redo reports the replayed text to the value binding"
        );
        undo.undo();
        assert_eq!(field.string(), "hé😀b");

        let _normalizer = value.watch({
            let value = value.clone();
            move |ctx| {
                if ctx.value() == &Str::from("x") {
                    value.set(Str::from("X"));
                }
            }
        });
        field.replace_range(NSRange::new(0, 5), &NSString::from_str("x"));
        assert_eq!(
            value.snapshot(),
            Str::from("X"),
            "the watcher rewrote the value synchronously"
        );
        assert_eq!(
            field.string(),
            "X",
            "the normalized value pushed back onto the field"
        );
    }

    pub fn a_selection_menu_command_edits_the_live_selection() {
        let mtm = mtm();
        let value = binding(Str::from("hé😀b"));
        let observed = Rc::new(RefCell::new((String::new(), 0usize, 0usize)));
        let optional_seen = Rc::new(RefCell::new(false));

        let observe = Command::builder("Observe").action({
            let observed = Rc::clone(&observed);
            move |selection: TextSelection| {
                let range = selection.range();
                *observed.borrow_mut() = (selection.text().to_string(), range.start, range.end);
            }
        });
        let optional = Command::builder("Optional").action({
            let optional_seen = Rc::clone(&optional_seen);
            move |selection: Option<TextSelection>| {
                *optional_seen.borrow_mut() = selection.is_some();
            }
        });
        // Selects a byte range of the live text — `Observe` proves it lands.
        let mark =
            Command::builder("Mark").action(|selection: TextSelection| selection.select(3..7));
        // Replaces whatever the selection covers when the command fires.
        let wrap_fired = Rc::new(RefCell::new(None));
        let wrap = Command::builder("Wrap").action({
            let wrap_fired = Rc::clone(&wrap_fired);
            move |selection: TextSelection| {
                let range = selection.range();
                *wrap_fired.borrow_mut() = Some(range);
                selection.replace("《");
            }
        });
        let styled = Command::builder("Styled").action(|selection: TextSelection| {
            selection
                .styled_value()
                .expect("a plain field answers styled_value")
                .set(StyledStr::plain("ZZ"));
        });
        let off = Command::builder("Off").action(|| {}).disabled(true);

        let mount = resolve::render(TextField::new("Name", &value).selection_menu(vec![
            MenuItem::Command(observe),
            MenuItem::Command(optional),
            MenuItem::Command(mark),
            MenuItem::Command(wrap),
            MenuItem::Command(styled),
            MenuItem::Command(off),
        ]));
        let window = leaf::attach(mtm, mount.view());
        let field_view =
            find::<KitTextField>(mount.view()).expect("the leaf mounts a kit TextField");
        let field = field_view
            .downcast::<KitTextField>()
            .expect("the found view is the kit TextField");
        assert!(field.begin_editing(), "the field never began editing");
        let editor = field.field_editor().expect("the field editor is live");

        // UTF-16 units: h=1 é=1 😀=2 b=1 — 😀 spans units 2..4, bytes 3..7.
        field.set_selected_range(NSRange::new(2, 2));
        let menu = edit_menu(&window, &editor);

        // The platform's own rows survive the custom append.
        assert!(
            menu.numberOfItems() > 6,
            "the standard edit rows were replaced rather than kept"
        );
        for title in ["Observe", "Optional", "Mark", "Wrap", "Styled", "Off"] {
            assert!(
                menu.itemWithTitle(&NSString::from_str(title)).is_some(),
                "the {title} row is missing from the editor's menu"
            );
        }

        // The selection moved after the menu was built: a command reads
        // the live range when it fires, not the built-over range.
        field.set_selected_range(NSRange::new(0, 2));
        perform(&menu, "Observe");
        let (text, start, end) = observed.borrow().clone();
        assert_eq!((text.as_str(), start, end), ("hé😀b", 0, 3));

        perform(&menu, "Optional");
        assert!(
            *optional_seen.borrow(),
            "Option<TextSelection> did not extract the live selection"
        );

        // A disabled command renders a disabled row.
        let off_item = menu
            .itemWithTitle(&NSString::from_str("Off"))
            .expect("the Off row");
        assert!(!off_item.isEnabled(), "a disabled command is not disabled");

        // `Mark` selects  😀 (bytes 3..7 → UTF-16 2..4).

        perform(&menu, "Mark");
        assert_eq!(
            field.selected_range(),
            Some(NSRange::new(2, 2)),
            "Mark selected 😀 through the command"
        );

        // `replace` edits through the native path: the field, the value
        // binding and the caret all reflect it.
        perform(&menu, "Wrap");
        assert_eq!(field.string(), "hé《b");
        assert_eq!(
            value.snapshot(),
            Str::from("hé《b"),
            "the native edit wrote through to the value binding"
        );
        assert_eq!(
            field.selected_range(),
            Some(NSRange::new(3, 0)),
            "the caret lands after the inserted text"
        );

        undo_and_normalization_roundtrip(&field, &value, &editor);

        // `styled_value` is the field's value binding: writing it lands on
        // `value` and the field shows the pushed text.
        perform(&menu, "Styled");
        assert_eq!(value.snapshot(), Str::from("ZZ"));
        assert_eq!(field.string(), "ZZ");

        window.close();
    }
}

/// §4.5 structural transitions: a removed `Dynamic` child leaves the
/// semantic tree at once — watchers and retained values die with the
/// leaf — while a declared removal keeps the outgoing view alive as a
/// non-interactive ghost beneath the new child until the clock ends.
mod transition {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use waterui::Identifiable;
    use waterui::ViewExt;
    use waterui::animation::Animation;
    use waterui::component::Dynamic;
    use waterui::component::label::label;
    use waterui::component::lazy::Lazy;
    use waterui::prelude::{VStack, hstack, spacer, text, vstack};
    use waterui::reactive::collection::List as ReactiveList;
    use waterui::views::ForEach;
    use waterui_apple::contract::Mounted;
    use waterui_apple::native_test_support::pump_main_until;
    use waterui_backend_core::AnyView;
    use waterui_core::Metadata;
    use waterui_core::metadata::Retain;
    use waterui_core::transition::{PropertyTransition, Transition, TransitionSpec};

    use cocoa_ui::objc2::rc::{Weak, autoreleasepool};
    use cocoa_ui::view;

    use super::Retained;
    use super::leaf::TestSubView;
    use super::{HostView, Label, PlatformView, mtm};

    /// The Stage 1 retirement trials — registered through
    /// `transition::trials()` like the other suites' registrars.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "transition::retired_host_is_outside_hit_testing_and_accessibility",
                || {
                    retired_host_is_outside_hit_testing_and_accessibility();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::mounted_drop_under_retired_host_keeps_views",
                || {
                    mounted_drop_under_retired_host_keeps_views();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test("transition::unowned_declaration_panics_at_mount", || {
                unowned_declaration_panics_at_mount();
                Ok(())
            }),
            libtest_mimic::Trial::test(
                "transition::a_replaced_child_semantics_die_while_its_view_ghosts",
                || {
                    a_replaced_child_semantics_die_while_its_view_ghosts();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::removed_descendants_die_but_their_views_stay_in_the_ghost",
                || {
                    removed_descendants_die_but_their_views_stay_in_the_ghost();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::opacity_composes_with_the_contents_own_opacity",
                || {
                    opacity_composes_with_the_contents_own_opacity();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::scale_composes_with_the_contents_own_scale",
                || {
                    scale_composes_with_the_contents_own_scale();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::combined_transition_samples_every_frame",
                || {
                    combined_transition_samples_every_frame();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test("transition::adopted_conditional_claims_each_child", || {
                adopted_conditional_claims_each_child();
                Ok(())
            }),
            libtest_mimic::Trial::test(
                "transition::the_contents_own_claim_beats_the_adopted_one",
                || {
                    the_contents_own_claim_beats_the_adopted_one();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test("transition::initial_build_plays_no_insertion", || {
                initial_build_plays_no_insertion();
                Ok(())
            }),
            libtest_mimic::Trial::test(
                "transition::zero_duration_removal_detaches_at_once",
                || {
                    zero_duration_removal_detaches_at_once();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::ghost_completion_does_not_need_the_site",
                || {
                    ghost_completion_does_not_need_the_site();
                    Ok(())
                },
            ),
        ]
        .into_iter()
        .chain(collection_trials())
        .collect()
    }

    /// The Stage 3 collection trials (§8.2).
    fn collection_trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "transition::collection_ghost_keeps_its_last_placement",
                || {
                    collection_ghost_keeps_its_last_placement();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test("transition::ghosts_rebase_across_later_edits", || {
                ghosts_rebase_across_later_edits();
                Ok(())
            }),
            libtest_mimic::Trial::test(
                "transition::reinsertion_creates_a_fresh_instance_beside_the_ghost",
                || {
                    reinsertion_creates_a_fresh_instance_beside_the_ghost();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::unplaced_lazy_row_releases_immediately",
                || {
                    unplaced_lazy_row_releases_immediately();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test("transition::siblings_close_the_gap_at_completion", || {
                siblings_close_the_gap_at_completion();
                Ok(())
            }),
            libtest_mimic::Trial::test(
                "transition::list_ghost_row_keeps_its_frozen_height",
                || {
                    list_ghost_row_keeps_its_frozen_height();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::list_ghost_row_is_not_selectable_or_accessible",
                || {
                    list_ghost_row_is_not_selectable_or_accessible();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::list_claimed_insertion_plays_without_row_animation",
                || {
                    list_claimed_insertion_plays_without_row_animation();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::an_insert_before_existing_members_paints_in_item_order",
                || {
                    an_insert_before_existing_members_paints_in_item_order();
                    Ok(())
                },
            ),
        ]
        .into_iter()
        .chain(navigation_trials())
        .collect()
    }

    /// The Stage 5 navigation trials (§8.2): a page root's
    /// `.transition(...)` replaces the stack's own push/pop animation.
    fn navigation_trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "transition::navigation_claimed_pop_ghosts_above_the_new_top",
                || {
                    navigation_claimed_pop_ghosts_above_the_new_top();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::navigation_claimed_push_replaces_the_stack_animation",
                || {
                    navigation_claimed_push_replaces_the_stack_animation();
                    Ok(())
                },
            ),
        ]
        .into_iter()
        .chain(overlay_trials())
        .chain(frozen_trials())
        .chain(blur_trials())
        .chain(pixel_trials())
        .collect()
    }

    /// The Stage 6 anchored-overlay trials (§8.2): a dismissal keeps the
    /// presentation container — the `NSPanel` on `AppKit`, the passthrough
    /// host on `UIKit` — painting, inert to input, until the removal
    /// transition completes; a re-present during the exit is a fresh
    /// instance beside it (§5.6).
    fn overlay_trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "transition::overlay_dismiss_keeps_the_panel_until_the_ghost_completes",
                || {
                    overlay_dismiss_keeps_the_panel_until_the_ghost_completes();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::overlay_reopen_during_exit_is_a_fresh_instance",
                || {
                    overlay_reopen_during_exit_is_a_fresh_instance();
                    Ok(())
                },
            ),
        ]
    }

    /// An anchor view whose overlay body root declares an opacity
    /// removal: `shown` drives present/dismiss, and `Manual` dismissal
    /// keeps outside interaction out of the assertion.
    fn claimed_overlay() -> (
        waterui::reactive::Binding<bool>,
        waterui_apple::contract::NativeLeaf,
    ) {
        use waterui::metadata::anchored_overlay::{AnchoredOverlay, Dismissal};
        let shown = waterui::reactive::binding(false);
        let leaf = super::resolve::render(
            text("anchor").anchored_overlay(
                AnchoredOverlay::new(&shown, move || {
                    Metadata::new(
                        text("pop"),
                        TransitionSpec::new(PropertyTransition::opacity())
                            .with_animation(Animation::linear(Duration::from_millis(300))),
                    )
                })
                .dismissal(Dismissal::Manual),
            ),
        );
        (shown, leaf)
    }

    /// The passthrough containers a window currently holds: the
    /// full-window `HostView`s that are not the anchor's own leaf view —
    /// `UIKit`'s presentation surface for the overlay (§5.6).
    #[cfg(target_os = "ios")]
    fn passthrough_hosts(
        window: &PlatformView,
        anchor_view: &PlatformView,
    ) -> Vec<Retained<PlatformView>> {
        view::subviews(window)
            .into_iter()
            .filter(|sub| {
                !std::ptr::eq(&raw const **sub, std::ptr::from_ref(anchor_view))
                    && sub.downcast_ref::<HostView>().is_some()
            })
            .collect()
    }

    /// The child windows a window holds — `AppKit`'s presentation
    /// surface for the overlay is a borderless `NSPanel`, and a fresh
    /// harness window's children are all ours (§5.6).
    #[cfg(target_os = "macos")]
    fn panels_of(
        window: &cocoa_ui::objc2_app_kit::NSWindow,
    ) -> Vec<Retained<cocoa_ui::objc2_app_kit::NSWindow>> {
        window
            .childWindows()
            .map_or_else(Vec::new, |children| children.to_vec())
    }

    /// A dismissal ghosts the overlay out of the semantic tree while the
    /// presentation container keeps painting until the removal transition
    /// completes (§8.2): `AppKit` keeps the panel visible and transparent
    /// to mouse events; `UIKit` keeps the passthrough host in the window
    /// but retired, so nothing inside answers a hit test.
    pub fn overlay_dismiss_keeps_the_panel_until_the_ghost_completes() {
        let _mtm = mtm();
        let (shown, leaf) = claimed_overlay();
        #[cfg(target_os = "ios")]
        let window = super::mount_and_order_front(leaf.view());
        #[cfg(target_os = "macos")]
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| shown.set(true));
        #[cfg(target_os = "ios")]
        let weak = {
            let window_view: &PlatformView = &window;
            assert!(
                pump_main_until(5.0, || {
                    autoreleasepool(|_| !passthrough_hosts(window_view, leaf.view()).is_empty())
                }),
                "the overlay presents its passthrough host"
            );
            autoreleasepool(|_| {
                let host = passthrough_hosts(window_view, leaf.view())
                    .into_iter()
                    .next()
                    .expect("the passthrough host is in the window");
                Weak::new(&*host)
            })
        };
        #[cfg(target_os = "macos")]
        let weak = {
            let parent = cocoa_ui::appkit::window_of(leaf.view()).expect("anchor in a window");
            assert!(
                pump_main_until(5.0, || {
                    autoreleasepool(|_| !panels_of(&parent).is_empty())
                }),
                "the overlay presents its panel"
            );
            autoreleasepool(|_| {
                let panel = panels_of(&parent)
                    .into_iter()
                    .next()
                    .expect("the panel is a child window");
                Weak::new(&*panel)
            })
        };
        autoreleasepool(|_| shown.set(false));
        #[cfg(target_os = "ios")]
        autoreleasepool(|_| {
            let window_view: &PlatformView = &window;
            let host = passthrough_hosts(window_view, leaf.view())
                .into_iter()
                .next()
                .expect("the passthrough host stays for the exit");
            assert!(
                host.downcast_ref::<HostView>()
                    .is_some_and(HostView::is_retired),
                "the exiting container is retired — not hit-testable"
            );
            let bounds = view::bounds(&host);
            let hit = host.hitTest_withEvent(
                objc2_core_foundation::CGPoint::new(
                    bounds.size.width / 2.0,
                    bounds.size.height / 2.0,
                ),
                None,
            );
            assert!(hit.is_none(), "the exiting container yields no hit");
        });
        #[cfg(target_os = "macos")]
        autoreleasepool(|_| {
            let parent = cocoa_ui::appkit::window_of(leaf.view()).expect("anchor in a window");
            let panel = panels_of(&parent)
                .into_iter()
                .next()
                .expect("the panel stays for the exit");
            assert!(
                panel.ignoresMouseEvents(),
                "the exiting panel is transparent to mouse events"
            );
            assert!(panel.isVisible(), "the exiting panel keeps painting");
        });
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| weak.load().is_none())),
            "the completion closes the container"
        );
    }

    /// A re-present while the previous exit is still running is a fresh
    /// instance beside the ghost: two presentation containers coexist —
    /// the new one hit-testable, the exiting one inert — until the old
    /// removal completes and its container leaves (§8.2).
    pub fn overlay_reopen_during_exit_is_a_fresh_instance() {
        let _mtm = mtm();
        let (shown, leaf) = claimed_overlay();
        #[cfg(target_os = "ios")]
        {
            let window = super::mount_and_order_front(leaf.view());
            let window_view: &PlatformView = &window;
            reopen_ios(&shown, &leaf, window_view);
        }
        #[cfg(target_os = "macos")]
        {
            let _window = super::mount_and_order_front(leaf.view());
            reopen_macos(&shown, &leaf);
        }
    }

    /// The `UIKit` half: two passthrough hosts coexist — the fresh one
    /// live, the exiting one retired — until the old removal completes.
    #[cfg(target_os = "ios")]
    fn reopen_ios(
        shown: &waterui::reactive::Binding<bool>,
        leaf: &waterui_apple::contract::NativeLeaf,
        window: &PlatformView,
    ) {
        autoreleasepool(|_| shown.set(true));
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| passthrough_hosts(window, leaf.view()).len() == 1)
            }),
            "the first presentation lands"
        );
        let old_weak = autoreleasepool(|_| {
            let host = passthrough_hosts(window, leaf.view())
                .into_iter()
                .next()
                .expect("the first passthrough host");
            Weak::new(&*host)
        });
        autoreleasepool(|_| {
            shown.set(false);
            shown.set(true);
        });
        let (count, retired) = autoreleasepool(|_| {
            let hosts = passthrough_hosts(window, leaf.view());
            (
                hosts.len(),
                hosts
                    .iter()
                    .filter(|host| {
                        host.downcast_ref::<HostView>()
                            .is_some_and(HostView::is_retired)
                    })
                    .count(),
            )
        });
        assert_eq!(
            (count, retired),
            (2, 1),
            "the re-present adds a fresh container beside the exiting one"
        );
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| old_weak.load().is_none())),
            "the old exit completes on its own clock"
        );
        autoreleasepool(|_| {
            assert_eq!(
                passthrough_hosts(window, leaf.view()).len(),
                1,
                "only the fresh presentation remains"
            );
        });
    }

    /// The `AppKit` half: two child panels coexist — the fresh one
    /// interactive, the exiting one ignoring mouse events — until the
    /// old removal completes.
    #[cfg(target_os = "macos")]
    fn reopen_macos(
        shown: &waterui::reactive::Binding<bool>,
        leaf: &waterui_apple::contract::NativeLeaf,
    ) {
        let parent = cocoa_ui::appkit::window_of(leaf.view()).expect("anchor in a window");
        autoreleasepool(|_| shown.set(true));
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| panels_of(&parent).len() == 1)),
            "the first presentation lands"
        );
        let old_weak = autoreleasepool(|_| {
            let panel = panels_of(&parent)
                .into_iter()
                .next()
                .expect("the first panel");
            Weak::new(&*panel)
        });
        autoreleasepool(|_| {
            shown.set(false);
            shown.set(true);
        });
        let (count, ignored) = autoreleasepool(|_| {
            let panels = panels_of(&parent);
            (
                panels.len(),
                panels.iter().filter(|p| p.ignoresMouseEvents()).count(),
            )
        });
        assert_eq!(
            (count, ignored),
            (2, 1),
            "the re-present adds a fresh panel beside the exiting one"
        );
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| old_weak.load().is_none())),
            "the old exit completes on its own clock"
        );
        autoreleasepool(|_| {
            assert_eq!(
                panels_of(&parent).len(),
                1,
                "only the fresh presentation remains"
            );
        });
    }

    /// The `UINavigationController` owning a view in the subtree,
    /// depth-first — either the view's own controller is the nav
    /// controller, or the page controller answers one.
    #[cfg(target_os = "ios")]
    fn nav_controller_in(
        view: &PlatformView,
    ) -> Option<Retained<cocoa_ui::objc2_ui_kit::UINavigationController>> {
        use cocoa_ui::uikit::view_controller::owning_controller;
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

    /// A path-driven stack whose pushed page carries a page-root
    /// `.transition(...)`: `path.push`/`path.pop` drive the model
    /// transactions the trials assert on.
    fn claimed_stack() -> (
        waterui::navigation::NavigationPath<i32>,
        waterui_apple::contract::NativeLeaf,
    ) {
        use waterui::navigation::{NavigationPath, NavigationStack, NavigationView};
        let path = NavigationPath::<i32>::new();
        let leaf = super::resolve::render(
            NavigationStack::with_path(path.clone(), NavigationView::new("Root", text("root")))
                .destination(|_route: i32| {
                    NavigationView::new(
                        "Detail",
                        Metadata::new(
                            text("detail"),
                            TransitionSpec::new(PropertyTransition::opacity())
                                .with_animation(Animation::linear(Duration::from_millis(150))),
                        ),
                    )
                }),
        );
        (path, leaf)
    }

    /// A claimed page pushed then popped through the model: the pop is
    /// unanimated and the outgoing page's transition host is reparented
    /// into `nav.view` directly beneath the navigation bar — above the new
    /// top, as Hydrolysis paints the moving page over the stationary one —
    /// at the frame it occupied inside its controller's view, staying
    /// until the removal transition's clock releases it (§5.5). On
    /// `AppKit` the ghost is the stack host's top subview at the entry's
    /// last frame.
    pub fn navigation_claimed_pop_ghosts_above_the_new_top() {
        let _mtm = mtm();
        let (path, leaf) = claimed_stack();
        let _window = super::mount_and_order_front(leaf.view());
        #[cfg(target_os = "ios")]
        pop_ghost_ios(&path, &leaf);
        #[cfg(target_os = "macos")]
        pop_ghost_macos(&path, &leaf);
    }

    /// The `UIKit` half of the claimed-pop assertion: the subview of
    /// `nav.view` directly beneath the navigation bar is the outgoing
    /// page's retired transition host.
    #[cfg(target_os = "ios")]
    fn pop_ghost_ios(
        path: &waterui::navigation::NavigationPath<i32>,
        leaf: &waterui_apple::contract::NativeLeaf,
    ) {
        let nav = nav_controller_in(leaf.view()).expect("the stack mounts a navigation controller");
        autoreleasepool(|_| path.push(1));
        assert!(
            pump_main_until(5.0, || nav.viewControllers().count() == 2),
            "the push installs the claimed page"
        );
        let nav_view = nav.view().expect("the navigation controller's view");
        let (page_ptr, page_frame, weak) = autoreleasepool(|_| {
            let top = nav.topViewController().expect("the pushed page is top");
            let page_view = top.view().expect("the page's view");
            let host = view::subviews(&page_view)
                .into_iter()
                .next()
                .expect("a claimed page mounts inside a transition host");
            (
                std::ptr::from_ref(&*host),
                view::convert_rect(&host, view::bounds(&host), Some(&nav_view)),
                Weak::new(&*host),
            )
        });
        // The subview directly beneath the navigation bar.
        let beneath_bar = || {
            let bar = nav.navigationBar();
            let subs = view::subviews(&nav_view);
            let index = subs
                .iter()
                .position(|sub| std::ptr::eq(&raw const **sub, (&raw const *bar).cast()))
                .expect("the navigation bar is a subview of nav.view");
            index.checked_sub(1).map(|below| subs[below].clone())
        };
        let _ = autoreleasepool(|_| path.pop());
        let landed = pump_main_until(5.0, || {
            autoreleasepool(|_| {
                nav.viewControllers().count() == 1
                    && beneath_bar().is_some_and(|sub| std::ptr::eq(&raw const *sub, page_ptr))
            })
        });
        let (count, subs) = autoreleasepool(|_| {
            (
                nav.viewControllers().count(),
                view::subviews(&nav_view)
                    .iter()
                    .map(|sub| {
                        (
                            std::ptr::eq(&raw const **sub, page_ptr),
                            sub.class().name().to_string_lossy().into_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        });
        assert!(
            landed,
            "the pop leaves the claimed page's ghost above the root, beneath the bar: \
             {count} controllers, nav.view subviews {subs:?}"
        );
        autoreleasepool(|_| {
            let ghost = &beneath_bar().expect("the ghost sits beneath the bar");
            assert!(
                ghost
                    .downcast_ref::<HostView>()
                    .is_some_and(HostView::is_retired),
                "the ghost host retired at the pop"
            );
            let frame = view::frame(ghost);
            assert_eq!(
                (
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height
                ),
                (
                    page_frame.origin.x,
                    page_frame.origin.y,
                    page_frame.size.width,
                    page_frame.size.height
                ),
                "the ghost keeps the page's last frame in nav.view"
            );
        });
        assert!(
            pump_main_until(5.0, || weak.load().is_none()),
            "the completion releases the ghost's visuals"
        );
    }

    /// The `AppKit` half: the entries are direct subviews of the stack
    /// host, and a claimed pop retires the top entry's transition host
    /// above the remaining pages — the host's top subview, at the entry's
    /// last frame.
    #[cfg(target_os = "macos")]
    fn pop_ghost_macos(
        path: &waterui::navigation::NavigationPath<i32>,
        leaf: &waterui_apple::contract::NativeLeaf,
    ) {
        let host = view::retain_base(leaf.view());
        autoreleasepool(|_| path.push(1));
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| view::subviews(&host).len() == 2)
            }),
            "the push installs the claimed page"
        );
        let (page_ptr, entry_frame, weak) = autoreleasepool(|_| {
            let entry = view::subviews(&host)
                .into_iter()
                .next_back()
                .expect("the pushed page's entry view");
            let ghost = view::subviews(&entry)
                .into_iter()
                .next()
                .expect("a claimed page mounts inside a transition host");
            (
                std::ptr::from_ref(&*ghost),
                view::frame(&entry),
                Weak::new(&*ghost),
            )
        });
        let _ = autoreleasepool(|_| path.pop());
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| {
                    view::subviews(&host)
                        .last()
                        .is_some_and(|sub| std::ptr::eq(&raw const **sub, page_ptr))
                })
            }),
            "the pop leaves the claimed page's ghost above the root"
        );
        autoreleasepool(|_| {
            let ghost = view::subviews(&host)
                .pop()
                .expect("the ghost is the host's top subview");
            let ghost = &ghost;
            assert!(
                ghost
                    .downcast_ref::<HostView>()
                    .is_some_and(HostView::is_retired),
                "the ghost host retired at the pop"
            );
            let frame = view::frame(ghost);
            assert_eq!(
                (
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height
                ),
                (
                    entry_frame.origin.x,
                    entry_frame.origin.y,
                    entry_frame.size.width,
                    entry_frame.size.height
                ),
                "the ghost keeps the entry's last frame in the stack host"
            );
        });
        assert!(
            pump_main_until(5.0, || weak.load().is_none()),
            "the completion releases the ghost's visuals"
        );
    }

    /// A claimed page pushed through the model: the stack's own push
    /// animation is skipped — `UIKit` reports no transition coordinator
    /// and `AppKit` skips its fade — while the page's transition host
    /// ramps its own insertion instead (§5.5).
    pub fn navigation_claimed_push_replaces_the_stack_animation() {
        let _mtm = mtm();
        let (path, leaf) = claimed_stack();
        let _window = super::mount_and_order_front(leaf.view());
        let mid_ramp = Cell::new(false);
        #[cfg(target_os = "ios")]
        push_unanimated_ios(&path, &leaf, &mid_ramp);
        #[cfg(target_os = "macos")]
        push_unanimated_macos(&path, &leaf, &mid_ramp);
        assert!(
            mid_ramp.get(),
            "the page host's alpha ramped through its own transition"
        );
    }

    /// `UIKit`: the claimed push installs without a transition
    /// coordinator and the page host's alpha ramps mid-flight.
    #[cfg(target_os = "ios")]
    fn push_unanimated_ios(
        path: &waterui::navigation::NavigationPath<i32>,
        leaf: &waterui_apple::contract::NativeLeaf,
        mid_ramp: &Cell<bool>,
    ) {
        let nav = nav_controller_in(leaf.view()).expect("the stack mounts a navigation controller");
        autoreleasepool(|_| path.push(1));
        assert!(
            pump_main_until(5.0, || nav.viewControllers().count() == 2),
            "the push installs the claimed page"
        );
        assert!(
            nav.transitionCoordinator().is_none(),
            "a claimed push is not the stack's animated transition"
        );
        let done = pump_main_until(5.0, || {
            autoreleasepool(|_| {
                let top = nav.topViewController().expect("the pushed page is top");
                let host = view::subviews(&top.view().expect("view"))
                    .into_iter()
                    .next()
                    .expect("a claimed page mounts inside a transition host");
                let alpha = view::alpha(&host);
                if alpha > 0.0 && alpha < 1.0 {
                    mid_ramp.set(true);
                }
                alpha >= 1.0
            })
        });
        assert!(done, "the insertion transition completes");
    }

    /// `AppKit`: the claimed push's entry host alpha ramps — the stack
    /// fade never ran for it.
    #[cfg(target_os = "macos")]
    fn push_unanimated_macos(
        path: &waterui::navigation::NavigationPath<i32>,
        leaf: &waterui_apple::contract::NativeLeaf,
        mid_ramp: &Cell<bool>,
    ) {
        let host = view::retain_base(leaf.view());
        autoreleasepool(|_| path.push(1));
        let done = pump_main_until(5.0, || {
            autoreleasepool(|_| {
                let subviews = view::subviews(&host);
                if subviews.len() < 2 {
                    return false;
                }
                let alpha = subviews.last().map_or(1.0, |entry| {
                    view::subviews(entry)
                        .first()
                        .map_or(1.0, |page| view::alpha(page))
                });
                if alpha > 0.0 && alpha < 1.0 {
                    mid_ramp.set(true);
                }
                alpha >= 1.0
            })
        });
        assert!(done, "the insertion transition completes");
    }

    /// A flag whose drop proves the leaf's keepalive ended — the same
    /// lifetime every watcher and subscription shares.
    struct Died(Rc<Cell<bool>>);

    impl Drop for Died {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    /// The outgoing child's leaf dies at replacement — the `Retain` probe
    /// proves every subscription ended — while its view is retained as a
    /// ghost beneath the incoming child until the transition clock
    /// releases it. The new child is untouched.
    pub fn a_replaced_child_semantics_die_while_its_view_ghosts() {
        let _mtm = mtm();
        let (handler, dynamic) = Dynamic::new();
        let leaf = gpu_leaf(dynamic);
        let _window = super::mount_and_order_front(leaf.view());

        let died = Rc::new(Cell::new(false));
        // The update path autoreleases `subviews` arrays that retain the
        // tree in the caller's pool — without these pools the weak probe
        // could never see the release.
        autoreleasepool(|_| {
            handler.set(Metadata::new(
                Metadata::new(label("gone"), Retain::new(Died(Rc::clone(&died)))),
                TransitionSpec::new(PropertyTransition::opacity())
                    .with_animation(Animation::linear(Duration::from_millis(60))),
            ));
        });
        let ghost_ptr: Cell<*const PlatformView> = Cell::new(std::ptr::null());
        let live_ptr: Cell<*const PlatformView> = Cell::new(std::ptr::null());
        let weak = autoreleasepool(|_| {
            let subviews = view::subviews(leaf.view());
            assert_eq!(subviews.len(), 1, "the child mounts inside the host");
            ghost_ptr.set(std::ptr::from_ref(&*subviews[0]));
            Weak::new(&*subviews[0])
        });

        autoreleasepool(|_| handler.set(label("here")));
        // The semantic boundary: the old leaf — its watchers, guards and
        // retained values — is dead immediately.
        assert!(died.get(), "the removed leaf's keepalive ended at removal");
        // Its view lives on as the ghost, painted beneath the new child.
        assert!(weak.load().is_some(), "the ghost retains the outgoing view");
        autoreleasepool(|_| {
            let subviews = view::subviews(leaf.view());
            assert_eq!(subviews.len(), 2, "ghost beneath the live child");
            assert_eq!(
                std::ptr::from_ref(&*subviews[0]),
                ghost_ptr.get(),
                "the ghost paints at the bottom of the host"
            );
            assert_ne!(
                std::ptr::from_ref(&*subviews[1]),
                ghost_ptr.get(),
                "the new child mounts on top"
            );
            live_ptr.set(std::ptr::from_ref(&*subviews[1]));
        });

        // The clock releases the ghost when the animation ends — the
        // window is ordered, so the frame clock ticks.
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| view::subviews(leaf.view()).len() == 1)
            }),
            "the completion detaches the ghost when the transition completes"
        );
        assert!(
            pump_main_until(1.0, || weak.load().is_none()),
            "the ghost releases the outgoing view"
        );
        let (count, first) = autoreleasepool(|_| {
            let remaining = view::subviews(leaf.view());
            (remaining.len(), std::ptr::from_ref(&*remaining[0]))
        });
        assert_eq!(
            (count, first),
            (1, live_ptr.get()),
            "only the live child remains"
        );
    }

    /// A retired host's subtree is a ghost: hit testing skips it whole,
    /// it reports no accessibility membership, and the inspector's
    /// published tree has no node inside it.
    pub fn retired_host_is_outside_hit_testing_and_accessibility() {
        use waterui_apple::native_test_support::inspector_tree;

        let mtm = mtm();
        let site = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let _window = super::mount_and_order_front(&site);
        let live = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let ghost = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let label = Label::new(mtm);
        // The live child covers the bottom half; the ghost, added last,
        // covers the top half above it.
        view::add_subview(&site, &live);
        view::set_frame(&live, cocoa_ui::Rect::new(0.0, 50.0, 200.0, 50.0));
        view::add_subview(&ghost, &label);
        view::add_subview(&site, &ghost);
        view::set_frame(&ghost, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 50.0));
        view::set_frame(&label, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 50.0));
        let site_ptr = std::ptr::from_ref(&*site).addr() as u64;
        let ghost_ptr = std::ptr::from_ref(&*ghost).addr() as u64;
        let label_ptr = std::ptr::from_ref(&*label).addr() as u64;
        ghost.retire();

        #[cfg(target_os = "macos")]
        {
            // `hitTest:` reads the point in the superview's coordinates.
            let site_view: &PlatformView = &site;
            let superview = view::superview(site_view).expect("the site is in a window");
            let inside_ghost =
                view::convert_point(site_view, cocoa_ui::Point::new(25.0, 25.0), &superview);
            let hit = site.hitTest(inside_ghost.into());
            assert!(
                hit.as_deref()
                    .is_some_and(|hit| std::ptr::from_ref(hit).addr() as u64 == site_ptr),
                "the retired host yields no hit inside its subtree"
            );
            let inside_live =
                view::convert_point(site_view, cocoa_ui::Point::new(25.0, 75.0), &superview);
            let hit = site.hitTest(inside_live.into());
            assert!(
                hit.as_deref()
                    .is_some_and(|hit| std::ptr::from_ref(hit).addr() as u64 != ghost_ptr),
                "the live child answers hits beneath the ghost's frame"
            );
            autoreleasepool(|_| {
                use cocoa_ui::objc2::msg_send;
                use cocoa_ui::objc2_foundation::NSArray;
                // SAFETY: the query answers an object array.
                let children: Option<Retained<NSArray>> =
                    unsafe { msg_send![&site, accessibilityChildren] };
                if let Some(children) = children {
                    for index in 0..children.count() {
                        let child = children.objectAtIndex(index);
                        let ptr = std::ptr::from_ref(&*child).addr() as u64;
                        assert!(
                            ptr != ghost_ptr && ptr != label_ptr,
                            "the site's accessibility children exclude the ghost"
                        );
                    }
                }
                // SAFETY: the query answers an object array.
                let ghost_children: Option<Retained<NSArray>> =
                    unsafe { msg_send![&ghost, accessibilityChildren] };
                assert_eq!(
                    ghost_children.map_or(0, |children| children.count()),
                    0,
                    "a retired host reports no accessibility children"
                );
                // SAFETY: `isAccessibilityElement` is a plain getter.
                let is_element: bool = unsafe { msg_send![&ghost, isAccessibilityElement] };
                assert!(!is_element, "a retired host is no accessibility element");
            });
        }
        #[cfg(target_os = "ios")]
        {
            use cocoa_ui::objc2_ui_kit::NSObjectUIAccessibility;
            // `hitTest:withEvent:` reads the point in the receiver's
            // bounds coordinates.
            let hit = site.hitTest_withEvent(objc2_core_foundation::CGPoint::new(25.0, 25.0), None);
            assert!(
                hit.as_deref()
                    .is_some_and(|hit| std::ptr::from_ref(hit).addr() as u64 == site_ptr),
                "the retired host yields no hit inside its subtree"
            );
            assert!(
                ghost.accessibilityElementsHidden(mtm),
                "a retired host hides its subtree from accessibility"
            );
            let _ = label_ptr;
        }

        // The inspector's published tree has no node for the ghost
        // subtree.
        for node in inspector_tree(&site) {
            assert_ne!(node.id.0, ghost_ptr, "the ghost is outside the tree");
            assert_ne!(
                node.id.0, label_ptr,
                "the ghost's content is outside the tree"
            );
        }
    }

    /// Dropping a `Mounted` inside a retired host leaves the child's
    /// superview untouched — the retired host owns the visuals now. The
    /// same drop under an unretired parent detaches, as before.
    pub fn mounted_drop_under_retired_host_keeps_views() {
        let mtm = mtm();
        let parent = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let child = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mounted = waterui_apple::contract::NativeLeaf::new(&*child, TestSubView).mount(&parent);
        let child_view = view::retain_base(mounted.view());
        parent.retire();
        drop(mounted);
        assert!(
            view::superview(&child_view).is_some(),
            "the retired host keeps the child's view"
        );

        let live_parent = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let live_child = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mounted =
            waterui_apple::contract::NativeLeaf::new(&*live_child, TestSubView).mount(&live_parent);
        let child_view = view::retain_base(mounted.view());
        drop(mounted);
        assert!(
            view::superview(&child_view).is_none(),
            "an unretired parent detaches the child's view"
        );
    }

    /// A `.transition(...)` beneath another modifier has no structural
    /// owner: rendering `content.transition(..).padding(..)` and mounting
    /// it directly panics in a debug build, naming the missing owner.
    pub fn unowned_declaration_panics_at_mount() {
        use waterui::ViewExt as _;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let leaf = super::resolve::render(
                Metadata::new(
                    label("unowned"),
                    TransitionSpec::new(PropertyTransition::opacity()),
                )
                .padding(),
            );
            let parent = HostView::new(mtm(), cocoa_ui::Rect::ZERO);
            let _mounted = leaf.mount(&parent);
        }));
        let Err(payload) = outcome else {
            panic!("an unowned `.transition(...)` declaration mounted");
        };
        let message = payload.downcast_ref::<String>().map_or_else(
            || {
                payload
                    .downcast_ref::<&'static str>()
                    .copied()
                    .unwrap_or_default()
            },
            String::as_str,
        );
        assert!(
            message.contains("no structural transition owner"),
            "unexpected panic message: {message}"
        );
    }

    /// The transition host of `leaf`'s dynamic — its single subview.
    fn transition_host(child: &Mounted) -> Retained<PlatformView> {
        autoreleasepool(|_| {
            view::subviews(child.view())
                .into_iter()
                .next()
                .expect("the dynamic holds one child")
        })
    }

    /// The content root a transition host frames — its single child.
    fn content_of(host: &PlatformView) -> Retained<PlatformView> {
        autoreleasepool(|_| {
            view::subviews(host)
                .into_iter()
                .next()
                .expect("the transition host frames one content view")
        })
    }

    /// The x-scale `outer` applies to its single child `inner`: `m11` of
    /// `outer`'s layer `sublayerTransform` on macOS — where the transition
    /// host and the scale modifier both put it — and `a` of `inner`'s own
    /// `CGAffineTransform` on iOS.
    fn scale_through(outer: &PlatformView, inner: &PlatformView) -> f64 {
        autoreleasepool(|_| {
            #[cfg(target_os = "macos")]
            {
                let _ = inner;
                let layer = view::layer(outer).expect("a layer-backed view");
                layer.sublayerTransform().m11
            }
            #[cfg(target_os = "ios")]
            {
                let _ = outer;
                inner.transform().a
            }
        })
    }

    /// The transition's scale pivots at the host's centre: the host
    /// layer's `sublayerTransform` maps the bounds centre to itself. iOS
    /// needs no check — `UIView.transform` applies about the centre.
    #[cfg(target_os = "macos")]
    fn assert_centre_fixed(host: &PlatformView) {
        autoreleasepool(|_| {
            let size = view::bounds(host).size;
            let (cx, cy) = (size.width * 0.5, size.height * 0.5);
            let t = view::layer(host)
                .expect("a layer-backed host")
                .sublayerTransform();
            let x = cy.mul_add(t.m21, cx.mul_add(t.m11, t.m41));
            let y = cy.mul_add(t.m22, cx.mul_add(t.m12, t.m42));
            assert!(
                (x - cx).abs() < 1e-6 && (y - cy).abs() < 1e-6,
                "the transition pivots at the centre ({cx}, {cy}), mapped to ({x}, {y})"
            );
        });
    }

    /// Removing a `Dynamic` child kills its semantics but not its views:
    /// every `Retain` in the outgoing subtree drops while every view
    /// stays in the ghost — descendants too, not just the root
    /// (§8.2).
    pub fn removed_descendants_die_but_their_views_stay_in_the_ghost() {
        let _mtm = mtm();
        let d1 = Rc::new(Cell::new(false));
        let d2 = Rc::new(Cell::new(false));
        let (nested, nested_view) = Dynamic::new();
        nested.set(Metadata::new(
            label("inner"),
            Retain::new(Died(Rc::clone(&d2))),
        ));
        let child = vstack((
            Metadata::new(label("first"), Retain::new(Died(Rc::clone(&d1)))),
            nested_view,
        ));
        let (handler, view) = Dynamic::new();
        handler.set(Metadata::new(
            child,
            TransitionSpec::new(PropertyTransition::opacity())
                .with_animation(Animation::linear(Duration::from_millis(60))),
        ));
        // Every kit call below runs in its own pool: the update path
        // autoreleases `subviews` arrays that retain the tree, and the weak
        // probe sees the release only once they drain.
        let (mounted, window) = autoreleasepool(|_| {
            let leaf = super::resolve::render(view);
            let mounted = leaf.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
            let window = super::mount_and_order_front(mounted.view());
            (mounted, window)
        });
        assert!(!d1.get() && !d2.get(), "the mounted child's Retains live");
        autoreleasepool(|_| handler.set(label("after")));
        assert!(d1.get(), "the removed child's Retain dies at the swap");
        assert!(d2.get(), "the nested Dynamic's Retain dies at the swap");
        let weak: Weak<PlatformView> = autoreleasepool(|_| {
            let ghost = transition_host(&mounted);
            let content = content_of(&ghost);
            let descendant = view::subviews(&content)
                .into_iter()
                .next()
                .expect("the ghost keeps the stack's children");
            assert!(
                view::superview(&descendant).is_some(),
                "the removed descendant's view stays in the ghost"
            );
            Weak::from(&descendant)
        });
        let completed = pump_main_until(2.0, || {
            autoreleasepool(|_| view::subviews(mounted.view()).len() == 1)
        });
        assert!(completed, "the ghost completes on the frame clock");
        assert!(
            pump_main_until(1.0, || weak.load().is_none()),
            "the ghost releases every view"
        );
        drop(window);
    }

    /// `PropertyTransition::opacity()` samples the transition host's
    /// alpha — the content's own `.opacity` is never touched (§8.2).
    pub fn opacity_composes_with_the_contents_own_opacity() {
        let _mtm = mtm();
        let (handler, view) = Dynamic::new();
        handler.set(label("before"));
        let leaf = super::resolve::render(view);
        let mounted = leaf.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
        let window = super::mount_and_order_front(mounted.view());
        handler.set(Metadata::new(
            label("coming").opacity(0.5),
            TransitionSpec::new(PropertyTransition::opacity())
                .with_animation(Animation::linear(Duration::from_millis(60))),
        ));
        autoreleasepool(|_| {
            let host = transition_host(&mounted);
            let content = content_of(&host);
            let mid = pump_main_until(2.0, || {
                let alpha = view::alpha(&host);
                alpha > 0.0 && alpha < 1.0
            });
            assert!(mid, "the insertion samples the host's alpha mid-flight");
            assert!(
                (view::alpha(&content) - 0.5).abs() < 1e-6,
                "the content's own opacity is untouched"
            );
            let settled = pump_main_until(2.0, || (view::alpha(&host) - 1.0).abs() < 1e-6);
            assert!(settled, "the insertion completes to full alpha");
            assert!((view::alpha(&content) - 0.5).abs() < 1e-6);
        });
        drop(window);
    }

    /// `PropertyTransition::scale` samples the transition child's
    /// transform — the content's own `.scale` is never touched (§8.2).
    pub fn scale_composes_with_the_contents_own_scale() {
        let _mtm = mtm();
        let (handler, view) = Dynamic::new();
        handler.set(label("before"));
        let leaf = super::resolve::render(view);
        let mounted = leaf.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
        let window = super::mount_and_order_front(mounted.view());
        handler.set(Metadata::new(
            label("coming").scale(0.5, 0.5),
            TransitionSpec::new(PropertyTransition::scale(0.5))
                .with_animation(Animation::linear(Duration::from_millis(60))),
        ));
        autoreleasepool(|_| {
            let host = transition_host(&mounted);
            let transition_child = content_of(&host);
            let inner = view::subviews(&transition_child)
                .into_iter()
                .next()
                .expect("the scale wrapper holds its child");
            let mid = pump_main_until(2.0, || {
                let scale = scale_through(&host, &transition_child);
                scale > 0.5 && scale < 1.0
            });
            assert!(mid, "the insertion samples the transition child's scale");
            #[cfg(target_os = "macos")]
            assert_centre_fixed(&host);
            let own = scale_through(&transition_child, &inner);
            assert!(
                (own - 0.5).abs() < 1e-6,
                "the content's own scale is untouched mid-flight, read {own}"
            );
            let settled = pump_main_until(2.0, || {
                (scale_through(&host, &transition_child) - 1.0).abs() < 1e-6
            });
            assert!(settled, "the insertion completes to the identity");
            let own = scale_through(&transition_child, &inner);
            assert!(
                (own - 0.5).abs() < 1e-6,
                "the content's own scale is untouched at rest, read {own}"
            );
        });
        drop(window);
    }

    /// A combined transition samples every property on every tick: the
    /// host alpha and the transition child's scale share one clock's
    /// progress, within a point sample's tolerance (§8.2).
    pub fn combined_transition_samples_every_frame() {
        let _mtm = mtm();
        let (handler, view) = Dynamic::new();
        handler.set(label("before"));
        let leaf = super::resolve::render(view);
        let mounted = leaf.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
        let window = super::mount_and_order_front(mounted.view());
        handler.set(Metadata::new(
            label("coming"),
            TransitionSpec::new(
                PropertyTransition::opacity().combined(PropertyTransition::scale(0.5)),
            )
            .with_animation(Animation::linear(Duration::from_millis(120))),
        ));
        autoreleasepool(|_| {
            let host = transition_host(&mounted);
            let transition_child = content_of(&host);
            let shared = Cell::new(false);
            let mid = pump_main_until(2.0, || {
                let alpha = view::alpha(&host);
                let scale = scale_through(&host, &transition_child);
                if alpha > 0.0 && alpha < 1.0 {
                    // Insertion progress implied by the scale sample.
                    let progress = 2.0 * (1.0 - scale);
                    assert!(
                        (alpha - (1.0 - progress)).abs() < 1e-3,
                        "alpha {alpha} and scale {scale} share one progress"
                    );
                    shared.set(true);
                }
                shared.get()
            });
            assert!(mid, "a combined tick landed mid-flight");
        });
        drop(window);
    }

    /// `.transition(...)` directly on a `Dynamic` adopts into the
    /// conditional: each child it swaps carries the claim — the swap
    /// ghosts the outgoing child while the `Dynamic` host itself is
    /// never retired (§5.1, §8.2).
    pub fn adopted_conditional_claims_each_child() {
        let _mtm = mtm();
        let (handler, view) = Dynamic::new();
        let leaf_view = Metadata::new(
            view,
            TransitionSpec::new(PropertyTransition::opacity())
                .with_animation(Animation::linear(Duration::from_millis(60))),
        );
        let leaf = super::resolve::render(leaf_view);
        let mounted = leaf.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
        let window = super::mount_and_order_front(mounted.view());
        handler.set(label("a"));
        handler.set(label("b"));
        autoreleasepool(|_| {
            let host = mounted
                .view()
                .downcast_ref::<HostView>()
                .expect("a HostView");
            assert!(
                !host.is_retired(),
                "the conditional's own mount never transitions"
            );
            assert_eq!(
                view::subviews(mounted.view()).len(),
                2,
                "the adopted claim ghosts the outgoing child"
            );
            let completed = pump_main_until(2.0, || view::subviews(mounted.view()).len() == 1);
            assert!(completed, "the adopted ghost completes");
        });
        drop(window);
    }

    /// A child's own `.transition(...)` declaration wins over the claim
    /// the conditional adopted: the outgoing child ghosts on its own
    /// scale, the host alpha is untouched (§5.1, §8.2).
    pub fn the_contents_own_claim_beats_the_adopted_one() {
        let _mtm = mtm();
        let (handler, view) = Dynamic::new();
        let leaf_view = Metadata::new(
            view,
            TransitionSpec::new(PropertyTransition::opacity())
                .with_animation(Animation::linear(Duration::from_millis(60))),
        );
        let leaf = super::resolve::render(leaf_view);
        let mounted = leaf.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
        let window = super::mount_and_order_front(mounted.view());
        handler.set(Metadata::new(
            label("a"),
            TransitionSpec::new(PropertyTransition::scale(0.5))
                .with_animation(Animation::linear(Duration::from_millis(60))),
        ));
        handler.set(label("b"));
        autoreleasepool(|_| {
            let host = transition_host(&mounted);
            let transition_child = content_of(&host);
            let mid = pump_main_until(2.0, || scale_through(&host, &transition_child) < 1.0);
            assert!(mid, "the child's own claim drives the ghost");
            assert!(
                (view::alpha(&host) - 1.0).abs() < 1e-6,
                "the adopted opacity never applies to a claimed child"
            );
        });
        drop(window);
    }

    /// The first-mounted child of a conditional is a mount, not an
    /// insertion: its transition host starts fully present (§5.2, §8.2).
    pub fn initial_build_plays_no_insertion() {
        let _mtm = mtm();
        let (handler, view) = Dynamic::new();
        let leaf = super::resolve::render(view);
        let mounted = leaf.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
        let window = super::mount_and_order_front(mounted.view());
        handler.set(Metadata::new(
            label("a"),
            TransitionSpec::new(PropertyTransition::opacity())
                .with_animation(Animation::linear(Duration::from_millis(60))),
        ));
        autoreleasepool(|_| {
            let host = transition_host(&mounted);
            assert!(
                (view::alpha(&host) - 1.0).abs() < 1e-6,
                "the initial child presents at full alpha before any pump"
            );
        });
        drop(window);
    }

    /// A zero-duration removal completes at `start`: the outgoing
    /// instance detaches in the same turn, with no ghost at all
    /// (§1.5, §8.2).
    pub fn zero_duration_removal_detaches_at_once() {
        let _mtm = mtm();
        let (handler, view) = Dynamic::new();
        handler.set(Metadata::new(
            label("a"),
            TransitionSpec::new(PropertyTransition::opacity())
                .with_animation(Animation::linear(Duration::ZERO)),
        ));
        let leaf = super::resolve::render(view);
        let mounted = leaf.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
        let window = super::mount_and_order_front(mounted.view());
        handler.set(label("b"));
        autoreleasepool(|_| {
            assert_eq!(
                view::subviews(mounted.view()).len(),
                1,
                "a zero-duration removal leaves no ghost behind"
            );
        });
        drop(window);
    }

    /// Completing a ghost needs nothing the site owns: dropping the
    /// `Dynamic` leaf cancels the run through the window handler's
    /// path, and a live site still sees the ordinary completion
    /// (§1.5, §8.2).
    pub fn ghost_completion_does_not_need_the_site() {
        let _mtm = mtm();
        let (handler, view) = Dynamic::new();
        handler.set(Metadata::new(
            label("a"),
            TransitionSpec::new(PropertyTransition::opacity())
                .with_animation(Animation::linear(Duration::from_millis(60))),
        ));
        // Every kit call below runs in its own pool: the update path
        // autoreleases `subviews` arrays that retain the tree, and the weak
        // probe sees the release only once they drain.
        let (mounted, window) = autoreleasepool(|_| {
            let leaf = super::resolve::render(view);
            let mounted = leaf.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
            let window = super::mount_and_order_front(mounted.view());
            (mounted, window)
        });
        autoreleasepool(|_| handler.set(label("b")));
        let weak: Weak<PlatformView> = autoreleasepool(|_| {
            let ghost = transition_host(&mounted);
            Weak::from(&ghost)
        });
        autoreleasepool(|_| drop(mounted));
        let cancelled = pump_main_until(2.0, || weak.load().is_none());
        assert!(
            cancelled,
            "the site's drop cancels the ghost within one pump"
        );

        // With the site live, the ghost completes through `on_complete`.
        let (handler2, view2) = Dynamic::new();
        handler2.set(Metadata::new(
            label("a"),
            TransitionSpec::new(PropertyTransition::opacity())
                .with_animation(Animation::linear(Duration::from_millis(30))),
        ));
        let leaf2 = super::resolve::render(view2);
        let mounted2 = leaf2.mount(&HostView::new(mtm(), cocoa_ui::Rect::ZERO));
        let window2 = super::mount_and_order_front(mounted2.view());
        handler2.set(label("b"));
        autoreleasepool(|_| {
            let completed = pump_main_until(2.0, || view::subviews(mounted2.view()).len() == 1);
            assert!(completed, "the ghost completes through on_complete");
        });
        drop(window2);
        drop(window);
    }

    /// A stable-id row for the `ForEach` memberships the collection
    /// trials edit.
    #[derive(Clone, Copy, Identifiable)]
    struct Row {
        #[id]
        id: u64,
    }

    /// The `Died` flags a row generator records, in render order, keyed
    /// by the row's collection id — reinsertion shows as a second entry.
    type DiedFlags = Rc<RefCell<Vec<(u64, Rc<Cell<bool>>)>>>;

    /// A 200 ms linear opacity declaration — long enough for a
    /// mid-flight membership edit.
    fn row_spec() -> TransitionSpec {
        TransitionSpec::new(PropertyTransition::opacity())
            .with_animation(Animation::linear(Duration::from_millis(200)))
    }

    /// The container's members as (is ghost, pointer, frame) triples in
    /// subview order — ghost hosts paint beneath the live members.
    fn member_frames(container: &PlatformView) -> Vec<(bool, usize, cocoa_ui::Rect)> {
        autoreleasepool(|_| {
            view::subviews(container)
                .iter()
                .map(|member| {
                    (
                        member
                            .downcast_ref::<HostView>()
                            .is_some_and(HostView::is_retired),
                        Retained::as_ptr(member) as usize,
                        view::frame(member),
                    )
                })
                .collect()
        })
    }

    /// The members sorted by frame origin — the member order layout
    /// produced, independent of subview z-order.
    fn members_by_origin(container: &PlatformView) -> Vec<(bool, usize, cocoa_ui::Rect)> {
        let mut members = member_frames(container);
        members.sort_by(|a, b| a.2.origin.y.total_cmp(&b.2.origin.y));
        members
    }

    /// True while any descendant of `view` is a retired host — the sign
    /// a ghost exists.
    fn has_retired_descendant(view: &PlatformView) -> bool {
        autoreleasepool(|_| {
            view::subviews(view).iter().any(|subview| {
                subview
                    .downcast_ref::<HostView>()
                    .is_some_and(HostView::is_retired)
                    || has_retired_descendant(subview)
            })
        })
    }

    /// The host that directly owns the member views: `VStack::for_each`
    /// wraps its container in an environment host and `Lazy::vstack`
    /// nests it in scroll chrome, so in both cases the container is the
    /// first descendant with more than two direct subviews.
    fn member_container(view: &PlatformView) -> Option<Retained<PlatformView>> {
        autoreleasepool(|_| {
            let subviews = view::subviews(view);
            if subviews.len() > 2 {
                return Some(view::retain_base(view));
            }
            subviews
                .iter()
                .find_map(|subview| member_container(subview))
        })
    }

    /// The ghost slot reports exactly the member's last placed frame —
    /// bit-equal, since it is copied — and a mid-flight width change
    /// re-measures live members but never the ghost.
    pub fn collection_ghost_keeps_its_last_placement() {
        let _mtm = mtm();
        let mut items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
        let leaf = super::resolve::render(VStack::for_each(items.clone(), |row: Row| {
            let height = match row.id % 3 {
                0 => 20.0,
                1 => 4.0,
                _ => 12.0,
            };
            Metadata::new(
                // The spacer stretches the row across the container's
                // width, so a wider container re-measures it.
                hstack((text(format!("row {}", row.id)), spacer())).padding_vertical(height),
                row_spec(),
            )
        }));
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        let container = member_container(leaf.view()).expect("the container holds the members");
        let (b_size, c_origin) = {
            let members = member_frames(&container);
            assert_eq!(members.len(), 3, "three members mounted");
            (members[1].2.size, members[2].2.origin)
        };
        autoreleasepool(|_| {
            let _ = items.remove(1);
        });
        assert!(
            pump_main_until(2.0, || member_frames(&container).iter().any(|m| m.0)),
            "the removed member ghosts"
        );
        let members = member_frames(&container);
        assert_eq!(members.len(), 3);
        let (ghost, _, frame) = members[0];
        assert!(ghost, "the ghost leads the subviews at the bottom");
        assert_eq!(
            (frame.size.width.to_bits(), frame.size.height.to_bits()),
            (b_size.width.to_bits(), b_size.height.to_bits()),
            "the ghost's frame is the member's last placement, bit-equal"
        );
        assert_eq!(
            members[2].2.origin.y.to_bits(),
            c_origin.y.to_bits(),
            "the surviving member's origin is unchanged while the slot holds"
        );
        // A mid-flight width change re-measures live members; the ghost
        // keeps its frozen size. The container takes the new bounds
        // directly — its `with` wrapper would only forward them anyway.
        let live_width = members[2].2.size.width;
        autoreleasepool(|_| {
            let mut frame = view::frame(&container);
            frame.size.width += 80.0;
            view::set_frame(&container, frame);
            view::layout_immediately(&container);
        });
        let resized = member_frames(&container);
        assert!(
            resized[2].2.size.width > live_width,
            "the live member re-measured at the new width"
        );
        let (ghost, _, ghost_frame) = resized[0];
        assert!(ghost, "the ghost still leads the subviews");
        assert_eq!(
            (
                ghost_frame.size.width.to_bits(),
                ghost_frame.size.height.to_bits()
            ),
            (b_size.width.to_bits(), b_size.height.to_bits()),
            "the ghost's size is frozen — never re-measured"
        );
    }

    /// A ghost re-anchors on every edit: it stays after the nearest
    /// preceding live id, or at the head when none survives — and a head
    /// ghost stays ahead of an insertion at the head, as in Hydrolysis.
    pub fn ghosts_rebase_across_later_edits() {
        let _mtm = mtm();
        let mut items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
        // Only row 2 claims a transition — an anchor that ghosts would
        // make "first" ambiguous.
        let leaf = super::resolve::render(VStack::for_each(items.clone(), |row: Row| {
            let content = hstack((text(format!("row {}", row.id)), spacer())).size(200.0, 20.0);
            if row.id == 2 {
                AnyView::new(Metadata::new(content, row_spec()))
            } else {
                AnyView::new(content)
            }
        }));
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        let container = member_container(leaf.view()).expect("the container holds the members");
        autoreleasepool(|_| {
            let _ = items.remove(1);
        });
        assert!(
            pump_main_until(2.0, || member_frames(&container).iter().any(|m| m.0)),
            "the removed member ghosts"
        );
        autoreleasepool(|_| {
            let _ = items.remove(0);
        });
        // With its anchor gone the ghost sits first.
        let members = member_frames(&container);
        assert_eq!(members.len(), 2);
        let (ghost, _, ghost_frame) = members[0];
        assert!(ghost, "the ghost leads the subviews");
        assert!(
            ghost_frame.origin.y < members[1].2.origin.y,
            "the head-anchored ghost sits first"
        );
        // The surviving member's identity, to name it after the insert.
        let c_ptr = members
            .iter()
            .find(|m| !m.0)
            .expect("the survivor is live")
            .1;
        // Inserting at the head keeps the head ghost first.
        autoreleasepool(|_| {
            items.insert(0, Row { id: 4 });
        });
        assert!(
            pump_main_until(2.0, || member_frames(&container).len() == 3),
            "the inserted member joins"
        );
        let members = members_by_origin(&container);
        assert_eq!(members.len(), 3);
        // By frame origin: the head ghost first, then the inserted
        // member, then the survivor.
        assert!(
            members[0].0 && !members[1].0 && !members[2].0,
            "member order by frame origin is ghost(B), D, C: {members:?}"
        );
        assert_eq!(members[2].1, c_ptr, "the survivor keeps the tail slot");
        assert_ne!(
            members[1].1, c_ptr,
            "the member after the ghost is the fresh insertion, not the survivor"
        );
    }

    /// Re-adding an id whose exit is still in flight joins a fresh live
    /// instance — new semantics, new insertion — while the old ghost
    /// completes on its own clock.
    pub fn reinsertion_creates_a_fresh_instance_beside_the_ghost() {
        let _mtm = mtm();
        let mut items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
        let flags: DiedFlags = Rc::new(RefCell::new(Vec::new()));
        let leaf = super::resolve::render(VStack::for_each(items.clone(), {
            let flags = Rc::clone(&flags);
            move |row: Row| {
                let died = Rc::new(Cell::new(false));
                flags.borrow_mut().push((row.id, Rc::clone(&died)));
                Metadata::new(
                    Metadata::new(
                        hstack((text(format!("row {}", row.id)), spacer())).size(200.0, 20.0),
                        Retain::new(Died(died)),
                    ),
                    row_spec(),
                )
            }
        }));
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        let container = member_container(leaf.view()).expect("the container holds the members");
        let old_died = flags
            .borrow()
            .iter()
            .find(|(id, _)| *id == 2)
            .map(|(_, died)| Rc::clone(died))
            .expect("row 2 rendered");
        autoreleasepool(|_| {
            let _ = items.remove(1);
        });
        assert!(
            old_died.get(),
            "the removed instance's semantics end at once"
        );
        assert!(
            pump_main_until(2.0, || member_frames(&container).iter().any(|m| m.0)),
            "the removed member ghosts"
        );
        // The same id comes back mid-flight.
        autoreleasepool(|_| {
            items.insert(1, Row { id: 2 });
        });
        assert!(
            pump_main_until(2.0, || member_frames(&container).len() == 4),
            "the reinserted member joins beside the ghost"
        );
        let members = member_frames(&container);
        assert_eq!(members.len(), 4, "the ghost beside three live members");
        assert!(members[0].0, "the ghost paints beneath the live members");
        let new_died = flags
            .borrow()
            .iter()
            .rfind(|(id, _)| *id == 2)
            .map(|(_, died)| Rc::clone(died))
            .expect("row 2 re-rendered");
        assert!(
            !new_died.get(),
            "the reinserted instance is a fresh live member"
        );
        assert!(
            !members
                .iter()
                .filter(|m| !m.0)
                .any(|m| m.2.size.height.to_bits() != 20.0f64.to_bits()),
            "the live members keep their frames"
        );
    }

    /// A lazy row that never materialized releases at once: removing it
    /// creates no ghost host, and the container's extent reflows on the
    /// next pass.
    pub fn unplaced_lazy_row_releases_immediately() {
        let _mtm = mtm();
        let rows: Vec<Row> = (0..40).map(|id| Row { id }).collect();
        let mut items = ReactiveList::from(rows);
        let leaf = super::resolve::render(Lazy::vstack(ForEach::new(items.clone(), |row: Row| {
            Metadata::new(
                hstack((text(format!("row {}", row.id)), spacer())).size(300.0, 30.0),
                row_spec(),
            )
        })));
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        let (container, height) = {
            let container =
                member_container(leaf.view()).expect("the lazy container materialized rows");
            let count = autoreleasepool(|_| view::subviews(&container).len());
            assert!(
                count < 40,
                "the window virtualizes: {count} materialized of 40"
            );
            let height = view::frame(&container).size.height;
            (container, height)
        };
        // A row past the window plus overscan never materialized.
        autoreleasepool(|_| {
            let _ = items.remove(39);
        });
        assert!(
            !pump_main_until(0.5, || has_retired_descendant(leaf.view())),
            "an unmaterialized row leaves no ghost"
        );
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        assert!(
            view::frame(&container).size.height < height,
            "the membership reflowed to one fewer row"
        );
    }

    /// The ghost's slot holds the gap until the clock ends — siblings
    /// move into it only when `on_complete` releases the slot.
    pub fn siblings_close_the_gap_at_completion() {
        let _mtm = mtm();
        let mut items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
        // Only the middle row claims a transition.
        let leaf = super::resolve::render(VStack::for_each(items.clone(), |row: Row| {
            let height = match row.id {
                1 => 10.0,
                2 => 30.0,
                _ => 20.0,
            };
            let content = hstack((text(format!("row {}", row.id)), spacer())).size(200.0, height);
            if row.id == 2 {
                AnyView::new(Metadata::new(content, row_spec()))
            } else {
                AnyView::new(content)
            }
        }));
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        let container = member_container(leaf.view()).expect("the container holds the members");
        let b_origin = member_frames(&container)[1].2.origin.y;
        autoreleasepool(|_| {
            let _ = items.remove(1);
        });
        assert!(
            pump_main_until(2.0, || member_frames(&container).iter().any(|m| m.0)),
            "the removed member ghosts"
        );
        // Completion drops the ghost and the slot; the trailing member's
        // origin lands exactly where the removed member began.
        assert!(
            pump_main_until(3.0, || {
                autoreleasepool(|_| view::layout_immediately(leaf.view()));
                let members = member_frames(&container);
                members.len() == 2 && members[1].2.origin.y.to_bits() == b_origin.to_bits()
            }),
            "siblings close the gap once the ghost completes"
        );
    }

    /// #3509: a membership insertion before existing members lands at
    /// the head of the slot sequence while its view mounts at the end —
    /// the host's subview order (paint order) must still equal item
    /// order, as the fixed path's `reconcile_subviews` keeps it.
    pub fn an_insert_before_existing_members_paints_in_item_order() {
        let _mtm = mtm();
        let mut items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
        let leaf = super::resolve::render(VStack::for_each(items.clone(), |row: Row| {
            hstack((text(format!("row {}", row.id)), spacer())).size(200.0, 20.0)
        }));
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        let container = member_container(leaf.view()).expect("the container holds the members");
        autoreleasepool(|_| {
            items.insert(0, Row { id: 0 });
        });
        assert!(
            pump_main_until(2.0, || member_frames(&container).len() == 4),
            "the inserted member joins"
        );
        // The subviews are the paint order, back to front; a vertical
        // stack's item order is increasing y. The inserted row's frame
        // leads — before the fix its view still painted above every
        // member materialized earlier.
        let paint_order: Vec<usize> = member_frames(&container)
            .iter()
            .map(|member| member.1)
            .collect();
        let item_order: Vec<usize> = members_by_origin(&container)
            .iter()
            .map(|member| member.1)
            .collect();
        assert_eq!(
            paint_order, item_order,
            "the host's subviews paint in item order"
        );
    }

    /// The list's table inside `view`'s subtree, `view` included —
    /// the table may be the leaf's own view or nested under it.
    fn find_table(view: &PlatformView) -> Option<Retained<PlatformView>> {
        if table_is(view) {
            return Some(view::retain_base(view));
        }
        view::subviews(view).iter().find_map(|sub| find_table(sub))
    }

    #[cfg(target_os = "ios")]
    fn table_is(view: &PlatformView) -> bool {
        view.downcast_ref::<cocoa_ui::uikit::TableView>().is_some()
    }

    #[cfg(target_os = "macos")]
    fn table_is(view: &PlatformView) -> bool {
        view.downcast_ref::<cocoa_ui::appkit::ListTableView>()
            .is_some()
    }

    /// The table's displayed row count — ghosts included.
    fn table_row_count(table: &PlatformView) -> usize {
        autoreleasepool(|_| {
            #[cfg(target_os = "ios")]
            // SAFETY: `numberOfRowsInSection:` is safe on a `UITableView`.
            unsafe {
                objc2::msg_send![table, numberOfRowsInSection: 0usize]
            }
            #[cfg(target_os = "macos")]
            {
                table
                    .downcast_ref::<cocoa_ui::appkit::ListTableView>()
                    .expect("the list mounts a ListTableView")
                    .row_count()
            }
        })
    }

    /// Display row `row`'s height — the frozen-height check.
    fn table_row_height(table: &PlatformView, row: usize) -> f64 {
        autoreleasepool(|_| {
            #[cfg(target_os = "ios")]
            {
                let table = table
                    .downcast_ref::<cocoa_ui::uikit::TableView>()
                    .expect("the list mounts a UITableView");
                let index = |row| cocoa_ui::uikit::IndexPath { section: 0, row };
                // Contiguous rows: a row's height is the distance to the
                // previous row's bottom edge.
                assert!(
                    row > 0,
                    "the height read needs the previous row's bottom edge"
                );
                table.row_bottom(index(row)) - table.row_bottom(index(row - 1))
            }
            #[cfg(target_os = "macos")]
            {
                table
                    .downcast_ref::<cocoa_ui::appkit::ListTableView>()
                    .expect("the list mounts a ListTableView")
                    .rect_of_row(row)
                    .size
                    .height
            }
        })
    }

    /// Whether the table's delegate allows `row` to be selected —
    /// `tableView:willSelectRowAtIndexPath:` on `UIKit`,
    /// `tableView:shouldSelectRow:` on `AppKit`.
    fn row_selectable(table: &PlatformView, row: usize) -> bool {
        autoreleasepool(|_| {
            #[cfg(target_os = "ios")]
            // SAFETY: the table's delegate answers the selection
            // callbacks on the main thread.
            unsafe {
                use cocoa_ui::objc2_foundation::NSIndexPath;
                use objc2::runtime::AnyObject;
                let delegate: Option<Retained<AnyObject>> = objc2::msg_send![table, delegate];
                let delegate = delegate.expect("the table carries a source delegate");
                // `NSIndexPath` stores its components in order — section
                // 0, then `row`.
                let mut components = [0usize, row];
                let index = NSIndexPath::indexPathWithIndexes_length(components.as_mut_ptr(), 2);
                let answer: Option<Retained<NSIndexPath>> = objc2::msg_send![
                    &*delegate,
                    tableView: table,
                    willSelectRowAtIndexPath: &*index
                ];
                answer.is_some()
            }
            #[cfg(target_os = "macos")]
            {
                use objc2::runtime::AnyObject;
                let inner = table
                    .downcast_ref::<cocoa_ui::appkit::ListTableView>()
                    .expect("the list mounts a ListTableView")
                    .table_view();
                // SAFETY: the table's delegate answers the selection
                // callbacks on the main thread.
                unsafe {
                    let delegate: Option<Retained<AnyObject>> = objc2::msg_send![&*inner, delegate];
                    let delegate = delegate.expect("the table carries a source delegate");
                    objc2::msg_send![
                        &*delegate,
                        tableView: &*inner,
                        shouldSelectRow: row.cast_signed()
                    ]
                }
            }
        })
    }

    /// The content the table materialized at `row` — a ghost row answers
    /// its retired host; a live row its `LiveChild` view.
    fn cell_content(table: &PlatformView, row: usize) -> Option<Retained<PlatformView>> {
        autoreleasepool(|_| {
            #[cfg(target_os = "ios")]
            {
                use cocoa_ui::objc2_foundation::NSArray;
                use cocoa_ui::objc2_ui_kit::UITableViewCell;
                use cocoa_ui::uikit::TableCell;
                let table_view = table
                    .downcast_ref::<cocoa_ui::uikit::TableView>()
                    .expect("the list mounts a UITableView");
                // SAFETY: `visibleCells` is safe on a `UITableView`.
                let cells: Option<Retained<NSArray<UITableViewCell>>> =
                    unsafe { objc2::msg_send![table_view, visibleCells] };
                cells.and_then(|cells| {
                    cells.iter().find_map(|cell| {
                        let cell = cell.downcast_ref::<TableCell>()?;
                        let index = table_view.index_path_for_cell(cell)?;
                        (index.section == 0 && index.row == row)
                            .then(|| cell.content().map(|content| view::retain_base(&content)))
                            .flatten()
                    })
                })
            }
            #[cfg(target_os = "macos")]
            {
                let inner = table
                    .downcast_ref::<cocoa_ui::appkit::ListTableView>()
                    .expect("the list mounts a ListTableView")
                    .table_view();
                // A row the table has not applied yet has no cell — the
                // contents notification lands on a later turn — and
                // `viewAtColumn:row:makeIfNecessary:` raises on it.
                if row.cast_signed() >= inner.numberOfRows() {
                    return None;
                }
                // SAFETY: `viewAtColumn:row:makeIfNecessary:` is safe on an
                // `NSTableView` on the main thread for a row inside
                // `numberOfRows`, checked above.
                let container: Option<Retained<PlatformView>> = unsafe {
                    objc2::msg_send![
                        &*inner,
                        viewAtColumn: 0isize,
                        row: row.cast_signed(),
                        makeIfNecessary: true
                    ]
                };
                container.and_then(|container| {
                    container
                        .downcast_ref::<cocoa_ui::appkit::RowContainer>()
                        .and_then(|container| {
                            container
                                .content()
                                .map(|content| view::retain_base(&content))
                        })
                })
            }
        })
    }

    /// Whether `view` is out of the accessibility tree — the retired
    /// host's own mark.
    fn accessibility_hidden(view: &PlatformView) -> bool {
        #[cfg(target_os = "ios")]
        // SAFETY: `accessibilityElementsHidden` is safe on a `UIView`.
        unsafe {
            objc2::msg_send![view, accessibilityElementsHidden]
        }
        #[cfg(target_os = "macos")]
        // SAFETY: `isAccessibilityElement` is safe on an `NSView`.
        unsafe {
            !objc2::msg_send![view, isAccessibilityElement]
        }
    }

    /// A single-selection list whose every row claims the row transition
    /// — fixed-height rows, so the frozen height is a real contract.
    fn row_list(items: &ReactiveList<Row>) -> AnyView {
        use waterui::component::list::{List, ListItem};
        use waterui::reactive::binding;
        AnyView::new(
            List::for_each(items.clone(), |row: Row| {
                let content = hstack((text(format!("row {}", row.id)), spacer())).size(200.0, 44.0);
                ListItem::new(Metadata::new(content, row_spec()))
            })
            .selection(&binding(Option::<u64>::None)),
        )
    }

    /// §8 row 4: removing a claimed row ghosts it at its frozen measured
    /// height — the table's row count holds through the exit,
    /// `heightForRow`/`heightOfRow` at the ghost's row answers the removed
    /// row's last measurement, and completion deletes the row.
    pub fn list_ghost_row_keeps_its_frozen_height() {
        let _mtm = mtm();
        let mut items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
        let leaf = super::resolve::render(row_list(&items));
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        let table = autoreleasepool(|_| find_table(leaf.view()).expect("the list mounts a table"));
        let before = table_row_height(&table, 1);
        assert_eq!(table_row_count(&table), 3, "three live rows mounted");
        autoreleasepool(|_| {
            let _ = items.remove(1);
        });
        assert_eq!(
            table_row_count(&table),
            3,
            "the ghost holds the removed row's slot",
        );
        assert_eq!(
            table_row_height(&table, 1).to_bits(),
            before.to_bits(),
            "the ghost row keeps the removed row's measured height",
        );
        assert!(
            pump_main_until(3.0, || table_row_count(&table) == 2),
            "the ghost row deletes when its transition completes",
        );
    }

    /// §8 row 4: `willSelectRow`/`shouldSelectRow` refuse the ghost index,
    /// and the ghost cell's content is the retired host — out of the
    /// accessibility tree.
    pub fn list_ghost_row_is_not_selectable_or_accessible() {
        let _mtm = mtm();
        let mut items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
        let leaf = super::resolve::render(row_list(&items));
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        let table = autoreleasepool(|_| find_table(leaf.view()).expect("the list mounts a table"));
        autoreleasepool(|_| {
            let _ = items.remove(1);
        });
        assert!(
            pump_main_until(2.0, || !row_selectable(&table, 1)),
            "the ghost row is not selectable",
        );
        assert!(row_selectable(&table, 0), "a live row stays selectable");
        let hidden = autoreleasepool(|_| {
            cell_content(&table, 1).is_some_and(|content| {
                content
                    .downcast_ref::<HostView>()
                    .is_some_and(HostView::is_retired)
                    && accessibility_hidden(&content)
            })
        });
        assert!(
            hidden,
            "the ghost cell's retired host stays out of the accessibility tree",
        );
    }

    /// §8 row 4: a claimed insertion materializes its row at once — the
    /// batch animates it with `None` — while the mounted `LiveChild`'s
    /// own transition ramps the host alpha from the absent sample.
    pub fn list_claimed_insertion_plays_without_row_animation() {
        let _mtm = mtm();
        let mut items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }, Row { id: 3 }]);
        let leaf = super::resolve::render(row_list(&items));
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| view::layout_immediately(leaf.view()));
        let table = autoreleasepool(|_| find_table(leaf.view()).expect("the list mounts a table"));
        autoreleasepool(|_| {
            items.push(Row { id: 4 });
        });
        // The inserted row's cell is configured immediately — the table
        // adds it with no row animation — while the transition host's
        // alpha ramps up from the absent sample.
        let saw_ramp = Cell::new(false);
        assert!(
            pump_main_until(3.0, || {
                autoreleasepool(|_| {
                    let Some(host) = cell_content(&table, 3) else {
                        return false;
                    };
                    let alpha = view::alpha(&host);
                    saw_ramp.set(saw_ramp.get() || alpha < 1.0);
                    alpha >= 1.0
                })
            }),
            "the inserted row's transition plays to full opacity",
        );
        assert!(
            saw_ramp.get(),
            "the host alpha ramped from the absent sample — the row itself was inserted without animation",
        );
    }

    /// The Stage 8 blur trials (§8.2): `PropertyTransition::blur` runs
    /// through the filtered GPU pipeline — a live capture for the
    /// insertion, the frozen exit capture for the removal, one lazy
    /// effect per instance.
    #[cfg(feature = "gpu_surface")]
    fn blur_trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "transition::blur_insertion_renders_through_the_live_effect",
                || {
                    blur_insertion_renders_through_the_live_effect();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::blur_removal_renders_from_the_frozen_capture",
                || {
                    blur_removal_renders_from_the_frozen_capture();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::blur_returning_to_zero_keeps_one_effect",
                || {
                    blur_returning_to_zero_keeps_one_effect();
                    Ok(())
                },
            ),
        ]
    }

    /// The Stage 9 pixel trials (§8.2): `captures_content` transitions
    /// run through the retired window overlay — a frozen capture for the
    /// removal, a live capture for the insertion — publishing a
    /// `TransitionFrame` every tick.
    #[cfg(feature = "gpu_surface")]
    fn pixel_trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "transition::a_pixel_removal_freezes_into_the_window_overlay",
                || {
                    a_pixel_removal_freezes_into_the_window_overlay();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::a_pixel_insertion_captures_live_content_in_the_overlay",
                || {
                    a_pixel_insertion_captures_live_content_in_the_overlay();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::pixel_frame_follows_the_slot_while_scrolling",
                || {
                    pixel_frame_follows_the_slot_while_scrolling();
                    Ok(())
                },
            ),
        ]
    }

    /// The Stage 7 frozen-frame trials (§8.2): retirement installs a
    /// `FrozenCapturable` into each armed surface's capturable slot.
    fn frozen_trials() -> Vec<libtest_mimic::Trial> {
        let mut trials = Vec::new();
        #[cfg(feature = "gpu_surface")]
        trials.extend(vec![
            libtest_mimic::Trial::test(
                "transition::gpu_surface_ghost_keeps_its_last_frame",
                || {
                    gpu_surface_ghost_keeps_its_last_frame();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test("transition::filtered_ghost_keeps_its_last_frame", || {
                filtered_ghost_keeps_its_last_frame();
                Ok(())
            }),
            libtest_mimic::Trial::test(
                "transition::ghost_inside_a_live_filter_still_resolves",
                || {
                    ghost_inside_a_live_filter_still_resolves();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "transition::unarmed_surfaces_stay_framebuffer_only",
                || {
                    unarmed_surfaces_stay_framebuffer_only();
                    Ok(())
                },
            ),
        ]);
        trials
    }

    /// Every capturable surface inside `root` — (view pointer, resolved
    /// surface) pairs in walk order.
    #[cfg(feature = "gpu_surface")]
    fn capturables_in(
        root: &PlatformView,
    ) -> Vec<(usize, Rc<dyn cocoa_ui::capture::CapturableSurface>)> {
        let mut entries = Vec::new();
        cocoa_ui::capture::for_each_capturable(root, &mut |view, surface| {
            entries.push((std::ptr::from_ref(view).addr(), Rc::clone(surface)));
        });
        entries
    }

    /// Capturable surfaces resolved at every depth of `root`'s subtree —
    /// the enclosing filter's view: `for_each_capturable` prunes at a
    /// resolved surface since its own producer composites it, but that
    /// producer's *content* — the subtree under it — is walked by the
    /// enclosing capture, so a nested surface still answers its slot.
    #[cfg(feature = "gpu_surface")]
    fn capturables_deep(
        root: &PlatformView,
    ) -> Vec<(usize, Rc<dyn cocoa_ui::capture::CapturableSurface>)> {
        fn walk(
            view: &PlatformView,
            entries: &mut Vec<(usize, Rc<dyn cocoa_ui::capture::CapturableSurface>)>,
        ) {
            if let Some(surface) = cocoa_ui::capture::resolve_capturable(view) {
                entries.push((std::ptr::from_ref(view).addr(), Rc::clone(&surface)));
            }
            for subview in cocoa_ui::view::subviews(view) {
                walk(&subview, entries);
            }
        }
        let mut entries = Vec::new();
        walk(root, &mut entries);
        entries
    }

    /// The retained view carrying `root`'s first resolved capturable —
    /// the surface's own view, never the leaf it renders under.
    #[cfg(feature = "gpu_surface")]
    fn capturable_view(root: &PlatformView) -> Option<Retained<PlatformView>> {
        let mut found = None;
        cocoa_ui::capture::for_each_capturable(root, &mut |view, _| {
            found.get_or_insert_with(|| view::retain_base(view));
        });
        found
    }

    /// Mounts `view` under a GPU-equipped environment — the runtime,
    /// scene engine, filter pipelines and `PresentationTime` the
    /// `gpu_runtime::prepare` production path installs.
    #[cfg(feature = "gpu_surface")]
    pub fn gpu_leaf(view: impl waterui_backend_core::View) -> waterui_apple::contract::NativeLeaf {
        // The filtered leaf's `AnyEffect::build` binds through the local
        // executor — `initialize_process` only runs on macOS, so install
        // the plain main-queue answer here the way the webview trials do.
        let _ = executor_core::try_init_local_executor(
            native_executor::NativeMainExecutor::new()
                .expect("the native suite runs on the process's main thread"),
        );
        let mut env = super::resolve::env();
        let runtime = waterui_apple::native_test_support::block_on_main(
            30.0,
            waterui_graphics::gpu::GpuRuntime::new(),
        )
        .expect("the harness provides a GPU runtime");
        waterui_apple::native_test_support::install_gpu(&mut env, runtime);
        waterui_apple::dispatch::render(waterui_backend_core::AnyView::new(view), &env)
    }

    /// A `MeshGradient` mounts the `SceneView` GPU surface leaf the
    /// frozen-frame trials arm.
    #[cfg(feature = "gpu_surface")]
    pub fn gpu_picture() -> impl waterui_backend_core::View {
        use waterui::graphics::color::WorkingColor;
        use waterui::graphics::gradient::MeshGradient;
        use waterui::reactive::constant;
        // Four distinct corners, so a review PNG tells the frozen frame
        // from a blank or cleared one.
        MeshGradient::new(
            2,
            2,
            constant(vec![
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
                WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
                WorkingColor::new([1.0, 1.0, 0.0, 1.0]),
            ]),
        )
    }

    /// A blurred label mounts the `FilteredView` GPU filter chain.
    #[cfg(feature = "gpu_surface")]
    fn filtered_label() -> impl waterui_backend_core::View {
        use waterui::view::FilterViewExt;
        label("filtered").blur(2.0)
    }

    /// The exit spec the frozen trials use — long enough to keep the
    /// ghost live across a capture and several pumps.
    #[cfg(feature = "gpu_surface")]
    fn frozen_spec() -> TransitionSpec {
        TransitionSpec::new(PropertyTransition::opacity())
            .with_animation(Animation::linear(Duration::from_millis(400)))
    }

    /// Writes the window's captured pixels to `<dir>/<name>.png`
    /// through the harness's own `capture_rgba` — visual review only;
    /// nothing asserts on pixel values.
    #[cfg(feature = "gpu_surface")]
    fn export_window_png(view: &PlatformView, dir: &str, name: &str) {
        let captured = waterui_apple::native_test_support::capture_view_rgba(view);
        let image = cocoa_ui::bitmap::image_from_rgba(
            &captured.pixels,
            captured.width as usize,
            captured.height as usize,
        )
        .expect("the captured pixels make a CGImage");
        #[cfg(target_os = "ios")]
        let data = cocoa_ui::objc2_ui_kit::UIImage::imageWithCGImage(&image)
            .png_representation()
            .expect("PNG encoding succeeds")
            .to_vec();
        #[cfg(target_os = "macos")]
        let data = {
            use cocoa_ui::objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
            use cocoa_ui::objc2_foundation::NSDictionary;
            let rep = NSBitmapImageRep::initWithCGImage(
                <NSBitmapImageRep as cocoa_ui::objc2::AllocAnyThread>::alloc(),
                &image,
            );
            // SAFETY: PNG encoding of a live bitmap rep.
            unsafe {
                rep.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::<cocoa_ui::objc2_foundation::NSString>::new(),
                )
            }
            .expect("PNG encoding succeeds")
            .to_vec()
        };
        std::fs::create_dir_all(dir).expect("the review directory creates");
        std::fs::write(format!("{dir}/{name}.png"), data).expect("the review PNG writes");
    }

    /// `gpu_surface_ghost_keeps_its_last_frame` — a GPU surface under a
    /// `.transition(opacity)` mounts its last presented frame into a
    /// `FrozenCapturable` at removal (§8.2): the live capturable's Rc
    /// dies with the leaf, the slot resolves to a frozen stand-in that
    /// answers `has_presented_frame` and never `can_present`, and the
    /// ghost's completion releases it.
    #[cfg(feature = "gpu_surface")]
    pub fn gpu_surface_ghost_keeps_its_last_frame() {
        let _mtm = mtm();
        let (handler, dynamic) = Dynamic::new();
        let leaf = gpu_leaf(dynamic);
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| {
            handler.set(Metadata::new(gpu_picture(), frozen_spec()));
        });
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| {
                capturables_in(leaf.view())
                    .iter()
                    .any(|(_, surface)| surface.has_presented_frame())
            })),
            "the GPU surface presents before removal"
        );
        let live = autoreleasepool(|_| {
            let entries = capturables_in(leaf.view());
            assert_eq!(entries.len(), 1, "one capturable surface in the subtree");
            Rc::downgrade(&entries[0].1)
        });
        autoreleasepool(|_| handler.set(label("gone")));
        assert!(
            live.upgrade().is_none(),
            "the removal dropped the live capturable with the leaf"
        );
        autoreleasepool(|_| {
            let entries = capturables_in(leaf.view());
            assert_eq!(entries.len(), 1, "the ghost keeps one resolved surface");
            assert!(
                entries[0].1.has_presented_frame(),
                "the frozen surface reports the frame it last presented"
            );
            assert!(
                !entries[0].1.can_present(),
                "a frozen surface never presents itself"
            );
        });
        export_window_png(
            leaf.view(),
            "/tmp/1472-stage7",
            "gpu_surface_ghost_keeps_its_last_frame",
        );
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| capturables_in(leaf.view()).is_empty())
            }),
            "the ghost's completion releases the frozen surface"
        );
    }

    /// `filtered_ghost_keeps_its_last_frame` — the same hand-off for a
    /// `FilteredView` GPU surface: retirement installs the frozen
    /// stand-in and the ghost answers `has_presented_frame` until it
    /// completes.
    #[cfg(feature = "gpu_surface")]
    pub fn filtered_ghost_keeps_its_last_frame() {
        let _mtm = mtm();
        let (handler, dynamic) = Dynamic::new();
        let leaf = gpu_leaf(dynamic);
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| {
            handler.set(Metadata::new(filtered_label(), frozen_spec()));
        });
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| {
                capturables_in(leaf.view())
                    .iter()
                    .any(|(_, surface)| surface.has_presented_frame())
            })),
            "the filtered surface presents before removal"
        );
        let live = autoreleasepool(|_| {
            let entries = capturables_in(leaf.view());
            assert_eq!(
                entries.len(),
                1,
                "the filtered chain is one capturable surface"
            );
            Rc::downgrade(&entries[0].1)
        });
        autoreleasepool(|_| handler.set(label("gone")));
        assert!(
            live.upgrade().is_none(),
            "the removal dropped the live capturable with the leaf"
        );
        autoreleasepool(|_| {
            let entries = capturables_in(leaf.view());
            assert_eq!(entries.len(), 1, "the ghost keeps one resolved surface");
            assert!(
                entries[0].1.has_presented_frame(),
                "the frozen surface reports the frame it last presented"
            );
            assert!(
                !entries[0].1.can_present(),
                "a frozen surface never presents itself"
            );
        });
        export_window_png(
            leaf.view(),
            "/tmp/1472-stage7",
            "filtered_ghost_keeps_its_last_frame",
        );
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| capturables_in(leaf.view()).is_empty())
            }),
            "the ghost's completion releases the frozen surface"
        );
    }

    /// `ghost_inside_a_live_filter_still_resolves` — a GPU surface ghost
    /// inside a still-live `FilteredView` chain: the frozen stand-in
    /// resolves through `for_each_capturable` while the enclosing filter
    /// keeps presenting — the enclosing capture's resolve keeps finding
    /// the ghost's surfaces instead of an unresolved Metal layer.
    #[cfg(feature = "gpu_surface")]
    pub fn ghost_inside_a_live_filter_still_resolves() {
        use waterui::view::FilterViewExt;
        let _mtm = mtm();
        let (handler, dynamic) = Dynamic::new();
        let (outer, wrapper) = Dynamic::new();
        let leaf = gpu_leaf(wrapper);
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| {
            outer.set(dynamic.blur(2.0));
            handler.set(Metadata::new(gpu_picture(), frozen_spec()));
        });
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| {
                capturables_in(leaf.view())
                    .iter()
                    .any(|(_, surface)| surface.has_presented_frame())
            })),
            "the surface inside the filter presents before removal"
        );
        autoreleasepool(|_| handler.set(label("gone")));
        autoreleasepool(|_| {
            let entries = capturables_deep(leaf.view());
            assert_eq!(
                entries.len(),
                2,
                "the live filter and the ghost's frozen surface both resolve"
            );
            let mut live: Vec<_> = entries
                .iter()
                .filter(|(_, surface)| surface.can_present())
                .collect();
            let mut frozen: Vec<_> = entries
                .iter()
                .filter(|(_, surface)| !surface.can_present())
                .collect();
            assert_eq!(
                live.len(),
                1,
                "the enclosing filter keeps its live producer"
            );
            assert_eq!(frozen.len(), 1, "the ghost resolves a frozen surface");
            assert!(
                live[0].1.has_presented_frame(),
                "the filter keeps presenting"
            );
            assert!(
                frozen[0].1.has_presented_frame(),
                "the ghost surface keeps its last frame"
            );
            live.clear();
            frozen.clear();
        });
        export_window_png(
            leaf.view(),
            "/tmp/1472-stage7",
            "ghost_inside_a_live_filter_still_resolves",
        );
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| {
                capturables_deep(leaf.view())
                    .iter()
                    .all(|(_, surface)| surface.can_present())
            })),
            "the ghost's completion leaves only the live filter resolving"
        );
    }

    /// `unarmed_surfaces_stay_framebuffer_only` — the arming switch is
    /// exactly `SnapshotDemand`: outside a declaration the layer stays
    /// framebuffer-only; inside one it becomes readable.
    #[cfg(feature = "gpu_surface")]
    pub fn unarmed_surfaces_stay_framebuffer_only() {
        #[cfg(target_os = "macos")]
        use cocoa_ui::appkit::surface_view::SurfaceView;
        #[cfg(target_os = "ios")]
        use cocoa_ui::uikit::surface_view::SurfaceView;

        let _mtm = mtm();
        let leaf = gpu_leaf(gpu_picture());
        let _window = super::mount_and_order_front(leaf.view());
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| {
                capturables_in(leaf.view())
                    .iter()
                    .any(|(_, surface)| surface.has_presented_frame())
            })),
            "the unarmed surface presents"
        );
        autoreleasepool(|_| {
            let view = capturable_view(leaf.view()).expect("the surface resolves");
            let surface = view
                .downcast_ref::<SurfaceView>()
                .expect("a GPU surface mounts the kit surface view");
            assert!(
                surface.presentation_layer().framebufferOnly(),
                "an unarmed surface keeps framebuffer-only drawables"
            );
        });

        let (handler, dynamic) = Dynamic::new();
        handler.set(Metadata::new(gpu_picture(), frozen_spec()));
        let leaf = gpu_leaf(dynamic);
        let _window = super::mount_and_order_front(leaf.view());
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| {
                capturables_in(leaf.view())
                    .iter()
                    .any(|(_, surface)| surface.has_presented_frame())
            })),
            "the armed surface presents"
        );
        autoreleasepool(|_| {
            let view = capturable_view(leaf.view()).expect("the surface resolves");
            let surface = view
                .downcast_ref::<SurfaceView>()
                .expect("a GPU surface mounts the kit surface view");
            assert!(
                !surface.presentation_layer().framebufferOnly(),
                "an armed surface's drawables are readable"
            );
        });
    }

    /// A blur spec — long enough to watch the effect live mid-flight.
    #[cfg(feature = "gpu_surface")]
    /// Sharp edges over the GPU gradient: a review PNG shows whether
    /// the frame is blurred, which a smooth gradient alone cannot.
    #[cfg(feature = "gpu_surface")]
    fn blur_picture() -> impl waterui_backend_core::View {
        use waterui::prelude::{text, vstack};
        vstack((text("BLUR").size(96.0), gpu_picture()))
    }

    fn blur_spec() -> TransitionSpec {
        TransitionSpec::new(PropertyTransition::blur(8.0))
            .with_animation(Animation::linear(Duration::from_millis(400)))
    }

    /// `blur_insertion_renders_through_the_live_effect` — a
    /// `.transition(blur)` insertion mounts the content's own view
    /// inside the transition host and adds the `LiveEffect`'s output as
    /// a sibling (§8.2): mid-flight the host carries both, the content
    /// view is never reparented, and completion removes the output —
    /// the same content pointer remains.
    #[cfg(feature = "gpu_surface")]
    pub fn blur_insertion_renders_through_the_live_effect() {
        let _mtm = mtm();
        let (handler, dynamic) = Dynamic::new();
        let leaf = gpu_leaf(dynamic);
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| handler.set(label("before")));
        assert!(
            pump_main_until(3.0, || {
                autoreleasepool(|_| !view::subviews(leaf.view()).is_empty())
            }),
            "the first child mounts"
        );
        autoreleasepool(|_| {
            handler.set(Metadata::new(blur_picture(), blur_spec()));
        });
        let host_ptr: Cell<*const PlatformView> = Cell::new(std::ptr::null());
        let content_ptr: Cell<*const PlatformView> = Cell::new(std::ptr::null());
        // Mid-flight the host carries the content view plus the effect's
        // output view.
        assert!(
            pump_main_until(3.0, || {
                autoreleasepool(|_| {
                    let Some(host) = view::subviews(leaf.view()).into_iter().next() else {
                        return false;
                    };
                    let subviews = view::subviews(&host);
                    if subviews.len() == 2 {
                        host_ptr.set(std::ptr::from_ref(&*host));
                        content_ptr.set(std::ptr::from_ref(&*subviews[0]));
                    }
                    subviews.len() == 2
                })
            }),
            "the live effect's output joins the transition host"
        );
        export_window_png(
            leaf.view(),
            "/tmp/1472-stage8",
            "blur_insertion_renders_through_the_live_effect",
        );
        // Completion: the output leaves and the content view is the same
        // pointer — it was never reparented.
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| {
                    let Some(host) = view::subviews(leaf.view()).into_iter().next() else {
                        return false;
                    };
                    let subviews = view::subviews(&host);
                    subviews.len() == 1 && std::ptr::from_ref(&*subviews[0]) == content_ptr.get()
                })
            }),
            "the effect's output leaves at completion and the content view stays"
        );
    }

    /// `blur_removal_renders_from_the_frozen_capture` — a blur removal
    /// ghosts the retired subtree: the ghost host gains the
    /// `FrozenEffect`'s output view, and the retired root is hidden only
    /// after the effect's first present (§8.2) — never at attach, so no
    /// frame shows a gap.
    #[cfg(feature = "gpu_surface")]
    pub fn blur_removal_renders_from_the_frozen_capture() {
        let _mtm = mtm();
        let (handler, dynamic) = Dynamic::new();
        let leaf = gpu_leaf(dynamic);
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| {
            handler.set(Metadata::new(blur_picture(), blur_spec()));
        });
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| {
                capturables_in(leaf.view())
                    .iter()
                    .any(|(_, surface)| surface.has_presented_frame())
            })),
            "the GPU surface presents before removal"
        );
        let host_ptr: Cell<*const PlatformView> = Cell::new(std::ptr::null());
        let root_ptr: Cell<*const PlatformView> = Cell::new(std::ptr::null());
        autoreleasepool(|_| {
            let host = view::subviews(leaf.view())
                .into_iter()
                .next()
                .expect("the transition host mounts");
            host_ptr.set(std::ptr::from_ref(&*host));
            root_ptr.set(std::ptr::from_ref(
                &*view::subviews(&host).into_iter().next().expect("content"),
            ));
        });
        autoreleasepool(|_| handler.set(label("gone")));
        // The root is still visible right at removal — it hides only
        // when the effect's first frame lands.
        let root_view = autoreleasepool(|_| {
            let host = view::subviews(leaf.view())
                .into_iter()
                .find(|v| std::ptr::from_ref(&**v) == host_ptr.get())
                .expect("the ghost host remains under the dynamic");
            let subviews = view::subviews(&host);
            assert_eq!(subviews.len(), 1, "the ghost starts with just the root");
            let root = view::retain_base(&*subviews[0]);
            assert_eq!(
                std::ptr::from_ref(&*root),
                root_ptr.get(),
                "the retired root is the outgoing content view"
            );
            assert!(
                !view::is_hidden(&root),
                "the retired root is visible until the effect's first present"
            );
            root
        });
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| view::is_hidden(&root_view))),
            "the first blurred frame hides the retired root"
        );
        autoreleasepool(|_| {
            let host = view::superview(&root_view).expect("the root stays in the ghost host");
            assert_eq!(
                view::subviews(&host).len(),
                2,
                "the frozen effect's output joined the ghost host"
            );
        });
        // Mid-ramp: the removal blur grows from 0, so the frame the
        // effect first presents is barely blurred.
        pump_main_until(0.2, || false);
        export_window_png(
            leaf.view(),
            "/tmp/1472-stage8",
            "blur_removal_renders_from_the_frozen_capture",
        );
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| view::subviews(leaf.view()).len() == 1)
            }),
            "the ghost's completion detaches it"
        );
    }

    /// `blur_returning_to_zero_keeps_one_effect` — a transition whose
    /// sampled blur rises and returns to zero inside one instance
    /// creates exactly one effect view, lazily on the first nonzero
    /// sample, and keeps it until the instance ends (§8.2).
    #[cfg(feature = "gpu_surface")]
    pub fn blur_returning_to_zero_keeps_one_effect() {
        use waterui_core::transition::{
            Transition, TransitionGeometry, TransitionPhase, TransitionProperties,
        };
        /// Blur `0 → 8 → 0` across the middle of the progress ramp —
        /// nonzero only for `progress ∈ (1/6, 5/6)`, so the transition
        /// still runs a while after the radius reaches 0 again.
        struct PulseBlur;
        impl Transition for PulseBlur {
            fn properties(
                &self,
                _phase: TransitionPhase,
                progress: f32,
                _geometry: TransitionGeometry,
            ) -> TransitionProperties {
                let mut properties = TransitionProperties::IDENTITY;
                properties.blur = 8.0 * (1.0 - 3.0f32.mul_add(progress, -1.5).abs()).max(0.0);
                properties
            }
        }
        let _mtm = mtm();
        let (handler, dynamic) = Dynamic::new();
        let leaf = gpu_leaf(dynamic);
        let _window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| handler.set(label("before")));
        assert!(
            pump_main_until(3.0, || {
                autoreleasepool(|_| !view::subviews(leaf.view()).is_empty())
            }),
            "the first child mounts"
        );
        autoreleasepool(|_| {
            handler.set(Metadata::new(
                label("pulse"),
                TransitionSpec::new(PulseBlur)
                    .with_animation(Animation::linear(Duration::from_millis(400))),
            ));
        });
        let first_effect: Cell<*const PlatformView> = Cell::new(std::ptr::null());
        let saw_effect = Cell::new(false);
        let recreated = Cell::new(false);
        // Watch every pump until the insertion completes: the effect
        // view appears once, never changes pointer — including across
        // the ramp's return to zero — and leaves at the end. Completion
        // reads "the effect was seen and is gone" so the pump does not
        // close on the pre-effect window.
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| {
                    let Some(host) = view::subviews(leaf.view()).into_iter().next() else {
                        return false;
                    };
                    let subviews = view::subviews(&host);
                    assert!(
                        subviews.len() <= 2,
                        "the host never carries more than content plus one effect"
                    );
                    if subviews.len() == 2 {
                        saw_effect.set(true);
                        let ptr = std::ptr::from_ref(&*subviews[1]);
                        if first_effect.get().is_null() {
                            first_effect.set(ptr);
                        } else {
                            recreated.set(recreated.get() || ptr != first_effect.get());
                        }
                    }
                    saw_effect.get() && subviews.len() == 1
                })
            }),
            "the insertion transition completes"
        );
        assert!(
            !first_effect.get().is_null(),
            "the effect was created during the nonzero-blur ramp"
        );
        assert!(
            !recreated.get(),
            "one effect view served the whole ramp — none was recreated at zero"
        );
        export_window_png(
            leaf.view(),
            "/tmp/1472-stage8",
            "blur_returning_to_zero_keeps_one_effect",
        );
    }
    /// A `captures_content` transition whose body is a tinted shader over
    /// the content — the PNG review distinguishes the effect output from
    /// the live pixels — and that stashes the published
    /// `Computed<TransitionFrame>` for the assertions (§8.2).
    #[cfg(feature = "gpu_surface")]
    struct PixelProbe {
        /// The frame signal the body's `TransitionContext` published.
        frame: Rc<
            RefCell<Option<waterui::reactive::Computed<waterui_core::transition::TransitionFrame>>>,
        >,
    }

    #[cfg(feature = "gpu_surface")]
    impl Transition for PixelProbe {
        fn captures_content(&self, _phase: waterui_core::transition::TransitionPhase) -> bool {
            true
        }

        fn body(
            &self,
            content: AnyView,
            transition_context: waterui_core::transition::TransitionContext,
        ) -> impl waterui_backend_core::View {
            *self.frame.borrow_mut() = Some(transition_context.frame);
            let shader =
                waterui::graphics::filtrate::ShaderEffect::new(include_str!("pixel_probe.wgsl"))
                    .expect("the probe shader is valid WGSL");
            waterui::graphics::WithEffect::new(content, shader)
        }
    }

    /// A pixel probe's frame cell plus its spec — a 400 ms linear ramp
    /// keeps the transition observable across several pumps.
    #[cfg(feature = "gpu_surface")]
    fn pixel_probe() -> (
        Rc<RefCell<Option<waterui::reactive::Computed<waterui_core::transition::TransitionFrame>>>>,
        TransitionSpec,
    ) {
        let frame = Rc::new(RefCell::new(None));
        let spec = TransitionSpec::new(PixelProbe {
            frame: Rc::clone(&frame),
        })
        .with_animation(Animation::linear(Duration::from_millis(400)));
        (frame, spec)
    }

    /// The retired overlay host the pixel transition added directly to
    /// the window — `None` while the effect has not mounted. It is the
    /// topmost retired host: on `AppKit` the content view is the site
    /// itself, whose retired ghost sits beneath the live child.
    #[cfg(feature = "gpu_surface")]
    fn retired_overlay(window: &super::AttachedWindow) -> Option<Retained<PlatformView>> {
        #[cfg(target_os = "macos")]
        let parent = window
            .native()
            .contentView()
            .expect("the window has a content view");
        #[cfg(target_os = "ios")]
        let parent = {
            let view: &PlatformView = window;
            view::retain_base(view)
        };
        view::subviews(&parent).into_iter().rev().find(|sub| {
            sub.downcast_ref::<HostView>()
                .is_some_and(HostView::is_retired)
        })
    }

    /// The `NSScrollView`/`UIScrollView` a subtree scrolls in — `view`
    /// itself first, then depth-first: on `AppKit` the scroll view is the
    /// window's content view.
    #[cfg(feature = "gpu_surface")]
    fn scroll_view_in(view: &PlatformView) -> Option<Retained<PlatformView>> {
        #[cfg(target_os = "macos")]
        let is_scroll = view
            .downcast_ref::<cocoa_ui::objc2_app_kit::NSScrollView>()
            .is_some();
        #[cfg(target_os = "ios")]
        let is_scroll = view
            .downcast_ref::<cocoa_ui::objc2_ui_kit::UIScrollView>()
            .is_some();
        if is_scroll {
            return Some(view::retain_base(view));
        }
        view::subviews(view)
            .iter()
            .find_map(|sub| scroll_view_in(sub))
    }

    /// The view the review PNGs capture mid-flight: the retired window
    /// overlay the pixel effect presents in.
    #[cfg(feature = "gpu_surface")]
    fn review_root(window: &super::AttachedWindow) -> Retained<PlatformView> {
        retired_overlay(window).expect("a pixel transition in flight has its overlay")
    }

    /// Pumps until a GPU surface under `site` has presented — the frame a
    /// later removal freezes.
    #[cfg(feature = "gpu_surface")]
    fn await_gpu_presented(site: &PlatformView) {
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| {
                capturables_in(site)
                    .iter()
                    .any(|(_, surface)| surface.has_presented_frame())
            })),
            "the claimed child's GPU surface presents before removal"
        );
    }

    /// The retired ghost host under `site`, once the pixel effect's first
    /// present has hidden its root.
    #[cfg(feature = "gpu_surface")]
    fn await_pixel_present(site: &PlatformView) -> Retained<PlatformView> {
        let ghost = autoreleasepool(|_| {
            view::subviews(site)
                .into_iter()
                .find(|sub| {
                    sub.downcast_ref::<HostView>()
                        .is_some_and(HostView::is_retired)
                })
                .expect("the retired ghost host stays under the site")
        });
        let root = autoreleasepool(|_| {
            view::subviews(&ghost)
                .into_iter()
                .next()
                .expect("the ghost host keeps its root")
        });
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| view::is_hidden(&root))),
            "the pixel effect presents in the overlay and hides the retired root"
        );
        ghost
    }

    /// The published frame's `source` — reads the stashed `Computed`.
    #[cfg(feature = "gpu_surface")]
    fn published_source(
        frame: &RefCell<
            Option<waterui::reactive::Computed<waterui_core::transition::TransitionFrame>>,
        >,
    ) -> waterui_core::layout::Rect {
        use waterui::reactive::Signal as _;
        frame
            .borrow()
            .as_ref()
            .expect("the transition body published a frame")
            .snapshot()
            .source
    }

    /// `a_pixel_removal_freezes_into_the_window_overlay` — a
    /// `captures_content` removal: the window gains a retired overlay
    /// host, the published `TransitionFrame.source` tracks the ghost
    /// host's window rect — including a mid-flight move — and completion
    /// releases the overlay, the effect and the retired root (§8.2).
    #[cfg(feature = "gpu_surface")]
    pub fn a_pixel_removal_freezes_into_the_window_overlay() {
        let _mtm = mtm();
        let (probe_frame, spec) = pixel_probe();
        let (handler, dynamic) = Dynamic::new();
        let leaf = gpu_leaf(dynamic);
        let window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| {
            handler.set(Metadata::new(blur_picture(), spec));
        });
        await_gpu_presented(leaf.view());
        autoreleasepool(|_| handler.set(label("gone")));
        // The frozen capture lands and the effect mounts in the window
        // overlay — the ghost stays beneath the live child.
        assert!(
            pump_main_until(5.0, || autoreleasepool(
                |_| retired_overlay(&window).is_some()
            )),
            "the retired window overlay mounts the effect"
        );
        // The published source is the ghost host's window rect, top-left.
        let ghost = await_pixel_present(leaf.view());
        let expected = autoreleasepool(|_| view::bounds_in_window(&ghost));
        let published = published_source(&probe_frame);
        assert!(
            (f64::from(published.x()) - expected.origin.x).abs() < 0.5
                && (f64::from(published.y()) - {
                    #[cfg(target_os = "macos")]
                    {
                        let overlay = retired_overlay(&window).expect("overlay");
                        view::bounds(&overlay).size.height
                            - expected.origin.y
                            - expected.size.height
                    }
                    #[cfg(target_os = "ios")]
                    {
                        expected.origin.y
                    }
                })
                .abs()
                    < 0.5,
            "the published source {published:?} matches the host's window rect {expected:?}"
        );
        // The site moving mid-flight republishes `source`: shift the
        // dynamic's host and the next tick reports the new rect.
        let moved = autoreleasepool(|_| {
            let mut frame = view::frame(leaf.view());
            frame.origin.x += 24.0;
            frame.origin.y += 12.0;
            view::set_frame(leaf.view(), frame);
            frame
        });
        let _ = moved;
        assert!(
            pump_main_until(3.0, || {
                let published = published_source(&probe_frame);
                autoreleasepool(|_| {
                    let expected = view::bounds_in_window(&ghost);
                    (f64::from(published.x()) - expected.origin.x).abs() < 0.5
                })
            }),
            "the published frame follows the moved site"
        );
        export_window_png(
            &review_root(&window),
            "/tmp/1472-stage9",
            "a_pixel_removal_freezes_into_the_window_overlay",
        );
        // Completion releases the overlay and the ghost host.
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| {
                    retired_overlay(&window).is_none() && view::subviews(leaf.view()).len() == 1
                })
            }),
            "the overlay and ghost release at completion"
        );
    }

    /// `a_pixel_insertion_captures_live_content_in_the_overlay` — a
    /// `captures_content` insertion: the content mounts live inside the
    /// transition host and is never reparented while the live-capture
    /// effect presents through the retired window overlay (§8.2).
    #[cfg(feature = "gpu_surface")]
    pub fn a_pixel_insertion_captures_live_content_in_the_overlay() {
        let _mtm = mtm();
        let (_frame, spec) = pixel_probe();
        let (handler, dynamic) = Dynamic::new();
        let leaf = gpu_leaf(dynamic);
        let window = super::mount_and_order_front(leaf.view());
        autoreleasepool(|_| handler.set(label("before")));
        assert!(
            pump_main_until(3.0, || {
                autoreleasepool(|_| !view::subviews(leaf.view()).is_empty())
            }),
            "the first child mounts"
        );
        let died = Rc::new(Cell::new(false));
        autoreleasepool(|_| {
            handler.set(Metadata::new(
                Metadata::new(blur_picture(), Retain::new(Died(Rc::clone(&died)))),
                spec,
            ));
        });
        // The claimed child mounts inside the transition host — its
        // content view keeps that parent while the effect lives in the
        // window overlay.
        assert!(
            pump_main_until(5.0, || autoreleasepool(
                |_| retired_overlay(&window).is_some()
            )),
            "the retired window overlay mounts the live effect"
        );
        let content = autoreleasepool(|_| {
            let host = view::subviews(leaf.view())
                .into_iter()
                .next()
                .expect("the transition host mounts");
            let content = view::subviews(&host)
                .into_iter()
                .next()
                .expect("the content view stays mounted");
            let parent = view::superview(&content).expect("the content has a parent");
            assert!(
                std::ptr::from_ref(&*parent) == std::ptr::from_ref(&*host),
                "the content view was never reparented into the overlay"
            );
            content
        });
        assert!(
            pump_main_until(5.0, || autoreleasepool(|_| view::is_hidden(&content))),
            "the presented effect hides the captured content"
        );
        autoreleasepool(|_| {
            let host = view::superview(&content).expect("the content has a parent");
            let grandparent = view::superview(&host).expect("the host has a parent");
            assert!(
                std::ptr::from_ref(&*grandparent) == std::ptr::from_ref(leaf.view()),
                "the content view was never reparented into the overlay"
            );
        });
        export_window_png(
            &review_root(&window),
            "/tmp/1472-stage9",
            "a_pixel_insertion_captures_live_content_in_the_overlay",
        );
        // Completion: the overlay leaves and the content shows again —
        // its `Retain` never died, since semantics stayed live.
        assert!(
            pump_main_until(5.0, || {
                autoreleasepool(|_| {
                    retired_overlay(&window).is_none() && {
                        let host = view::subviews(leaf.view()).into_iter().next();
                        host.is_some_and(|host| {
                            view::subviews(&host)
                                .into_iter()
                                .next()
                                .is_some_and(|content| !view::is_hidden(&content))
                        })
                    }
                })
            }),
            "the overlay releases and the content reveals at completion"
        );
        assert!(
            !died.get(),
            "the content's semantics stayed live through the insertion"
        );
    }

    /// The scroll distance the frame-follow trial applies.
    #[cfg(feature = "gpu_surface")]
    const SCROLL_DELTA: f64 = 48.0;

    /// `pixel_frame_follows_the_slot_while_scrolling` — a pixel removal
    /// inside a scroll view: the claimed child mounts inside a real
    /// `UIScrollView`/`NSScrollView` document, and scrolling mid-flight
    /// moves the published `source.origin` by the scroll delta (§8.2).
    #[cfg(feature = "gpu_surface")]
    pub fn pixel_frame_follows_the_slot_while_scrolling() {
        let _mtm = mtm();
        let (probe_frame, spec) = pixel_probe();
        let (handler, dynamic) = Dynamic::new();
        let leaf = gpu_leaf(dynamic);
        // The site lives inside a scroll view sized beyond the window:
        // the claimed child gets a nonzero slot at once and the ghost's
        // window rect moves with the content offset.
        #[cfg(target_os = "macos")]
        let window = {
            use cocoa_ui::objc2::{MainThreadOnly, msg_send};
            use cocoa_ui::objc2_app_kit::NSScrollView;
            // SAFETY: plain `NSScrollView` init on the main thread.
            let scroll: Retained<NSScrollView> =
                unsafe { msg_send![NSScrollView::alloc(mtm()), init] };
            let window = super::mount_and_order_front(&scroll);
            // A flipped document, as every WaterUI scroll content is: a
            // larger clip origin scrolls the content up.
            let document = HostView::new(mtm(), cocoa_ui::Rect::new(0.0, 0.0, 390.0, 1200.0));
            scroll.setDocumentView(Some(&document));
            view::add_subview(&document, leaf.view());
            view::set_frame(leaf.view(), cocoa_ui::Rect::new(0.0, 0.0, 390.0, 1200.0));
            window
        };
        #[cfg(target_os = "ios")]
        let window = {
            use cocoa_ui::objc2::{MainThreadOnly, msg_send};
            use cocoa_ui::objc2_ui_kit::UIScrollView;
            // SAFETY: plain `UIScrollView` init on the main thread.
            let scroll: Retained<UIScrollView> =
                unsafe { msg_send![UIScrollView::alloc(mtm()), init] };
            let window = super::mount_and_order_front(&scroll);
            let scroll_view: &PlatformView = &scroll;
            view::add_subview(scroll_view, leaf.view());
            view::set_frame(leaf.view(), cocoa_ui::Rect::new(0.0, 0.0, 390.0, 1200.0));
            scroll.setContentSize(cocoa_ui::objc2_core_foundation::CGSize::new(390.0, 1200.0));
            window
        };
        autoreleasepool(|_| {
            handler.set(Metadata::new(blur_picture(), spec));
        });
        await_gpu_presented(leaf.view());
        autoreleasepool(|_| handler.set(label("gone")));
        assert!(
            pump_main_until(5.0, || autoreleasepool(
                |_| retired_overlay(&window).is_some()
            )),
            "the retired window overlay mounts the effect"
        );
        await_pixel_present(leaf.view());
        let before = published_source(&probe_frame);
        // Scroll: the ghost's slot moves with the document, and the next
        // tick republishes it.
        autoreleasepool(|_| {
            #[cfg(target_os = "macos")]
            {
                use cocoa_ui::objc2_app_kit::NSScrollView;
                let content = window.native().contentView().expect("content");
                let scroll_view = scroll_view_in(&content).expect("the NSScrollView");
                let scroll = scroll_view.downcast_ref::<NSScrollView>().expect("scroll");
                let clip = scroll.contentView();
                let mut origin = clip.bounds().origin;
                origin.y += SCROLL_DELTA;
                clip.scrollToPoint(origin);
                scroll.reflectScrolledClipView(&clip);
            }
            #[cfg(target_os = "ios")]
            {
                use cocoa_ui::objc2_ui_kit::UIScrollView;
                let window_view: &PlatformView = &window;
                let scroll_view = scroll_view_in(window_view).expect("the UIScrollView");
                let scroll = scroll_view.downcast_ref::<UIScrollView>().expect("scroll");
                let mut offset = scroll.contentOffset();
                offset.y += SCROLL_DELTA;
                scroll.setContentOffset_animated(offset, false);
            }
        });
        assert!(
            pump_main_until(3.0, || {
                let after = published_source(&probe_frame);
                (f64::from(before.y() - after.y()) - SCROLL_DELTA).abs() < 1.0
            }),
            "the published source.origin moves by the scroll delta"
        );
        export_window_png(
            &review_root(&window),
            "/tmp/1472-stage9",
            "pixel_frame_follows_the_slot_while_scrolling",
        );
    }
}

/// `scale`/`rotation` on `AppKit`: the modifier's transform lives on the
/// wrapper layer's `sublayerTransform`, the one layer transform
/// `AppKit`'s display-time geometry sync does not rewrite (#3504). After
/// `displayIfNeeded` the anchored matrix must still be there, and a
/// wrapper resize must recompute it for the new bounds size.
#[cfg(all(target_os = "macos", feature = "scale", feature = "rotation"))]
mod transform_modifiers {
    use cocoa_ui::objc2::msg_send;
    use cocoa_ui::objc2::rc::autoreleasepool;
    use cocoa_ui::objc2_quartz_core::CATransform3D;
    use cocoa_ui::{PlatformView, Point, Rect, Retained, layer, view};
    use libtest_mimic::Trial;
    use waterui::ViewExt as _;
    use waterui::component::label::label;
    use waterui::style::Anchor;
    use waterui_apple::contract::NativeLeaf;
    use waterui_apple::native_test_support::pump_main_until;
    use waterui_backend_core::View;

    use super::AttachedWindow;

    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "transform_modifiers::scale_survives_display_and_resize",
                || {
                    scale_survives_display_and_resize();
                    Ok(())
                },
            ),
            Trial::test(
                "transform_modifiers::rotation_survives_display_and_resize",
                || {
                    rotation_survives_display_and_resize();
                    Ok(())
                },
            ),
        ]
    }

    /// `T(−A) · M · T(A)` expanded element-wise — the case's own
    /// arithmetic, not the production `concat` path it verifies. For an
    /// affine `m` the 2×2 part passes through while the translation row
    /// becomes `A − A·M + t` — what the anchor-maps-to-itself contract
    /// resolves to.
    fn expected(m: CATransform3D, anchor: Point, size: cocoa_ui::Size) -> CATransform3D {
        let ax = anchor.x * size.width;
        let ay = anchor.y * size.height;
        CATransform3D {
            m41: ay.mul_add(-m.m21, ax.mul_add(1.0 - m.m11, m.m41)),
            m42: ax.mul_add(-m.m12, ay.mul_add(1.0 - m.m22, m.m42)),
            m43: ay.mul_add(-m.m23, ax.mul_add(-m.m13, m.m43)),
            ..m
        }
    }

    /// All sixteen elements of two `CATransform3D`s within 1e-6 — the
    /// anchored math's exactness is the point, the tolerance covers only
    /// float representation.
    fn assert_transform_close(actual: CATransform3D, expected: CATransform3D, what: &str) {
        let flatten = |t: &CATransform3D| {
            [
                t.m11, t.m12, t.m13, t.m14, t.m21, t.m22, t.m23, t.m24, t.m31, t.m32, t.m33, t.m34,
                t.m41, t.m42, t.m43, t.m44,
            ]
        };
        for (i, (a, e)) in flatten(&actual).iter().zip(flatten(&expected)).enumerate() {
            assert!(
                (a - e).abs() < 1e-6,
                "{what}: m{}{} differs — {a} vs {e}",
                i / 4 + 1,
                i % 4 + 1
            );
        }
    }

    /// Renders `view` and orders its window in — the leaf and window must
    /// outlive the case's assertions.
    fn mount(view: impl View) -> (NativeLeaf, Retained<PlatformView>, AttachedWindow) {
        let leaf = autoreleasepool(|_| super::resolve::render(view));
        let wrapper = autoreleasepool(|_| view::retain_base(leaf.view()));
        let window = autoreleasepool(|_| super::mount_and_order_front(&wrapper));
        (leaf, wrapper, window)
    }

    /// `NSWindow.layoutIfNeeded` + `displayIfNeeded`, then a bounded
    /// run-loop turn — the sequence that used to reset the content
    /// layer's transform.
    fn layout_display_pump(wrapper: &PlatformView) {
        autoreleasepool(|_| {
            let window = view::window(wrapper).expect("the wrapper is in a window");
            // SAFETY: documented `NSWindow` layout/display entry points on
            // the main thread the harness provides.
            unsafe {
                let () = msg_send![&window, layoutIfNeeded];
                let () = msg_send![&window, displayIfNeeded];
            }
        });
        pump_main_until(0.1, || false);
    }

    /// The wrapper layer's `sublayerTransform` as it stands now.
    fn sublayer_transform(wrapper: &PlatformView) -> CATransform3D {
        autoreleasepool(|_| {
            layer::layer_of(wrapper)
                .expect("a transform modifier's wrapper is layer-backed")
                .sublayerTransform()
        })
    }

    /// The display–resize–display sequence both cases run: the transform
    /// survives the sync `displayIfNeeded` performs and follows the new
    /// bounds size after `resize`.
    fn assert_survives_display_and_resize(
        wrapper: &Retained<PlatformView>,
        m: CATransform3D,
        anchor: Point,
        resize: Rect,
    ) {
        layout_display_pump(wrapper);
        let size = autoreleasepool(|_| view::bounds(wrapper).size);
        assert_transform_close(
            sublayer_transform(wrapper),
            expected(m, anchor, size),
            "after display",
        );

        autoreleasepool(|_| view::set_frame(wrapper, resize));
        layout_display_pump(wrapper);
        let size = autoreleasepool(|_| view::bounds(wrapper).size);
        assert_transform_close(
            sublayer_transform(wrapper),
            expected(m, anchor, size),
            "after resize",
        );
    }

    /// `.scale(0.5, 0.5)` at the default centre anchor (#3504's own
    /// reproduction): `m11`/`m22` stay `0.5` after display, and the
    /// anchor's translation term tracks the wrapper's new size.
    fn scale_survives_display_and_resize() {
        let (_leaf, wrapper, _window) = mount(label("coming").scale(0.5, 0.5));
        assert_survives_display_and_resize(
            &wrapper,
            CATransform3D::new_scale(0.5, 0.5, 1.0),
            Point::new(0.5, 0.5),
            Rect::new(0.0, 0.0, 400.0, 300.0),
        );
    }

    /// `.rotation_from(30, …)` at a non-centre anchor: the rotation matrix
    /// and its anchor translation both survive display, and a quarter-x
    /// three-quarter-y anchor recomputes for the new size.
    fn rotation_survives_display_and_resize() {
        let (_leaf, wrapper, _window) =
            mount(label("coming").rotation_from(30.0, Anchor::new(0.25, 0.75)));
        assert_survives_display_and_resize(
            &wrapper,
            CATransform3D::new_rotation(30f64.to_radians(), 0.0, 0.0, 1.0),
            Point::new(0.25, 0.75),
            Rect::new(0.0, 0.0, 400.0, 300.0),
        );
    }
}
