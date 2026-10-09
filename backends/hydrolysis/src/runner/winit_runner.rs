//! Desktop event loop on winit, including AccessKit integration.

use async_task::spawn_local as spawn_local_task;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::mem;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use accesskit::ActivationHandler;
use accesskit_winit::{
    Adapter as AccessKitAdapter, Event as AccessKitEvent, WindowEvent as AccessKitWindowEvent,
};
use executor_core::{
    LocalExecutor,
    async_task::{AsyncTask, Runnable},
    try_init_local_executor,
};
use nami::Signal;
use waterui::app::{
    App, AppParts, LastWindowPolicy, TerminationHandle, TerminationHost, TerminationKind,
};
use waterui::window::{Monitor, MonitorSelector, Window, WindowState};
use waterui_core::Environment;
#[cfg(hydrolysis_wayland_platform)]
use waterui_core::Str;
use waterui_graphics::gpu::RedrawHandle;
use waterui_text::FontCollection;

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, DeviceEvents, EventLoop};
#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
use winit::window::{Window as NativeWindow, WindowId};

use crate::platform::{PlatformWindow, WinitGpuContext, WinitWindow};
use crate::renderer::{
    FontFamilyResolution, HydrolysisRenderer, HydrolysisTextContextMenuMode,
    HydrolysisWindowOrigin, MenuShortcutRegistry, PopupWindowManager,
};
#[cfg(hydrolysis_wayland_platform)]
use crate::runner::x11_state_watch::{self, X11StateWatch};
use crate::runner::{
    RenderDiagnosticsConfig, RuntimeWindow, advance_runtime, handle_input_events_with,
    pump_window_semantics, render_window, runtime_window_origin,
};
use crate::text::SessionTextEngine;

#[derive(Debug)]
pub(super) enum RunnerEvent {
    PollLocalTasks,
    MountPendingWindows,
    /// A window's wake, posted by the [`RedrawHandle`] its [`WinitWindow`]
    /// hands GPU content and the engine: the window is redrawn here, on the
    /// event loop, because the handle runs on — and its last clone can be
    /// dropped on — threads that must never touch the winit window.
    RedrawWindow(WindowId),
    AccessKit(AccessKitEvent),
    /// The X11 state watch saw `_NET_WM_STATE`/`WM_STATE` change or the
    /// window (un)map — the minimize/restore transition winit drops (see
    /// `x11_state_watch`).
    #[cfg(hydrolysis_wayland_platform)]
    X11VisibilitySignal,
    /// Sent by the termination handler installed in [`run`]: the runner
    /// files a [`Required`](TerminationKind::Required) request with its
    /// [`TerminationHandle`].
    ///
    /// No windowing system turns a termination signal into a winit event, on
    /// any desktop platform, so the runner listens for the signals itself. The
    /// variant exists wherever that handler does — every target with signals or
    /// Windows console control events.
    #[cfg(any(unix, windows))]
    Terminate,
    /// Sent by the [`WindowCloser`](crate::renderer::WindowCloser) the
    /// runner installs: a declared `MenuItem::CloseWindow` was chosen, or
    /// its chord pressed, for the given window — the payload names it.
    #[cfg(hydrolysis_closable_windows)]
    CloseWindow(crate::renderer::WindowId),
    /// The termination machine finished its work — the runner's
    /// [`TerminationHost`] sent it — so teardown happens on the event loop,
    /// where runtime cleanup is safe.
    TerminationFinished,
    /// The console window closing, sent by `console_close_handler`, which
    /// blocks until the sender sends — once the machine reports `terminate`.
    #[cfg(target_os = "windows")]
    ConsoleClose(mpsc::Sender<()>),
}

/// What a termination signal does, given how many arrived before it.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminationAction {
    /// Ask the event loop to tear the runtime down, the way the last window
    /// closing does.
    RequestExit,
    /// Stop asking. The loop had its chance and did not take it.
    ForceExit,
}

/// Collapses repeated termination signals into "ask once, then stop asking".
///
/// Installing a handler replaces the default disposition of SIGINT, so without
/// this a wedged process becomes unkillable from the terminal that started it:
/// the graceful request is only reachable through the very event loop the hang
/// lives in.
#[cfg(windows)]
#[derive(Debug, Default)]
struct TerminationRequests {
    requested: std::sync::atomic::AtomicBool,
}

#[cfg(windows)]
impl TerminationRequests {
    fn record(&self) -> TerminationAction {
        if self
            .requested
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            TerminationAction::ForceExit
        } else {
            TerminationAction::RequestExit
        }
    }
}

/// Shell convention for a process killed by SIGINT (128 + 2). `ctrlc` does not
/// report which signal arrived, and Ctrl-C is what a user is pressing when the
/// forced path is reached.
#[cfg(windows)]
const FORCED_TERMINATION_EXIT_CODE: i32 = 130;

/// Turns the termination signals into [`RunnerEvent::Terminate`].
///
/// [`TerminationSignals`] owns the contract: the first delivery of a watched
/// signal is this send — the graceful request the event loop serves — and a
/// repeated signal ends the process by the signal's own default action. The
/// returned guard keeps the watcher alive for the duration of the run.
#[cfg(unix)]
fn install_termination_handler(
    event_proxy: &winit::event_loop::EventLoopProxy<RunnerEvent>,
) -> super::termination::TerminationSignals {
    let signal_proxy = event_proxy.clone();
    super::termination::TerminationSignals::install(move |_signal| {
        // The watcher reads deliveries on its own thread rather than inside a
        // signal handler, so waking the loop from here is an ordinary send.
        let _ = signal_proxy.send_event(RunnerEvent::Terminate);
    })
}

/// Turns the console control events into [`RunnerEvent::Terminate`].
///
/// The `termination` feature of `ctrlc` covers Ctrl+C and Ctrl+Break, so every
/// way a desktop shell or session manager asks a windowed app to stop reaches
/// the same teardown the last window closing does. Closing the console goes to
/// `console_close_handler` instead, which holds the event open until
/// `on_terminate` finished; a logoff or shutdown reaches the application
/// windows as `WM_ENDSESSION` (see `session_end_proc`).
#[cfg(windows)]
fn install_termination_handler(event_proxy: &winit::event_loop::EventLoopProxy<RunnerEvent>) {
    let requests = TerminationRequests::default();
    let signal_proxy = event_proxy.clone();
    ctrlc::set_handler(move || match requests.record() {
        TerminationAction::RequestExit => {
            // `ctrlc` runs this on a thread of its own rather than inside a
            // signal handler, so waking the loop from here is an ordinary send.
            let _ = signal_proxy.send_event(RunnerEvent::Terminate);
        }
        TerminationAction::ForceExit => {
            tracing::warn!(
                "hydrolysis runner: termination signal repeated, exiting without runtime teardown"
            );
            std::process::exit(FORCED_TERMINATION_EXIT_CODE);
        }
    })
    .expect("hydrolysis runner: failed to install the termination handler");
    install_console_close_handler(event_proxy);
}

/// Whether the event loop ends, given the application's last-window policy
/// and how many windows it still owns, mounted or waiting to be mounted.
///
/// Asked at startup, where an application that declares no window and quits
/// after its last one has nothing to run, and again whenever a window closes.
const fn ends_event_loop(policy: LastWindowPolicy, open_windows: usize) -> bool {
    match policy {
        LastWindowPolicy::Quit => open_windows == 0,
        LastWindowPolicy::StayResident => false,
    }
}

/// The window a `MenuItem::CloseWindow` request acts on: the window the
/// dispatch invoked it for — a chord's dispatching window, a popup row's
/// owner — resolved to its native id, provided it still lives. `None` when
/// no window may take the request: the source named no window (`None` in),
/// it is a popup (which owns no close button), or it is already gone.
/// Whether the window may close is the shared close path's check, not this
/// one's.
#[cfg(hydrolysis_closable_windows)]
fn close_window_target<K, W>(
    source: Option<K>,
    popup_ids: &std::collections::HashSet<K>,
    windows: &HashMap<K, W>,
) -> Option<K>
where
    K: Copy + Eq + std::hash::Hash,
{
    let native = source?;
    (!popup_ids.contains(&native) && windows.contains_key(&native)).then_some(native)
}

