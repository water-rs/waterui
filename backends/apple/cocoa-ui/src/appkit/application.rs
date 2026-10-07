//! The application object and the events of its life.
//!
//! # Safety
//!
//! The `unsafe` here defines the application delegate class and initializes
//! it. The delegate's methods have the signatures `NSApplicationDelegate`
//! declares, `AppKit` sends them on the main thread, and the delegate stays
//! alive for as long as the application runs with it, because
//! [`Application::run`] owns it for that long.

use std::cell::Cell;
use std::fmt;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationTerminateReply, NSRequestUserAttentionType,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol};

use super::menu::Menu;
use crate::callback::guarded;

/// How the application presents itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivationPolicy {
    /// An ordinary application, with a Dock icon, a menu bar and windows.
    Regular,
    /// No Dock icon and no menu bar of its own, but it can show windows and
    /// be activated by clicking one.
    Accessory,
    /// Never activated and never shown: a process that runs headless.
    Prohibited,
}

impl ActivationPolicy {
    const fn native(self) -> NSApplicationActivationPolicy {
        match self {
            Self::Regular => NSApplicationActivationPolicy::Regular,
            Self::Accessory => NSApplicationActivationPolicy::Accessory,
            Self::Prohibited => NSApplicationActivationPolicy::Prohibited,
        }
    }

    /// The policy a native `NSApplicationActivationPolicy` reads —
    /// [`Self::native`]'s inverse.
    fn from_native(policy: NSApplicationActivationPolicy) -> Self {
        if policy == NSApplicationActivationPolicy::Regular {
            Self::Regular
        } else if policy == NSApplicationActivationPolicy::Accessory {
            Self::Accessory
        } else {
            // `Prohibited`, and anything the platform adds later — an
            // unlisted policy is at least as hidden.
            Self::Prohibited
        }
    }
}

/// How urgently an attention request presents itself —
/// `NSRequestUserAttentionType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttentionRequest {
    /// A single bounce of the Dock icon — `NSInformationalRequest`.
    Informational,
    /// The Dock icon bounces until the request is cancelled or the
    /// application is focused — `NSCriticalRequest`.
    Critical,
}

impl AttentionRequest {
    const fn native(self) -> NSRequestUserAttentionType {
        match self {
            Self::Informational => NSRequestUserAttentionType::InformationalRequest,
            Self::Critical => NSRequestUserAttentionType::CriticalRequest,
        }
    }
}

/// An outstanding attention request — what
/// [`Application::request_user_attention`] hands back and
/// [`Application::cancel_user_attention_request`] takes to stop the
/// request early.
#[derive(Debug)]
#[must_use = "an uncancelled request bounces until the application is activated"]
pub struct AttentionRequestToken(isize);

/// How the application answers a request to quit —
/// `NSApplicationTerminateReply`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminateReply {
    /// Quit now: `applicationWillTerminate:` follows and the process exits.
    Now,
    /// Keep running; a logout, restart or shutdown that asked is cancelled.
    Cancel,
    /// Decide later: the application keeps processing events, in the modal
    /// panel run-loop mode, until [`Application::reply_to_should_terminate`]
    /// answers.
    Later,
}

impl TerminateReply {
    const fn native(self) -> NSApplicationTerminateReply {
        match self {
            Self::Now => NSApplicationTerminateReply::TerminateNow,
            Self::Cancel => NSApplicationTerminateReply::TerminateCancel,
            Self::Later => NSApplicationTerminateReply::TerminateLater,
        }
    }
}

type OnceHandler = Box<dyn FnOnce(MainThreadMarker)>;
type QueryHandler = Box<dyn Fn(MainThreadMarker) -> bool>;
type TerminateHandler = Box<dyn Fn(MainThreadMarker) -> TerminateReply>;

/// What the application does at each point of its life, given to
/// [`Application::run`].
///
/// Every handler is optional; an event without one gets `AppKit`'s default
/// behaviour.
#[derive(Default)]
#[must_use = "handlers do nothing until they are passed to `Application::run`"]
pub struct ApplicationHandlers {
    did_finish_launching: Option<OnceHandler>,
    should_terminate: Option<TerminateHandler>,
    should_terminate_after_last_window_closed: Option<QueryHandler>,
    will_terminate: Option<OnceHandler>,
}

impl ApplicationHandlers {
    /// Handlers for no event.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `handler` once the application has launched and is about to
    /// handle its first event: the place to create the first windows.
    pub fn did_finish_launching(
        mut self,
        handler: impl FnOnce(MainThreadMarker) + 'static,
    ) -> Self {
        self.did_finish_launching = Some(Box::new(handler));
        self
    }

    /// Asks `handler` whether the application may quit, whenever something
    /// sends it `terminate:` — the Quit menu item and its ⌘Q, Quit in the
    /// Dock menu, a quit Apple event, a logout, restart or shutdown, and
    /// [`Application::terminate`]. Without a handler it quits at once.
    pub fn should_terminate(
        mut self,
        handler: impl Fn(MainThreadMarker) -> TerminateReply + 'static,
    ) -> Self {
        self.should_terminate = Some(Box::new(handler));
        self
    }

    /// Asks `handler`, when the user closes the last window, whether the
    /// application should quit. Without a handler it keeps running.
    pub fn should_terminate_after_last_window_closed(
        mut self,
        handler: impl Fn(MainThreadMarker) -> bool + 'static,
    ) -> Self {
        self.should_terminate_after_last_window_closed = Some(Box::new(handler));
        self
    }

    /// Runs `handler` once, just before the application quits.
    pub fn will_terminate(mut self, handler: impl FnOnce(MainThreadMarker) + 'static) -> Self {
        self.will_terminate = Some(Box::new(handler));
        self
    }
}

