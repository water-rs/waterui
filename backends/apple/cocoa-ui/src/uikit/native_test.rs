//! The application the `native_app` test harnesses run their cases in.
//!
//! `UIKit` drives part of its own machinery — the animation behind
//! `UIScrollView.setContentOffset(_:animated: true)` among it — from the
//! application's update cycle, which only exists once `UIApplicationMain`
//! has launched the process as an application and the system has connected
//! a window scene. [`run`] starts that application through [`super::run`],
//! runs the selected `libtest-mimic` trials on its main thread once the
//! scene's key window is up and the application is active, and ends the
//! process with the harness's result. [`window`] gives a case a window in
//! that scene; outside the application it panics, and [`run`] refuses to
//! start without the runner's launch, so no app-dependent case can pass
//! without the application around it. The bare `native` harnesses, which
//! `simctl spawn` runs without an application, keep their sceneless
//! windows and never reach this module.
//!
//! The `.github/scripts/nextest-ios-sim.sh` target runner launches the
//! process as an application: it wraps the test binary in an `.app` whose
//! `Info.plist` is the one [`native_test_info_plist!`](crate::native_test_info_plist)
//! embeds in the binary's `__TEXT,__info_plist` section. `simctl launch`
//! does not forward the application's exit status, so [`run`] writes it to
//! the file the runner names in [`STATUS_PATH_VARIABLE`] before exiting.
//!
//! # Safety
//!
//! The `unsafe` here schedules a block on the main run loop from the main
//! thread; the block runs on that same thread and never crosses threads.

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::process::ExitCode;

use block2::RcBlock;
use libtest_mimic::{Arguments, Trial};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_foundation::NSRunLoop;
use objc2_ui_kit::UIWindow;

use super::application::{ApplicationHandlers, WindowScene};
use super::window::{Window, application_is_active};
use super::{ViewController, window_root};
use crate::callback::guarded;
use crate::geometry::Rect;
use crate::native_test::pump_main_until;

const INFO_PLIST_LEN: usize = include_bytes!("native_test.plist").len();

/// The harness application's `Info.plist`: its bundle identity and the
/// scene manifest naming `SceneDelegate`, the class through which
/// [`super::run`] receives the window scene.
pub const INFO_PLIST: [u8; INFO_PLIST_LEN] = *include_bytes!("native_test.plist");

/// The environment variable naming the file [`run`] writes the process's
/// exit status to.
pub const STATUS_PATH_VARIABLE: &str = "WATERUI_NATIVE_TEST_STATUS";

/// The bound the launched application gets to connect its window scene
/// and become active.
const ACTIVATION_DEADLINE: f64 = 10.0;

/// Embeds [`INFO_PLIST`](crate::uikit::native_test::INFO_PLIST) in the
/// invoking binary's `__TEXT,__info_plist` section.
///
/// The target runner launches the binary as an application with that
/// bundle manifest. Invoke once at the root of a harness binary.
#[doc(hidden)]
#[macro_export]
macro_rules! native_test_info_plist {
    () => {
        #[used]
        #[unsafe(link_section = "__TEXT,__info_plist")]
        static NATIVE_TEST_INFO_PLIST: [u8; $crate::uikit::native_test::INFO_PLIST.len()] =
            $crate::uikit::native_test::INFO_PLIST;
    };
}

thread_local! {
    /// The window scene the harness application connected.
    static SCENE: RefCell<Option<WindowScene>> = const { RefCell::new(None) };
}

/// Runs the trials `arguments` selects inside a launched `UIApplication`
/// and exits the process with the harness's status.
///
/// `--list` is answered at once, without the application. Otherwise the
/// trials run on the main thread, sequentially, after the system has
/// connected the application's window scene, its key window is visible
/// and the application is active.
///
/// # Panics
///
/// If called off the main thread, if [`STATUS_PATH_VARIABLE`] is unset, if
/// the window scene does not connect or the application does not become
/// active within [`ACTIVATION_DEADLINE`] of launching, or if the status
/// file cannot be written.
pub fn run(arguments: Arguments, trials: impl FnOnce() -> Vec<Trial> + 'static) -> ! {
    if arguments.list {
        libtest_mimic::run(&arguments, trials()).exit();
    }
    let status_path = std::env::var_os(STATUS_PATH_VARIABLE).unwrap_or_else(|| {
        panic!(
            "{STATUS_PATH_VARIABLE} is unset — the harness application is launched by the \
             nextest-ios-sim.sh target runner, which names the file its exit status goes to"
        )
    });
    let mtm = MainThreadMarker::new().expect("a harness's `main` runs on the main thread");
    super::run(
        mtm,
        ApplicationHandlers::new(|scene| {
            let mtm = scene.main_thread();
            let window = Window::new(scene);
            window.set_root_view_controller(&ViewController::new(mtm, window_root(mtm)));
            window.make_key_and_visible();
            window.layout_if_needed();
            SCENE.with(|slot| {
                assert!(
                    slot.replace(Some(scene.clone())).is_none(),
                    "the harness application connected a second window scene"
                );
            });
            window
        })
        .did_finish_launching(move |_| {
            perform_on_main_run_loop(move || run_trials(&arguments, trials, &status_path));
        }),
    )
}

/// A hidden window at `frame` in the harness application's window scene.
///
/// # Panics
///
/// If no harness application is running — a case that asks for a window
/// must run inside [`run`].
#[must_use]
pub fn window(mtm: MainThreadMarker, frame: Rect) -> Retained<UIWindow> {
    let scene = SCENE.with(|slot| slot.borrow().clone()).expect(
        "a native case asked for a window outside the harness application — \
         its harness must run the trials through `native_test::run`",
    );
    let window = UIWindow::initWithWindowScene(UIWindow::alloc(mtm), scene.native());
    window.setFrame(frame.into());
    window
}

/// Runs `job` once from the main run loop — a run-loop block rather than a
/// main-queue block, so the main queue keeps draining while `job` pumps
/// the run loop.
fn perform_on_main_run_loop(job: impl FnOnce() + 'static) {
    let job = Cell::new(Some(job));
    let block = RcBlock::new(move || {
        let job = job
            .take()
            .expect("the main run loop runs a performed block once");
        guarded("native test harness", job);
    });
    // SAFETY: see the module safety note.
    unsafe { NSRunLoop::mainRunLoop().performBlock(&block) };
}

fn run_trials(arguments: &Arguments, trials: impl FnOnce() -> Vec<Trial>, status_path: &OsString) {
    let scene_is_connected = || SCENE.with(|slot| slot.borrow().is_some());
    assert!(
        pump_main_until(ACTIVATION_DEADLINE, || {
            scene_is_connected() && application_is_active()
        }),
        "the harness application did not connect its window scene and become active within \
         {ACTIVATION_DEADLINE} s of launching (scene connected: {}, active: {}) — the bundle's \
         `UISceneDelegateClassName` must name the `SceneDelegate` class `uikit::run` registers",
        scene_is_connected(),
        application_is_active()
    );
    let conclusion = libtest_mimic::run(arguments, trials());
    // `ExitCode` exposes no number on stable; recover the conclusion's by
    // comparison so the status file carries exactly the process's exit.
    let exit_code = conclusion.exit_code();
    let status = (0..=u8::MAX)
        .find(|&code| ExitCode::from(code) == exit_code)
        .expect("a libtest conclusion exits with a status byte");
    std::fs::write(status_path, status.to_string()).unwrap_or_else(|error| {
        panic!(
            "writing the exit status to {}: {error}",
            status_path.display()
        )
    });
    conclusion.exit();
}