/// The runner's [`TerminationHost`]: the machine's `terminate` arrives on
/// the event loop as [`RunnerEvent::TerminationFinished`] — teardown only
/// runs on the loop thread. On Windows it also tells the session-end paths
/// that `on_terminate` finished, and both answers release the
/// `ShutdownBlockReason`s the `WM_QUERYENDSESSION` subclass created.
struct WinitTerminationHost {
    event_proxy: winit::event_loop::EventLoopProxy<RunnerEvent>,
    #[cfg(target_os = "windows")]
    session_end: Rc<SessionEnd>,
}

impl TerminationHost for WinitTerminationHost {
    fn terminate(&self) {
        #[cfg(target_os = "windows")]
        self.session_end.terminated();
        let _ = self
            .event_proxy
            .send_event(RunnerEvent::TerminationFinished);
    }
    fn refuse(&self) {
        #[cfg(target_os = "windows")]
        self.session_end.destroy_block_reasons();
    }
}

/// What the Windows session-end paths share with the runner's
/// [`TerminationHost`].
///
/// Windows ends the process as soon as a session-end notification returns —
/// `WM_ENDSESSION` on an application window for a logoff or shutdown, or
/// the console close event — so each of them holds its notification open
/// until the machine reports `terminate`, which is what proves
/// `on_terminate` ran to completion.
#[cfg(target_os = "windows")]
#[derive(Default)]
struct SessionEnd {
    /// HWNDs holding a `ShutdownBlockReasonCreate`, filled by the
    /// `WM_QUERYENDSESSION` subclass.
    block_reasons: RefCell<std::collections::HashSet<isize>>,
    /// Whether the machine reported `terminate`.
    terminated: std::cell::Cell<bool>,
    /// Console control handlers waiting for `terminate`, each released by a
    /// send on its channel.
    console_waiters: RefCell<Vec<mpsc::Sender<()>>>,
}

#[cfg(target_os = "windows")]
impl SessionEnd {
    /// Destroys every outstanding `ShutdownBlockReason` — on `terminate`
    /// because the veto is moot, on `refuse` because the veto is lifted.
    fn destroy_block_reasons(&self) {
        for hwnd in self.block_reasons.borrow_mut().drain() {
            // SAFETY: each hwnd is an application window this process owns
            // and the reason it holds was created by `session_end_proc`.
            unsafe {
                windows_sys::Win32::System::Shutdown::ShutdownBlockReasonDestroy(hwnd);
            }
        }
    }

    /// The machine reported `terminate`: release every session-end path
    /// waiting on it.
    fn terminated(&self) {
        self.destroy_block_reasons();
        self.terminated.set(true);
        for waiter in self.console_waiters.borrow_mut().drain(..) {
            // A waiter whose handler thread is gone has nothing to release.
            let _ = waiter.send(());
        }
    }

    /// Holds a console control handler until the machine reports
    /// `terminate` — released at once when it already has.
    fn release_console_on_terminate(&self, waiter: mpsc::Sender<()>) {
        if self.terminated.get() {
            let _ = waiter.send(());
        } else {
            self.console_waiters.borrow_mut().push(waiter);
        }
    }
}

/// State handed to the session-end subclass: the machine to report through,
/// what the host shares with the session-end paths, and the main-thread
/// executor's queue, which `WM_ENDSESSION` drives itself while it holds the
/// message open.
#[cfg(target_os = "windows")]
struct TerminationSubclass {
    termination: TerminationHandle,
    session_end: Rc<SessionEnd>,
    runnables: Rc<mpsc::Receiver<Runnable>>,
}

#[cfg(target_os = "windows")]
impl TerminationSubclass {
    /// Runs main-thread tasks until the machine reports `terminate`.
    ///
    /// The event loop is inside this window message, so it neither runs the
    /// executor nor pumps the thread's message queue until the message
    /// returns. This loop does both: it runs every runnable queued so far,
    /// then waits for the queue to receive input and dispatches it. A waker
    /// on any thread queues its runnable and posts the loop's wake-up
    /// message, so the wait ends for executor work as well as for the
    /// window and loop messages an `on_terminate` future may be awaiting.
    ///
    /// # Panics
    ///
    /// When `MsgWaitForMultipleObjectsEx` fails, with `GetLastError`.
    fn run_until_terminated(&self) {
        use windows_sys::Win32::Foundation::WAIT_FAILED;
        use windows_sys::Win32::System::Threading::INFINITE;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE,
            PeekMessageW, QS_ALLINPUT, TranslateMessage,
        };
        loop {
            while let Ok(runnable) = self.runnables.try_recv() {
                runnable.run();
            }
            if self.session_end.terminated.get() {
                return;
            }
            // SAFETY: no handles are passed, so the wait is on this thread's
            // message queue alone.
            let woke = unsafe {
                MsgWaitForMultipleObjectsEx(
                    0,
                    std::ptr::null(),
                    INFINITE,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                )
            };
            assert!(
                woke != WAIT_FAILED,
                "hydrolysis runner: waiting for on_terminate at session end failed: {}",
                std::io::Error::last_os_error()
            );
            // SAFETY: `MSG` is plain data, and the all-zero value is valid.
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            // SAFETY: `msg` is a writable `MSG`; a null HWND takes every
            // message of this thread.
            while unsafe { PeekMessageW(&raw mut msg, 0, 0, 0, PM_REMOVE) } != 0 {
                // SAFETY: `msg` was just filled by `PeekMessageW`.
                unsafe {
                    TranslateMessage(&raw const msg);
                    DispatchMessageW(&raw const msg);
                }
            }
        }
    }
}

/// `SetWindowSubclass` id for the session-end subclass. Unique per window
/// among this process's subclasses — this module is the only one that
/// subclasses application windows.
#[cfg(target_os = "windows")]
const TERMINATION_SUBCLASS_ID: usize = 1;

/// Answers the session-end messages on an application window.
///
/// `WM_QUERYENDSESSION` and `WM_ENDSESSION` arrive on every top-level window
/// of the process, so the subclass is installed on each application window
/// where the menu bar attaches its `HMENU`.
///
/// - `WM_QUERYENDSESSION` asks whether the session may end. With
///   termination hooks set the proc blocks the shutdown with a reason
///   string, answers `FALSE`, and files a cancellable request — the machine
///   deduplicates the per-window repetition. With no hook there is nothing
///   to ask, so the message passes through, letting the session end.
/// - `WM_ENDSESSION` with `wParam` `TRUE` says the session is ending
///   regardless — the user chose to end it anyway, or nobody vetoed. The
///   proc files a required request, then runs the local executor and
///   dispatches the thread's messages until the machine reports
///   `terminate`, so `on_terminate` has finished before the message returns
///   and Windows ends the process.
#[cfg(target_os = "windows")]
unsafe extern "system" fn session_end_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    _uidsubclass: usize,
    refdata: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::System::Shutdown::ShutdownBlockReasonCreate;
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_ENDSESSION, WM_QUERYENDSESSION};
    // SAFETY: `refdata` is the `TerminationSubclass` box installed by
    // `install_termination_subclass`, alive until
    // `remove_termination_subclass` reclaims it at window teardown.
    let subclass = unsafe { &*(refdata as *const TerminationSubclass) };
    match msg {
        WM_QUERYENDSESSION if subclass.termination.has_hooks() => {
            // SAFETY: `hwnd` is the window the message arrived on.
            unsafe {
                ShutdownBlockReasonCreate(hwnd, windows_sys::w!("The application is finishing up"));
            }
            subclass.session_end.block_reasons.borrow_mut().insert(hwnd);
            subclass.termination.request(TerminationKind::Cancellable);
            0
        }
        WM_ENDSESSION if wparam != 0 => {
            subclass.termination.request(TerminationKind::Required);
            subclass.run_until_terminated();
            0
        }
        // SAFETY: forwarding every unclaimed message to the previous proc.
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

/// Subclasses an application window's `HWND` for the session-end messages.
///
/// # Panics
///
/// When `SetWindowSubclass` fails, with the error `GetLastError` reports:
/// the window would otherwise end with the session without running
/// `on_terminate`.
#[cfg(target_os = "windows")]
fn install_termination_subclass(hwnd: isize, subclass: TerminationSubclass) {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    let subclass = Box::into_raw(Box::new(subclass));
    // SAFETY: `hwnd` is a live application window this process owns; the
    // `TerminationSubclass` allocation is reclaimed by
    // `remove_termination_subclass` before the window is destroyed.
    let installed = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(session_end_proc),
            TERMINATION_SUBCLASS_ID,
            subclass as usize,
        )
    };
    if installed == 0 {
        let error = std::io::Error::last_os_error();
        // SAFETY: the subclass was not installed, so nothing else holds the
        // allocation `Box::into_raw` produced above.
        drop(unsafe { Box::from_raw(subclass) });
        panic!(
            "hydrolysis runner: SetWindowSubclass failed on an application window, so its \
             session end would skip on_terminate: {error}"
        );
    }
}