impl fmt::Debug for ApplicationHandlers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApplicationHandlers")
            .field("did_finish_launching", &self.did_finish_launching.is_some())
            .field("should_terminate", &self.should_terminate.is_some())
            .field(
                "should_terminate_after_last_window_closed",
                &self.should_terminate_after_last_window_closed.is_some(),
            )
            .field("will_terminate", &self.will_terminate.is_some())
            .finish()
    }
}

/// The process's application object.
#[derive(Debug, Clone)]
pub struct Application {
    app: Retained<NSApplication>,
}

impl Application {
    /// The application object, created on first use.
    #[must_use]
    pub fn shared(mtm: MainThreadMarker) -> Self {
        Self {
            app: NSApplication::sharedApplication(mtm),
        }
    }

    /// The policy the process currently holds — `activationPolicy`.
    #[must_use]
    pub fn activation_policy(&self) -> ActivationPolicy {
        ActivationPolicy::from_native(self.app.activationPolicy())
    }

    /// Sets how the application presents itself.
    ///
    /// Returns whether `AppKit` accepted the change. The system can refuse
    /// during early startup even for a well-formed app, so callers treat it
    /// as best-effort — the bundle's `Info.plist` policy still applies.
    #[must_use]
    pub fn set_activation_policy(&self, policy: ActivationPolicy) -> bool {
        self.app.setActivationPolicy(policy.native())
    }

    /// Makes `menu` the menu bar: each of its items is one top-level menu,
    /// and the first is the application menu.
    pub fn set_main_menu(&self, menu: &Menu) {
        self.app.setMainMenu(Some(menu.native()));
    }

    /// Makes `menu` the Services menu, which the system fills with the
    /// services that apply to the current selection.
    pub fn set_services_menu(&self, menu: &Menu) {
        self.app.setServicesMenu(Some(menu.native()));
    }

    /// Makes `menu` the Window menu, which the system extends with an item
    /// for every open window.
    pub fn set_windows_menu(&self, menu: &Menu) {
        self.app.setWindowsMenu(Some(menu.native()));
    }

    /// Asks for the user's attention at `kind`'s urgency — the Dock-icon
    /// bounce `NSApplication.requestUserAttention` produces — and answers
    /// the request's token, which [`Self::cancel_user_attention_request`]
    /// takes to stop it early.
    pub fn request_user_attention(&self, kind: AttentionRequest) -> AttentionRequestToken {
        AttentionRequestToken(self.app.requestUserAttention(kind.native()))
    }

    /// Cancels an outstanding attention request —
    /// `NSApplication.cancelUserAttentionRequest`, which also stops the Dock
    /// bounce.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "consuming the token is the contract: it names one request and cannot be cancelled twice"
    )]
    pub fn cancel_user_attention_request(&self, request: AttentionRequestToken) {
        self.app.cancelUserAttentionRequest(request.0);
    }

    /// Terminates the application: the equivalent of Quit — the delegate's
    /// `should_terminate` is asked, and once it agrees `will_terminate` runs
    /// and the process exits.
    pub fn terminate(&self) {
        self.app.terminate(None);
    }

    /// Answers a quit the delegate's `should_terminate` deferred with
    /// [`TerminateReply::Later`]: `true` quits, `false` keeps the
    /// application running and cancels a logout, restart or shutdown that
    /// asked.
    pub fn reply_to_should_terminate(&self, terminate: bool) {
        self.app.replyToApplicationShouldTerminate(terminate);
    }

    /// Runs the application's event loop, calling `handlers` as its events
    /// occur, until the application stops.
    ///
    /// An application that quits exits the process from inside this call;
    /// it returns only when the loop is stopped without quitting.
    pub fn run(&self, handlers: ApplicationHandlers) {
        let delegate = Delegate::new(self.app.mtm(), handlers);
        self.app
            .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        self.app.run();
        self.app.setDelegate(None);
    }

    pub(super) fn native(&self) -> &NSApplication {
        &self.app
    }
}

struct DelegateIvars {
    did_finish_launching: Cell<Option<OnceHandler>>,
    should_terminate: Option<TerminateHandler>,
    should_terminate_after_last_window_closed: Option<QueryHandler>,
    will_terminate: Cell<Option<OnceHandler>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class does
    // not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiApplicationDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct Delegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: see the module safety note.
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            guarded("applicationDidFinishLaunching:", || {
                if let Some(handler) = self.ivars().did_finish_launching.take() {
                    handler(self.mtm());
                }
            });
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn application_should_terminate(
            &self,
            _sender: &NSApplication,
        ) -> NSApplicationTerminateReply {
            guarded("applicationShouldTerminate:", || {
                self.ivars()
                    .should_terminate
                    .as_ref()
                    .map_or(TerminateReply::Now, |handler| handler(self.mtm()))
                    .native()
            })
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn application_should_terminate_after_last_window_closed(
            &self,
            _sender: &NSApplication,
        ) -> bool {
            guarded("applicationShouldTerminateAfterLastWindowClosed:", || {
                self.ivars()
                    .should_terminate_after_last_window_closed
                    .as_ref()
                    .is_some_and(|handler| handler(self.mtm()))
            })
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn application_will_terminate(&self, _notification: &NSNotification) {
            guarded("applicationWillTerminate:", || {
                if let Some(handler) = self.ivars().will_terminate.take() {
                    handler(self.mtm());
                }
            });
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, handlers: ApplicationHandlers) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars {
            did_finish_launching: Cell::new(handlers.did_finish_launching),
            should_terminate: handlers.should_terminate,
            should_terminate_after_last_window_closed: handlers
                .should_terminate_after_last_window_closed,
            will_terminate: Cell::new(handlers.will_terminate),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}