/// Removes the session-end subclass before the window's `HWND` is
/// destroyed — the same teardown point the menu bar detaches at.
#[cfg(target_os = "windows")]
fn remove_termination_subclass(hwnd: isize) {
    use windows_sys::Win32::UI::Shell::{GetWindowSubclass, RemoveWindowSubclass};
    // SAFETY: `hwnd` is still a live application window (this runs before
    // the winit window is dropped).
    unsafe {
        let mut refdata = 0usize;
        if GetWindowSubclass(
            hwnd,
            Some(session_end_proc),
            TERMINATION_SUBCLASS_ID,
            &raw mut refdata,
        ) != 0
            && RemoveWindowSubclass(hwnd, Some(session_end_proc), TERMINATION_SUBCLASS_ID) != 0
            && refdata != 0
        {
            let subclass = Box::from_raw(refdata as *mut TerminationSubclass);
            // A reason this window still holds dies with it.
            subclass
                .session_end
                .block_reasons
                .borrow_mut()
                .remove(&hwnd);
        }
    }
}

/// The event loop the console control handler reports to.
///
/// `SetConsoleCtrlHandler` calls a bare function pointer with no context
/// argument, so the one piece of state the handler needs — how to reach the
/// loop — has to live in a process-wide slot. It is set once, by
/// [`install_console_close_handler`].
#[cfg(target_os = "windows")]
static CONSOLE_CLOSE: std::sync::OnceLock<winit::event_loop::EventLoopProxy<RunnerEvent>> =
    std::sync::OnceLock::new();

/// Holds the console close event open until `on_terminate` finished.
///
/// Windows ends the process as soon as the handler for this event returns,
/// and `ctrlc`'s handler returns at once, so the request it wakes the loop
/// with would never be served. This handler runs first — the system calls
/// the most recently registered handler first — files a required request on
/// the loop, and blocks its own thread, which the system created for the
/// event, until the machine reports `terminate`. Ctrl+C and Ctrl+Break fall
/// through to `ctrlc`. The logoff and shutdown control events never arrive:
/// the system withholds them from a process that loads user32, and the
/// application windows receive `WM_ENDSESSION` instead.
#[cfg(target_os = "windows")]
unsafe extern "system" fn console_close_handler(
    ctrl_type: u32,
) -> windows_sys::Win32::Foundation::BOOL {
    use windows_sys::Win32::System::Console::CTRL_CLOSE_EVENT;
    if ctrl_type != CTRL_CLOSE_EVENT {
        return 0;
    }
    let event_proxy = CONSOLE_CLOSE
        .get()
        .expect("the console handler is registered after its event loop is recorded");
    let (released, release) = mpsc::channel();
    // A loop that already exited drops the event and its sender, which ends
    // the wait at once.
    let _ = event_proxy.send_event(RunnerEvent::ConsoleClose(released));
    let _ = release.recv();
    1
}

/// Registers [`console_close_handler`] ahead of `ctrlc`'s handler.
///
/// # Panics
///
/// When it is called a second time in the process, or when
/// `SetConsoleCtrlHandler` fails.
#[cfg(target_os = "windows")]
fn install_console_close_handler(event_proxy: &winit::event_loop::EventLoopProxy<RunnerEvent>) {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    assert!(
        CONSOLE_CLOSE.set(event_proxy.clone()).is_ok(),
        "hydrolysis runner: the console close handler is installed once per process"
    );
    // SAFETY: `console_close_handler` matches `PHANDLER_ROUTINE` and lives
    // for the whole process.
    let installed = unsafe { SetConsoleCtrlHandler(Some(console_close_handler), 1) };
    assert!(
        installed != 0,
        "hydrolysis runner: failed to install the console close handler: {}",
        std::io::Error::last_os_error()
    );
}

struct PendingWindow {
    window: Window,
    activates: bool,
}

impl PendingWindow {
    const fn application(window: Window) -> Self {
        Self {
            window,
            activates: true,
        }
    }

    const fn popup(window: Window) -> Self {
        Self {
            window,
            activates: false,
        }
    }
}

impl From<AccessKitEvent> for RunnerEvent {
    fn from(value: AccessKitEvent) -> Self {
        Self::AccessKit(value)
    }
}

#[derive(Clone)]
struct InitialAccessibilityTree {
    tree_update: accesskit::TreeUpdate,
}

impl ActivationHandler for InitialAccessibilityTree {
    fn request_initial_tree(&mut self) -> Option<accesskit::TreeUpdate> {
        Some(self.tree_update.clone())
    }
}

#[derive(Clone)]
struct WinitMainThreadExecutor {
    runnable_tx: mpsc::Sender<Runnable>,
    event_proxy: winit::event_loop::EventLoopProxy<RunnerEvent>,
}

impl LocalExecutor for WinitMainThreadExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let runnable_tx = self.runnable_tx.clone();
        let event_proxy = self.event_proxy.clone();
        let (runnable, task) = spawn_local_task(fut, move |runnable: Runnable| {
            if let Err(unsent) = runnable_tx.send(runnable) {
                // Teardown race: a waker held by another thread fired after
                // the event loop dropped the receiver. Dropping a
                // `spawn_local` runnable off its spawning thread panics by
                // design (async-task's thread check), so leak it instead —
                // bounded to shutdown, reclaimed at process exit.
                std::mem::forget(unsent);
                return;
            }
            let _ = event_proxy.send_event(RunnerEvent::PollLocalTasks);
        });
        runnable.schedule();
        AsyncTask::from(task)
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "startup wires the event loop, the first windows and the executor in one ordered sequence; splitting it would obscure the setup order"
)]
pub fn run(
    app: App,
    style: impl crate::Style,
    inspector: Option<waterui::inspector::InspectorRuntime>,
) {
    let AppParts {
        windows,
        menu_bar,
        env,
        last_window,
        termination,
    } = app.into_parts();
    // No early return for a windowless `Quit` launch: its required
    // termination goes through the machine like every other path, and its
    // hook future needs the loop running — the first `resumed` files it.
    let mut event_loop_builder = EventLoop::<RunnerEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        // Launch activation belongs to a window that is actually shown with
        // `Activation::OnShow` at startup. A resident app that mounts no
        // window, or only windows that defer focus (`OnClick`/`Never`),
        // must not pull focus when it launches — a drop-down terminal
        // started at login or by a hotkey daemon is the case this
        // protects. `ActivationPolicy::Regular` stays either way, so the
        // Dock icon and menu bar keep working, and a window shown later
        // still takes focus per its own policy.
        let activate_at_launch = windows.iter().any(|window| {
            window.activation == waterui::window::Activation::OnShow
                && window.state.snapshot() != waterui::window::WindowState::Closed
        });
        // `NSApp.mainMenu` belongs to hydrolysis: `NativeMenuBar::install`
        // has already projected the declared menu bar onto it, and winit's
        // launch-time default menu (EventLoopBuilderExtMacOS::with_default_menu,
        // `true` by default) would replace it when the application finishes
        // launching (water-rs/hydrolysis#321).
        event_loop_builder
            .with_activation_policy(ActivationPolicy::Regular)
            .with_default_menu(false)
            .with_activate_ignoring_other_apps(activate_at_launch);
    }
    let event_loop = event_loop_builder
        .build()
        .expect("hydrolysis runner: failed to create event loop");
    let event_proxy = event_loop.create_proxy();
    let (local_runnable_tx, local_runnable_rx) = mpsc::channel::<Runnable>();
    let local_runnable_rx = Rc::new(local_runnable_rx);
    let local_executor = WinitMainThreadExecutor {
        runnable_tx: local_runnable_tx,
        event_proxy: event_proxy.clone(),
    };

    // The reactive graph is thread-confined, so its observer is installed here,
    // on the thread that owns the event loop, and lives as long as the loop.
    #[cfg(feature = "inspector-signals")]
    let _signal_scope = inspector
        .as_ref()
        .map(waterui::inspector::InspectorRuntime::observe_signals);

    let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
    waterui_core::install_application_resources(&mut env);
    waterui::inspector::install(&mut env, inspector);
    let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
    let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
    super::install_native_component_hooks(&mut env);
    #[cfg(unix)]
    let _termination_signals = install_termination_handler(&event_proxy);
    #[cfg(windows)]
    install_termination_handler(&event_proxy);
    env.insert(HydrolysisTextContextMenuMode::Overlay);
    env.insert(waterui::window::WindowManager::new({
        let pending_window_queue = Rc::clone(&pending_window_queue);
        let event_proxy = event_proxy.clone();
        move |window| {
            pending_window_queue
                .borrow_mut()
                .push(PendingWindow::application(window));
            let _ = event_proxy.send_event(RunnerEvent::MountPendingWindows);
        }
    }));
    env.insert(PopupWindowManager::new({
        let pending_window_queue = Rc::clone(&pending_window_queue);
        let event_proxy = event_proxy.clone();
        move |window| {
            pending_window_queue
                .borrow_mut()
                .push(PendingWindow::popup(window));
            let _ = event_proxy.send_event(RunnerEvent::MountPendingWindows);
        }
    }));
    // Every window of the app resolves menu chords through the shared
    // registry: a compositor that hands a popup window keyboard focus routes
    // the menu's chords to that window's dispatch, which must resolve them
    // while the menu is open (water-rs/hydrolysis#247).
    let _ = env.get_or_insert_with::<MenuShortcutRegistry, _>(MenuShortcutRegistry::default);
    crate::theme::install_theme_tokens(&mut env, Some(&style));
    let theme: Rc<dyn crate::engine::WidgetTheme> = Rc::new(style);
    env.insert(waterui_core::ViewRenderer::new(
        crate::view_renderer::HydrolysisViewRenderer::new(Rc::clone(&theme)),
    ));
    // The application's fonts, discovered once. Every window's renderer is
    // seeded from this collection, and a self-drawn component that typesets
    // text itself reads it out of the environment instead of enumerating the
    // system's fonts for itself.
    let fonts = crate::text::fonts::native_collection(&env);
    fonts.clone().install(&mut env);
    let window_icon =
        load_staged_window_icon(waterui_core::ResourceContext::from_environment(&env));
    // The machine installs `Quit` into the composition-root environment and
    // hands its hooks that environment as it stands now; their futures run
    // on the local executor the first `resumed` installs.
    #[cfg(target_os = "windows")]
    let session_end = Rc::new(SessionEnd::default());
    let termination = termination.start(
        &mut env,
        WinitTerminationHost {
            event_proxy: event_proxy.clone(),
            #[cfg(target_os = "windows")]
            session_end: Rc::clone(&session_end),
        },
    );
    // A declared `MenuItem::CloseWindow` closes the window it was invoked
    // for through the runner, which owns the windows: the primitive only
    // posts the target's identity to the event loop.
    #[cfg(hydrolysis_closable_windows)]
    env.insert(crate::renderer::WindowCloser::new({
        let event_proxy = event_proxy.clone();
        move |window| {
            event_proxy
                .send_event(RunnerEvent::CloseWindow(window))
                .expect("the runner's event loop outlives the WindowCloser it installs");
        }
    }));
    // The app's menu bar resolves after the machine installed `Quit` and
    // the close primitive went in, so a declared `MenuItem::Quit` or
    // `MenuItem::CloseWindow` and `|quit: Quit|` command actions find them.
    // Its command chords arm on the shared registry, and the native
    // menu-bar surface installs where the platform has one — see `menu_bar`
    // for the per-platform contract (the two chord paths see disjoint keys,
    // so dispatch stays exactly-once).
    // Only the winit runner turns the resolved menus into a native
    // surface: `NSApp.mainMenu` on macOS, an `HMENU` per application
    // window on Windows. Every other runner just arms the chords. The
    // event loop is the main thread, which is what the install's
    // `MainThreadMarker` contract needs.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let native_menu_bar = crate::platform::native_menu_bar::NativeMenuBar::install(
        &super::menu_bar::register_menu_bar(&menu_bar, &env),
        &env,
    );
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    super::menu_bar::register_menu_bar(&menu_bar, &env);
    let mut runner = WinitRunner {
        env,
        theme,
        fonts,
        window_icon,
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        native_menu_bar,
        pending_windows: windows
            .into_iter()
            .map(PendingWindow::application)
            .collect(),
        last_window,
        pending_window_queue,
        windows: HashMap::new(),
        popup_window_ids: std::collections::HashSet::new(),
        gpu_context: None,
        accesskit_adapters: HashMap::new(),
        last_accessibility_updates: HashMap::new(),
        local_executor: Some(local_executor),
        local_runnable_rx,
        event_proxy,
        render_diagnostics_config,
        outside_pointer_presses: 0,
        focused_window: None,
        last_pointer_window: None,
        termination,
        #[cfg(target_os = "windows")]
        session_end,
        #[cfg(hydrolysis_wayland_platform)]
        x11_state_watch: None,
    };

    event_loop.set_control_flow(ControlFlow::Wait);
    // Raw pointer presses are reported for every device on the seat, not just
    // presses over our own windows — that is the only way a press on the
    // desktop or in another app reaches us, and it is what dismisses popup
    // windows when the pointer goes down outside them.
    event_loop.listen_device_events(DeviceEvents::Always);
    let run_result = event_loop.run_app(&mut runner);
    waterui_locale::shutdown_current_thread_runtime_locale_state();
    let _ = runner.drain_local_executor_queue();
    run_result.expect("hydrolysis runner: event loop failed");
}

struct WinitRunner {
    env: Environment,
    /// The style the runtime was launched with: every window's renderer
    /// measures and encodes with this widget theme.
    theme: Rc<dyn crate::engine::WidgetTheme>,
    /// The application's font collection, the same one the environment carries.
    /// Every window's renderer is seeded from it.
    fonts: FontCollection,
    /// Taskbar/window icon staged by the water CLI next to the asset bundle.
    /// X11 and Windows honor it; macOS uses the bundle's icns and Wayland
    /// resolves icons through the desktop entry instead.
    window_icon: Option<winit::window::Icon>,
    /// The installed `App::menu_bar` native surface (macOS
    /// `NSApp.mainMenu`, Windows per-window `HMENU`): kept alive for the
    /// app's duration and pumped on every event-loop pass. Linux has no
    /// surface — the chords are still armed (see `menu_bar`).
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_menu_bar: crate::platform::native_menu_bar::NativeMenuBar,
    pending_windows: Vec<PendingWindow>,
    /// What the application does once it has no window left.
    last_window: LastWindowPolicy,
    pending_window_queue: Rc<RefCell<Vec<PendingWindow>>>,
    windows: HashMap<WindowId, RuntimeWindow<WinitWindow>>,
    /// Transient popup windows (mounted through `PopupWindowManager`): a
    /// pointer press that lands outside every app window dismisses them.
    popup_window_ids: std::collections::HashSet<WindowId>,
    gpu_context: Option<WinitGpuContext>,
    accesskit_adapters: HashMap<WindowId, AccessKitAdapter>,
    last_accessibility_updates: HashMap<WindowId, accesskit::TreeUpdate>,
    /// The executor is installed the first time the active event loop is
    /// reachable (the first `resumed`), which is the earliest winit 0.30
    /// exposes monitor enumeration.
    local_executor: Option<WinitMainThreadExecutor>,
    /// The main-thread executor's queue — shared with the Windows
    /// session-end subclass, which drives it while it holds `WM_ENDSESSION`
    /// open.
    local_runnable_rx: Rc<mpsc::Receiver<Runnable>>,
    event_proxy: winit::event_loop::EventLoopProxy<RunnerEvent>,
    render_diagnostics_config: RenderDiagnosticsConfig,
    /// Raw pointer presses seen at the device level but not (yet) matched by a
    /// window-level pointer event in the same event batch. What is left over
    /// when the batch settles is a press outside every window we own.
    outside_pointer_presses: u32,
    /// The window winit last reported `Focused(true)` for — the `Focused`
    /// monitor selector's answer (water-rs/waterui#1302).
    focused_window: Option<WindowId>,
    /// The window a pointer event last arrived on. Wayland exposes no global
    /// pointer position, so for `MonitorSelector::Pointer` this window's
    /// monitor stands in for the pointer's home.
    last_pointer_window: Option<WindowId>,
    /// The termination machine every quit path reports through: signals,
    /// the last window closing, `Quit` in the environment, Windows'
    /// `WM_QUERYENDSESSION`.
    termination: TerminationHandle,
    /// What the Windows session-end paths — the window subclass and the
    /// console control handler — share with the `TerminationHost`.
    #[cfg(target_os = "windows")]
    session_end: Rc<SessionEnd>,
    /// The second-connection `_NET_WM_STATE`/unmap watch that delivers the
    /// X11 transitions winit drops. `None` on Wayland, and stays `None` if
    /// no connection could be opened — the coverage is then what winit
    /// delivers, the documented gap.
    #[cfg(hydrolysis_wayland_platform)]
    x11_state_watch: Option<X11StateWatch>,
}

/// Loads the window icon the water CLI stages into the asset bundle root.
///
/// Absence is a legitimate state (bare `cargo run`, tests, previews without
/// staging); a present-but-undecodable icon is reported and skipped.
fn load_staged_window_icon(
    resources: &waterui_core::ResourceContext,
) -> Option<winit::window::Icon> {
    let root = waterui_assets::bundle_root(resources);
    let path = root.join(waterui_assets::WINDOW_ICON_FILE);
    let file = std::fs::File::open(&path).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = match decoder.read_info() {
        Ok(reader) => reader,
        Err(error) => {
            tracing::warn!(
                "staged window icon {} is not decodable: {error}",
                path.display()
            );
            return None;
        }
    };
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let info = match reader.next_frame(&mut pixels) {
        Ok(info) => info,
        Err(error) => {
            tracing::warn!(
                "staged window icon {} is not decodable: {error}",
                path.display()
            );
            return None;
        }
    };
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        tracing::warn!(
            "staged window icon {} must be 8-bit RGBA, got {:?}/{:?}",
            path.display(),
            info.color_type,
            info.bit_depth
        );
        return None;
    }
    pixels.truncate(info.buffer_size());
    match winit::window::Icon::from_rgba(pixels, info.width, info.height) {
        Ok(icon) => Some(icon),
        Err(error) => {
            tracing::warn!("staged window icon {} is unusable: {error}", path.display());
            None
        }
    }
}

fn native_window_attributes(
    window: &Window,
    env: &Environment,
    activates: bool,
    icon: Option<winit::window::Icon>,
) -> winit::window::WindowAttributes {
    let frame = crate::platform::validated_window_frame(window.frame.snapshot());
    // A Fullscreen request present at creation travels as a window
    // attribute — the same way the position does — so the window manager
    // sees it with the map request instead of after it. The retry in
    // `apply_properties` still re-delivers it on the first mapped event:
    // a state written between creation and map, or a manager that ignored
    // the attribute, is covered by the same mapped signal.
    let state = window.state.snapshot();
    let fullscreen = matches!(state, waterui::window::WindowState::Fullscreen);
    let maximized = matches!(state, waterui::window::WindowState::Maximized);
    let attributes = NativeWindow::default_attributes()
        .with_window_icon(icon)
        .with_title(window.display_title().snapshot().as_str())
        .with_resizable(window.resizable)
        .with_enabled_buttons(crate::platform::enabled_window_buttons(window.closable))
        .with_visible(false)
        .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
        .with_maximized(maximized)
        .with_window_level(match window.level.snapshot() {
            waterui::window::WindowLevel::Normal => winit::window::WindowLevel::Normal,
            waterui::window::WindowLevel::AlwaysOnTop => winit::window::WindowLevel::AlwaysOnTop,
        })
        // `Activation::OnClick` and `Never` both map without activation: the
        // platform parts that `with_active` cannot express (X11 `WM_HINTS`,
        // the AppKit style mask, `WS_EX_NOACTIVATE`) are applied after
        // creation, before the window is mapped.
        .with_active(activates && window.activation == waterui::window::Activation::OnShow)
        .with_transparent(super::window_requires_transparency(window, env))
        .with_decorations(!matches!(
            window.style.snapshot(),
            waterui::window::WindowStyle::Borderless
        ))
        .with_inner_size(winit::dpi::LogicalSize::new(
            f64::from(frame.width()),
            f64::from(frame.height()),
        ))
        // The requested position is a creation-time attribute: on X11 a
        // position set on an unmapped window is dropped, so it must travel
        // with the map request for the window manager to honor it.
        .with_position(winit::dpi::LogicalPosition::new(
            f64::from(frame.x()),
            f64::from(frame.y()),
        ));

    // The window's desktop identity: on X11 the `WM_CLASS` pair is split
    // between its class (winit's `general`, the window's resolved `app_id`)
    // and its instance (`res_name`, the window's `instance_name` — which
    // resolves to the `app_id` when undeclared); on Wayland the `app_id`
    // alone names the `xdg_toplevel`. Window-manager rules, desktop-file
    // matching and dock grouping land on the identifier the build compiled
    // in as `WATERUI_APP_ID` or the window declared itself. An empty
    // identity leaves winit's default (the executable name) in place.
    #[cfg(hydrolysis_wayland_platform)]
    let attributes = {
        use winit::platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11};
        match window_desktop_identity(window) {
            Some((class, instance)) => WindowAttributesExtWayland::with_name(
                WindowAttributesExtX11::with_name(attributes, class.as_str(), instance.as_str()),
                class.as_str(),
                instance.as_str(),
            ),
            None => attributes,
        }
    };
    // Resize increments write `WM_NORMAL_HINTS` — a stored property, so they
    // can travel with the map request safely. `with_resize_increments` takes
    // a size rather than an option, so the attribute applies conditionally.
    match window.resize_increments.as_ref() {
        Some(signal) => {
            let size = signal.snapshot();
            attributes.with_resize_increments(winit::dpi::LogicalSize::new(
                f64::from(size.width),
                f64::from(size.height),
            ))
        }
        None => attributes,
    }
}

/// The window's desktop identity as a `(class, instance)` pair.
///
/// `class` is the window's resolved `app_id` — the X11 `WM_CLASS` class
/// part and the Wayland `app_id`; `instance` is the window's
/// `instance_name`, resolving to the `app_id` when undeclared so a window
/// that does not opt in keeps the historical one-name behaviour. `None`
/// when the window and the application both leave the identity unset, so
/// winit's default (the executable name) applies.
#[cfg(hydrolysis_wayland_platform)]
fn window_desktop_identity(window: &Window) -> Option<(Str, Str)> {
    let class = window.display_app_id();
    (!class.is_empty()).then(|| (class, window.display_instance_name()))
}

impl WinitRunner {
    fn drain_runnable_queue(local_runnable_rx: &mpsc::Receiver<Runnable>) -> bool {
        let mut drained = false;
        while let Ok(runnable) = local_runnable_rx.try_recv() {
            drained = true;
            runnable.run();
        }
        drained
    }

    fn drain_local_executor_queue(&self) -> bool {
        Self::drain_runnable_queue(&self.local_runnable_rx)
    }

    fn exit_after_runtime_cleanup(&self, event_loop: &ActiveEventLoop) {
        waterui_locale::shutdown_current_thread_runtime_locale_state();
        let _ = self.drain_local_executor_queue();
        event_loop.exit();
    }

    /// Files a required termination when the application's last-window
    /// policy says a runner with no window left stops. The loop keeps
    /// running — possibly with zero windows — until `on_terminate` finishes
    /// and the host's `TerminationFinished` reaches
    /// [`Self::exit_after_runtime_cleanup`].
    fn exit_if_last_window_closed(&self) {
        if ends_event_loop(
            self.last_window,
            self.windows.len() + self.pending_windows.len(),
        ) {
            self.termination.request(TerminationKind::Required);
        }
    }

    fn current_window_origin(runtime: &RuntimeWindow<WinitWindow>) -> HydrolysisWindowOrigin {
        let native_window = runtime.platform.native_window();
        if let Ok(position) = native_window.outer_position() {
            let logical = position.to_logical::<f64>(native_window.scale_factor());
            return HydrolysisWindowOrigin {
                x: crate::num_cast::f64_as_f32(logical.x),
                y: crate::num_cast::f64_as_f32(logical.y),
            };
        }
        runtime_window_origin(runtime)
    }

    /// Realizes `native_window` as the platform window of `window` on the
    /// runner's shared GPU context, creating that context with the first
    /// window.
    ///
    /// The window's wake — what GPU content and the engine call for another
    /// frame — posts [`RunnerEvent::RedrawWindow`] to this loop. It owns no
    /// part of the winit window, so it may run and drop on any thread.
    fn create_platform_window(
        &mut self,
        native_window: Arc<NativeWindow>,
        window: &Window,
    ) -> WinitWindow {
        let wake = {
            let event_proxy = self.event_proxy.clone();
            let window_id = native_window.id();
            RedrawHandle::new(move || {
                // A loop that already exited has no window left to redraw.
                let _ = event_proxy.send_event(RunnerEvent::RedrawWindow(window_id));
            })
        };
        let (platform, gpu_context) = pollster::block_on(WinitWindow::new_with_shared_gpu(
            native_window,
            wake,
            self.gpu_context.as_ref(),
            super::window_requires_transparency(window, &self.env),
        ));
        if self.gpu_context.is_none() {
            self.gpu_context = Some(gpu_context);
        }
        platform
    }

    fn create_runtime_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        pending: PendingWindow,
    ) -> (RuntimeWindow<WinitWindow>, AccessKitAdapter) {
        let PendingWindow { window, activates } = pending;
        let activation = window.activation;
        // `Window::placement` resolves each time the window is shown — and a
        // winit window is shown exactly once, at mount (a `Closed` window is
        // destroyed, never re-shown) — so the resolved rect is written into
        // `frame` here, before the attributes carry it into the map request.
        if let Some(placement) = window.placement.as_ref() {
            let monitor = self.resolve_placement_monitor(event_loop, placement.monitor);
            window.frame.set((placement.place)(&monitor));
        }
        let attributes =
            native_window_attributes(&window, &self.env, activates, self.window_icon.clone());

        let native_window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("hydrolysis runner: failed to create winit window"),
        );
        #[cfg(hydrolysis_wayland_platform)]
        if let Some(xid) = x11_state_watch::x11_window_id(&native_window) {
            // The window is on X11, so the app's own server connection is
            // already up — every watch step is expected to succeed, and a
            // failure means minimize would go undetected. The window's
            // creation fails rather than silently covering the gap.
            if self.x11_state_watch.is_none() {
                self.x11_state_watch = Some(
                    X11StateWatch::connect(self.event_proxy.clone())
                        .expect("hydrolysis runner: failed to start the X11 state watch"),
                );
            }
            self.x11_state_watch
                .as_ref()
                .expect("hydrolysis runner: X11 state watch missing after connect")
                .select(xid)
                .expect("hydrolysis runner: failed to subscribe to X11 window state events");
        }
        #[cfg(target_os = "windows")]
        if activates {
            // Windows' menu bar lives on the window: attach the app bar's
            // HMENU to this HWND (see `menu_bar` for the platform contract),
            // and subclass the same HWND so a session shutdown asks the
            // termination machine before ending the process.
            use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = native_window.window_handle()
                && let RawWindowHandle::Win32(win) = handle.as_raw()
            {
                self.native_menu_bar.attach_hwnd(
                    win.hwnd.get(),
                    crate::renderer::WindowId::Winit(native_window.id()),
                );
                install_termination_subclass(
                    win.hwnd.get(),
                    TerminationSubclass {
                        termination: self.termination.clone(),
                        session_end: Rc::clone(&self.session_end),
                        runnables: Rc::clone(&self.local_runnable_rx),
                    },
                );
            }
        }
        let mut platform = self.create_platform_window(native_window, &window);
        platform.apply_properties(&window);
        let text = SessionTextEngine::from_collection(&self.fonts, FontFamilyResolution::Lenient);
        let renderer = HydrolysisRenderer::with_engine(Rc::clone(&self.theme), text);
        let closable = window.closable;
        let mut runtime =
            RuntimeWindow::new(window, platform, renderer, self.render_diagnostics_config);
        runtime
            .renderer
            .set_window_id(crate::renderer::WindowId::Winit(runtime.platform.id()));
        runtime.renderer.set_window_closable(closable);
        if !activates {
            self.popup_window_ids.insert(runtime.platform.id());
        }
        let _ = pump_window_semantics(&mut runtime, &self.env);
        // The adapter is created unconditionally: accesskit resolves the
        // platform accessibility bus itself, lazily, so a missing org.a11y.Bus
        // is its concern — probing it up front only raced the bus's startup.
        let adapter = {
            let initial_tree_update = runtime.renderer.take_accessibility_tree_update().expect(
                "hydrolysis winit accessibility: initial tree update missing after initial semantic rebuild",
            );
            let last_tree_update = initial_tree_update.clone();
            let adapter = AccessKitAdapter::with_mixed_handlers(
                event_loop,
                runtime.platform.native_window(),
                InitialAccessibilityTree {
                    tree_update: initial_tree_update,
                },
                self.event_proxy.clone(),
            );
            tracing::trace!(
                target: "waterui::hydrolysis::a11y",
                window_id = ?runtime.platform.id(),
                title = runtime.window.title.snapshot().as_str(),
                "created accesskit adapter for window"
            );
            self.last_accessibility_updates
                .insert(runtime.platform.id(), last_tree_update);
            adapter
        };
        // The activation parts window attributes cannot express, applied
        // before the window maps (see `with_active` above).
        crate::runner::placement::apply_activation(
            event_loop,
            runtime.platform.native_window(),
            activation,
        );
        // A window declared `Closed` at mount stays unmapped: ordering it
        // front and reaping it in `remove_closed_windows` would flash it on
        // screen — and `makeKeyAndOrderFront` would activate the app at
        // launch, the resident drop-down terminal bug this guards. Shown
        // windows map through `show_at_mount`, which orders `OnClick`/`Never`
        // windows front without activation (see that function).
        if runtime.window.state.snapshot() != WindowState::Closed {
            crate::runner::placement::show_at_mount(runtime.platform.native_window(), activation);
            if activates && activation == waterui::window::Activation::OnShow {
                runtime.platform.native_window().focus_window();
            }
        }
        (runtime, adapter)
    }

    /// Installs the monitored main-thread executor on the first active event
    /// loop callback, still ahead of the first window mount.
    ///
    /// `available_monitors` exists only on `ActiveEventLoop`/`Window` in winit
    /// 0.30, so the highest connected refresh rate can only be read here.
    fn start_main_thread_executor(&mut self, event_loop: &ActiveEventLoop) {
        let Some(local_executor) = self.local_executor.take() else {
            return;
        };
        let refresh_rate = event_loop
            .available_monitors()
            .filter_map(|monitor| monitor.refresh_rate_millihertz())
            .max()
            .and_then(NonZeroU32::new)
            .map_or_else(
                || {
                    // Some Wayland compositors report no rate (winit returns
                    // None); pace the executor's frame budget at the headless rate.
                    tracing::debug!(
                        "hydrolysis runner: no monitor reported a refresh rate, executor paced at the headless rate"
                    );
                    waterui::task::RefreshRate::HEADLESS
                },
                waterui::task::RefreshRate::from_millihertz,
            );
        let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
            local_executor,
            refresh_rate,
            self.env
                .get::<waterui::inspector::InspectorRuntime>()
                .map(waterui::inspector::InspectorRuntime::runtime_probe),
        ));
        // Locale changes reach views through a mailbox, whose pump needs the
        // executor installed just above.
        waterui_locale::start_system_locale_listener();
    }

    fn mount_pending_windows(&mut self, event_loop: &ActiveEventLoop) {
        self.start_main_thread_executor(event_loop);
        let mut pending = mem::take(&mut self.pending_windows);
        pending.extend(self.pending_window_queue.borrow_mut().drain(..));
        for pending in pending {
            let (runtime, adapter) = self.create_runtime_window(event_loop, pending);
            let id = runtime.platform.id();
            self.windows.insert(id, runtime);
            self.accesskit_adapters.insert(id, adapter);
        }
    }

    /// The monitor a window's [`MonitorSelector`] resolves to, evaluated
    /// with the event loop in hand — called once per window, at mount (the
    /// only "shown" a winit window has; see `placement` module docs).
    fn resolve_placement_monitor(
        &self,
        event_loop: &ActiveEventLoop,
        selector: MonitorSelector,
    ) -> Monitor {
        crate::runner::placement::resolve_placement_monitor(
            &crate::runner::placement::PlacementContext {
                event_loop,
                focused_window: self
                    .focused_window
                    .and_then(|id| self.windows.get(&id))
                    .map(|runtime| runtime.platform.native_window()),
                pointer_window: self
                    .last_pointer_window
                    .and_then(|id| self.windows.get(&id))
                    .map(|runtime| runtime.platform.native_window()),
            },
            selector,
        )
    }

    fn handle_input_events(runtime: &mut RuntimeWindow<WinitWindow>, env: &Environment) -> bool {
        handle_input_events_with(runtime, env, |runtime, env| {
            env.extending(Self::current_window_origin(runtime))
        })
    }

    /// Lets the native menu bar drop a closing window's attached bar — and
    /// removes the session-end subclass — before the winit window (and
    /// its HWND) is destroyed, so detaching the menu and both subclass
    /// procs only ever run on live handles.
    #[cfg(target_os = "windows")]
    fn detach_menu_bar_hwnd(&self, runtime: &RuntimeWindow<WinitWindow>) {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = runtime.platform.native_window().window_handle()
            && let RawWindowHandle::Win32(win) = handle.as_raw()
        {
            self.native_menu_bar.detach_hwnd(win.hwnd.get());
            remove_termination_subclass(win.hwnd.get());
        }
    }

    fn remove_closed_windows(&mut self) {
        let mut close_ids = Vec::new();
        for (id, runtime) in &mut self.windows {
            runtime.platform.apply_properties(&runtime.window);
            if runtime.window.state.snapshot() == WindowState::Closed {
                close_ids.push(*id);
            }
        }

        for id in close_ids {
            #[cfg(target_os = "windows")]
            if let Some(runtime) = self.windows.get(&id) {
                self.detach_menu_bar_hwnd(runtime);
            }
            self.windows.remove(&id);
            self.popup_window_ids.remove(&id);
            self.accesskit_adapters.remove(&id);
            self.last_accessibility_updates.remove(&id);
        }

        self.exit_if_last_window_closed();
    }

    /// A declared `MenuItem::CloseWindow` asks `source` to close — the
    /// window the command was invoked for, carried by the dispatch —
    /// through the window's ordinary close path, so closing the last one
    /// still follows the `LastWindowPolicy`.
    #[cfg(hydrolysis_closable_windows)]
    fn close_window(&mut self, source: crate::renderer::WindowId) {
        let Some(window_id) =
            close_window_target(source.as_winit(), &self.popup_window_ids, &self.windows)
        else {
            return;
        };
        let runtime = self
            .windows
            .get_mut(&window_id)
            .expect("close_window_target only names a window in `self.windows`");
        // The request lands in the window's own event queue, so it takes
        // the identical path the title-bar close button takes — its
        // `closable` check lives there, in the one close path.
        runtime.platform.request_close();
        if Self::handle_input_events(runtime, &self.env) {
            self.reap_closed_window(window_id);
        }
    }

    /// Removes a window whose close ran: its native bar attachment, adapter
    /// and cached tree go with it, and the last-window policy answers
    /// whether the app exits.
    fn reap_closed_window(&mut self, window_id: WindowId) {
        #[cfg(target_os = "windows")]
        if let Some(runtime) = self.windows.get(&window_id) {
            self.detach_menu_bar_hwnd(runtime);
        }
        self.windows.remove(&window_id);
        self.popup_window_ids.remove(&window_id);
        self.accesskit_adapters.remove(&window_id);
        self.last_accessibility_updates.remove(&window_id);
        self.exit_if_last_window_closed();
    }

    /// An accesskit adapter's request for one of the runner's windows: the
    /// initial tree, or an action an assistive technology performs.
    fn accesskit_event(&mut self, event: AccessKitEvent) {
        let Some(runtime) = self.windows.get_mut(&event.window_id) else {
            return;
        };
        let Some(adapter) = self.accesskit_adapters.get_mut(&event.window_id) else {
            return;
        };
        match event.window_event {
            AccessKitWindowEvent::InitialTreeRequested => {
                tracing::trace!(
                    target: "waterui::hydrolysis::a11y",
                    window_id = ?event.window_id,
                    "accesskit initial tree requested"
                );
                if let Some(update) = runtime.renderer.take_accessibility_tree_update() {
                    self.last_accessibility_updates
                        .insert(event.window_id, update.clone());
                    tracing::trace!(
                        target: "waterui::hydrolysis::a11y",
                        window_id = ?event.window_id,
                        "publishing accessibility tree update for initial request"
                    );
                    adapter.update_if_active(|| update);
                } else if let Some(update) = self
                    .last_accessibility_updates
                    .get(&event.window_id)
                    .cloned()
                {
                    tracing::trace!(
                        target: "waterui::hydrolysis::a11y",
                        window_id = ?event.window_id,
                        "replaying cached accessibility tree update for initial request"
                    );
                    adapter.update_if_active(|| update);
                } else {
                    tracing::trace!(
                        target: "waterui::hydrolysis::a11y",
                        window_id = ?event.window_id,
                        "missing accessibility tree update for initial request, scheduling rebuild"
                    );
                    runtime.request_refresh();
                    runtime.request_redraw();
                    runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
                }
            }
            AccessKitWindowEvent::ActionRequested(request) => {
                tracing::trace!(
                    target: "waterui::hydrolysis::a11y",
                    window_id = ?event.window_id,
                    action = ?request.action,
                    target = ?request.target_node,
                    "accesskit action requested"
                );
                let action_env = self.env.extending(Self::current_window_origin(runtime));
                if runtime
                    .renderer
                    .handle_accessibility_action(request, &action_env)
                {
                    runtime.request_refresh();
                    runtime.request_redraw();
                    runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
                }
                self.flush_cross_window_rebuild_requests();
            }
            AccessKitWindowEvent::AccessibilityDeactivated => {}
        }
    }

    fn flush_cross_window_rebuild_requests(&mut self) {
        for runtime in self.windows.values_mut() {
            if runtime.renderer.has_structure_marks() {
                runtime.request_refresh();
                runtime.request_redraw();
                runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
            }
        }
    }
}

impl ApplicationHandler<RunnerEvent> for WinitRunner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let _ = self.drain_local_executor_queue();
        self.mount_pending_windows(event_loop);
        for runtime in self.windows.values_mut() {
            // A resume can carry the window across a visibility boundary
            // (iOS foregrounding); the pump reflects what the platform
            // reports now.
            runtime.sync_occlusion();
            runtime.request_redraw();
            runtime.renderer.frame_work_counters_mut().host_wakeups += 1;
        }
        // A windowless launch under `Quit` is a required termination —
        // asked here, not before the loop runs, so the hook futures have a
        // local executor to run on (installed by `mount_pending_windows`).
        self.exit_if_last_window_closed();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let _ = self.drain_local_executor_queue();
        let is_pointer_press = match &event {
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => true,
            WindowEvent::Touch(touch) => touch.phase == TouchPhase::Started,
            _ => false,
        };
        if is_pointer_press {
            // A press that lands on one of our windows cancels out one raw
            // device-level press; what remains at the end of the batch is
            // presses outside every window we own.
            self.outside_pointer_presses = self.outside_pointer_presses.saturating_sub(1);
        }
        // Selector inputs that outlive a single event: `Focused` feeds the
        // `Focused` monitor selector, and the last pointer-touched window is
        // Wayland's only answer to "which monitor is the pointer on".
        match &event {
            WindowEvent::Focused(focused) => {
                self.focused_window = focused.then_some(window_id);
            }
            WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::Touch(_) => {
                self.last_pointer_window = Some(window_id);
            }
            _ => {}
        }
        let should_close = {
            let Some(runtime) = self.windows.get_mut(&window_id) else {
                return;
            };
            if let Some(adapter) = self.accesskit_adapters.get_mut(&window_id) {
                adapter.process_event(runtime.platform.native_window(), &event);
            }
            runtime.platform.handle_window_event(&event);
            // The platform's occlusion report moves into the pump state
            // here, so a `RedrawRequested` handled below and the next
            // `about_to_wait` tick both see the window as it is now.
            runtime.sync_occlusion();
            Self::handle_input_events(runtime, &self.env)
        };

        if !self.pending_window_queue.borrow().is_empty() || !self.pending_windows.is_empty() {
            self.mount_pending_windows(event_loop);
        }

        self.flush_cross_window_rebuild_requests();

        if should_close {
            self.reap_closed_window(window_id);
            return;
        }

        if event == WindowEvent::RedrawRequested {
            let Some(runtime) = self.windows.get_mut(&window_id) else {
                return;
            };
            render_window(runtime, &self.env, &mut || {
                Self::drain_runnable_queue(&self.local_runnable_rx)
            });
            if let Some(adapter) = self.accesskit_adapters.get_mut(&window_id) {
                if let Some(update) = runtime.renderer.take_accessibility_tree_update() {
                    self.last_accessibility_updates
                        .insert(window_id, update.clone());
                    tracing::trace!(
                        target: "waterui::hydrolysis::a11y",
                        window_id = ?window_id,
                        "publishing accessibility tree update on redraw"
                    );
                    adapter.update_if_active(|| update);
                } else {
                    tracing::trace!(
                        target: "waterui::hydrolysis::a11y",
                        window_id = ?window_id,
                        "no accessibility tree update available on redraw"
                    );
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let _ = self.drain_local_executor_queue();
        // Native menu-bar activations: each bar reports them as `CommandId`s
        // on its `events()` stream — drain here so each dispatches on the
        // event loop. On macOS a bar item's action acts on the focused
        // window; on Windows it acts on the bar's own window — Close
        // Window's request names it through the shared `WindowCloser` path.
        #[cfg(target_os = "macos")]
        self.native_menu_bar
            .pump_menu_events(self.focused_window.map(crate::renderer::WindowId::Winit));
        #[cfg(target_os = "windows")]
        self.native_menu_bar.pump_menu_events();
        self.mount_pending_windows(event_loop);
        let now = Instant::now();
        let mut next_gesture_deadline: Option<Instant> = None;
        for runtime in self.windows.values_mut() {
            if let Some(deadline) = advance_runtime(runtime, &self.env, now) {
                next_gesture_deadline =
                    Some(next_gesture_deadline.map_or(deadline, |existing| existing.min(deadline)));
            }
        }
        if let Some(deadline) = next_gesture_deadline {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
        if self.outside_pointer_presses > 0 {
            self.outside_pointer_presses = 0;
            // A raw device press left over after window-level presses are
            // accounted for landed outside every window we own — on X11 the
            // server only delivers pointer events for presses on our own
            // windows, so this is the only signal a press on the desktop or
            // another app ever reaches us. That outside press is what popup
            // windows dismiss on.
            for id in &self.popup_window_ids {
                if let Some(runtime) = self.windows.get(id) {
                    runtime.window.state.set(WindowState::Closed);
                }
            }
        }
        self.remove_closed_windows();
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::Button {
            state: ElementState::Pressed,
            ..
        } = event
        {
            self.outside_pointer_presses += 1;
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RunnerEvent) {
        match event {
            RunnerEvent::PollLocalTasks => {
                let _ = self.drain_local_executor_queue();
            }
            RunnerEvent::MountPendingWindows => {
                self.mount_pending_windows(event_loop);
            }
            RunnerEvent::RedrawWindow(window_id) => {
                // The window may have closed while the wake was in flight.
                // The wake asks winit for a frame only: GPU content's
                // cadence does not hold the window's ProMotion demand.
                if let Some(runtime) = self.windows.get(&window_id) {
                    runtime.platform.native_window().request_redraw();
                }
            }

            RunnerEvent::AccessKit(event) => self.accesskit_event(event),
            #[cfg(any(unix, windows))]
            RunnerEvent::Terminate => {
                self.termination.request(TerminationKind::Required);
            }
            #[cfg(target_os = "windows")]
            RunnerEvent::ConsoleClose(release) => {
                self.session_end.release_console_on_terminate(release);
                self.termination.request(TerminationKind::Required);
            }
            RunnerEvent::TerminationFinished => {
                self.exit_after_runtime_cleanup(event_loop);
            }
            #[cfg(hydrolysis_closable_windows)]
            RunnerEvent::CloseWindow(source) => self.close_window(source),
            #[cfg(hydrolysis_wayland_platform)]
            RunnerEvent::X11VisibilitySignal => {
                // `_NET_WM_STATE`/`WM_STATE` changed or a window
                // (un)mapped — a minimize or restore winit emitted no
                // `WindowEvent` for. The notification only wakes the
                // re-query: `is_minimized` reads `_NET_WM_STATE_HIDDEN`
                // again and the pump lands wherever the query puts it.
                for runtime in self.windows.values_mut() {
                    runtime.platform.refresh_visibility_signals();
                    runtime.sync_occlusion();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::{TerminationAction, TerminationRequests};
    use super::{ends_event_loop, native_window_attributes};
    use waterui::window::{Window, WindowState};
    use waterui_core::{Binding, binding};

    #[cfg(windows)]
    #[test]
    fn first_termination_signal_asks_the_loop_and_later_ones_do_not() {
        let requests = TerminationRequests::default();

        assert_eq!(requests.record(), TerminationAction::RequestExit);
        assert_eq!(requests.record(), TerminationAction::ForceExit);
        assert_eq!(requests.record(), TerminationAction::ForceExit);
    }

    #[test]
    fn quit_policy_ends_the_loop_once_no_window_is_left() {
        use waterui::app::LastWindowPolicy;

        // At startup, a quitting app that declares no window has nothing to run.
        assert!(ends_event_loop(LastWindowPolicy::Quit, 0));
        assert!(!ends_event_loop(LastWindowPolicy::Quit, 1));
        assert!(!ends_event_loop(LastWindowPolicy::Quit, 2));
    }

    #[test]
    fn stay_resident_policy_keeps_the_loop_with_no_window() {
        use waterui::app::LastWindowPolicy;

        assert!(!ends_event_loop(LastWindowPolicy::StayResident, 0));
        assert!(!ends_event_loop(LastWindowPolicy::StayResident, 1));
    }

    #[test]
    fn popup_window_attributes_do_not_activate() {
        let window = Window::new("", binding(WindowState::Normal), || ());
        let env = crate::renderer::tests::test_environment();

        assert!(native_window_attributes(&window, &env, true, None).active);
        assert!(!native_window_attributes(&window, &env, false, None).active);
    }

    #[cfg(hydrolysis_wayland_platform)]
    #[test]
    fn window_attributes_split_wm_class_from_instance_name() {
        use waterui_core::Str;

        let mut window = Window::new("", binding(WindowState::Normal), || ());
        window.app_id = Some(Str::from("myclass"));
        window.instance_name = Some(Str::from("myinstance"));
        assert_eq!(
            super::window_desktop_identity(&window),
            Some((Str::from("myclass"), Str::from("myinstance"))),
            "WM_CLASS carries (app_id, instance_name)"
        );

        let mut window = Window::new("", binding(WindowState::Normal), || ());
        window.app_id = Some(Str::from("myclass"));
        assert_eq!(
            super::window_desktop_identity(&window),
            Some((Str::from("myclass"), Str::from("myclass"))),
            "an undeclared instance_name resolves to the app_id"
        );
    }

    #[test]
    fn window_attributes_carry_the_requested_frame() {
        use waterui_core::layout::{Point, Rect, Size};
        let frame = Binding::container(Rect::new(Point::new(12.0, 34.0), Size::new(800.0, 300.0)));
        let mut window = Window::new("", binding(WindowState::Normal), || ());
        window.frame = frame;
        let env = crate::renderer::tests::test_environment();

        let attributes = native_window_attributes(&window, &env, false, None);
        assert_eq!(
            attributes.position,
            Some(winit::dpi::Position::Logical(
                winit::dpi::LogicalPosition::new(12.0, 34.0)
            )),
            "the requested origin must travel with the window attributes: a position set while unmapped is dropped on X11"
        );
        assert_eq!(
            attributes.inner_size,
            Some(winit::dpi::Size::Logical(winit::dpi::LogicalSize::new(
                800.0, 300.0
            ))),
        );
    }

    #[test]
    fn window_attributes_disable_only_the_close_button_on_a_non_closable_window() {
        use winit::window::WindowButtons;

        let mut window = Window::new("", binding(WindowState::Normal), || ());
        window.closable = false;
        let env = crate::renderer::tests::test_environment();

        assert_eq!(
            native_window_attributes(&window, &env, false, None).enabled_buttons,
            WindowButtons::MINIMIZE | WindowButtons::MAXIMIZE,
            "a non-closable window drops only the close button"
        );

        let window = Window::new("", binding(WindowState::Normal), || ());
        assert_eq!(
            native_window_attributes(&window, &env, false, None).enabled_buttons,
            WindowButtons::all(),
            "a closable window keeps every title-bar button"
        );
    }

    /// The window a Close Window request acts on is the window the
    /// dispatch invoked it for, and a popup is never the target — its
    /// owner is, which the bare source cannot name.
    #[cfg(hydrolysis_closable_windows)]
    #[test]
    fn a_close_request_targets_the_dispatching_window_never_a_popup() {
        let windows = std::collections::HashMap::from([(1_u32, "window"), (2, "popup")]);
        let popups = std::collections::HashSet::from([2_u32]);
        let pick = |source| super::close_window_target(source, &popups, &windows);

        assert_eq!(
            pick(Some(1)),
            Some(1),
            "the dispatching window's id is the target"
        );
        assert_eq!(pick(Some(2)), None, "a popup is never the target");
    }
}
