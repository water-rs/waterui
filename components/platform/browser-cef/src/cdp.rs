use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use cef::rc::Rc as _;
use cef::{
    Browser, BrowserHost, DevToolsMessageObserver, ImplBrowserHost, ImplDevToolsMessageObserver,
    Registration, WrapDevToolsMessageObserver,
};
use futures::channel::oneshot;
use serde_json::{Value, json};

struct PendingCommand {
    method: String,
    response: oneshot::Sender<Result<Value, CefCdpError>>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum CefCdpError {
    #[error("CDP command `{method}` failed ({code}): {message}")]
    Protocol {
        method: String,
        code: i64,
        message: String,
    },
    #[error("CEF page closed while CDP operation `{method}` was pending")]
    PageClosed { method: String },
    #[error("CEF DevTools agent detached")]
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CefCdpEvent {
    pub method: String,
    pub params: Value,
}

type EventWatcher = Rc<dyn Fn(&CefCdpEvent)>;
#[cfg(feature = "chromium")]
type AttachmentWatcher = Box<dyn Fn()>;

#[derive(Default)]
struct CdpState {
    next_message_id: Cell<i32>,
    pending: RefCell<HashMap<i32, PendingCommand>>,
    event_watchers: RefCell<Vec<EventWatcher>>,
    attachment_waiters: RefCell<Vec<oneshot::Sender<Result<(), CefCdpError>>>>,
    #[cfg(feature = "chromium")]
    attachment_watchers: RefCell<Vec<AttachmentWatcher>>,
    registration: RefCell<Option<Registration>>,
    attached: Cell<bool>,
    closed: Cell<bool>,
}

impl CdpState {
    fn next_message_id(&self) -> i32 {
        let next = self
            .next_message_id
            .get()
            .checked_add(1)
            .expect("CEF CDP message identifier overflowed");
        self.next_message_id.set(next);
        next
    }

    fn handle_message(&self, message: &[u8]) {
        let message: Value = match serde_json::from_slice(message) {
            Ok(message) => message,
            Err(error) => {
                tracing::error!(%error, "CEF emitted malformed CDP JSON; ignoring the message");
                return;
            }
        };
        if let Some(id) = message.get("id").and_then(Value::as_i64) {
            let Ok(id) = i32::try_from(id) else {
                tracing::error!(id, "CEF emitted an out-of-range CDP message id; ignoring");
                return;
            };
            let Some(command) = self.pending.borrow_mut().remove(&id) else {
                return;
            };
            let result = if let Some(error) = message.get("error") {
                Err(CefCdpError::Protocol {
                    method: command.method.clone(),
                    code: error
                        .get("code")
                        .and_then(Value::as_i64)
                        .unwrap_or_default(),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Chromium returned an unspecified CDP error")
                        .to_string(),
                })
            } else {
                Ok(message.get("result").cloned().unwrap_or(Value::Null))
            };
            // A dropped receiver means the command was issued fire-and-forget, which is a
            // supported pattern. A *failure* nobody awaited would otherwise vanish
            // silently, so report it here instead of at every call site.
            if let Err(Err(error)) = command.response.send(result) {
                tracing::error!(
                    method = %command.method,
                    %error,
                    "CDP command failed and its result was never awaited"
                );
            }
            return;
        }

        let Some(method) = message.get("method").and_then(Value::as_str) else {
            tracing::error!("CEF emitted a CDP message with neither `id` nor `method`; ignoring");
            return;
        };
        let event = CefCdpEvent {
            method: method.to_string(),
            params: message.get("params").cloned().unwrap_or(Value::Null),
        };
        for watcher in self.event_watchers.borrow().iter() {
            watcher(&event);
        }
    }

    fn attached(&self) {
        self.attached.set(true);
        for waiter in self.attachment_waiters.borrow_mut().drain(..) {
            let _ = waiter.send(Ok(()));
        }
        #[cfg(feature = "chromium")]
        for watcher in self.attachment_watchers.borrow_mut().drain(..) {
            watcher();
        }
    }

    fn detached(&self) {
        self.attached.set(false);
        self.registration.borrow_mut().take();
        for (_, pending) in self.pending.borrow_mut().drain() {
            let _ = pending.response.send(Err(CefCdpError::Detached));
        }
        for waiter in self.attachment_waiters.borrow_mut().drain(..) {
            let _ = waiter.send(Err(CefCdpError::Detached));
        }
    }

    #[cfg(feature = "chromium")]
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.detached();
    }
}

#[allow(
    clippy::transmute_ptr_to_ptr,
    reason = "CEF wrapper macros generate ABI pointer casts outside WaterUI's control"
)]
fn new_dev_tools_observer(state: Rc<CdpState>) -> DevToolsMessageObserver {
    cef::wrap_dev_tools_message_observer! {
        struct WaterDevToolsObserver {
            state: Rc<CdpState>,
        }

        impl DevToolsMessageObserver {
            fn on_dev_tools_message(
                &self,
                _browser: Option<&mut Browser>,
                message: Option<&[u8]>,
            ) -> std::os::raw::c_int {
                let message = message.expect("CEF DevTools callback must contain a message");
                self.state.handle_message(message);
                1
            }

            fn on_dev_tools_agent_attached(&self, _browser: Option<&mut Browser>) {
                self.state.attached();
            }

            fn on_dev_tools_agent_detached(&self, _browser: Option<&mut Browser>) {
                self.state.detached();
            }
        }
    }

    WaterDevToolsObserver::new(state)
}

/// In-process CDP transport owned by one CEF page.
#[derive(Clone)]
pub struct CefCdpSession {
    host: BrowserHost,
    state: Rc<CdpState>,
}

impl core::fmt::Debug for CefCdpSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CefCdpSession")
            .field("attached", &self.state.attached.get())
            .field("closed", &self.state.closed.get())
            .finish_non_exhaustive()
    }
}

impl CefCdpSession {
    pub(crate) fn attach(host: BrowserHost) -> Self {
        let state = Rc::new(CdpState::default());
        let mut observer = new_dev_tools_observer(Rc::clone(&state));
        let registration = host
            .add_dev_tools_message_observer(Some(&mut observer))
            .expect("CEF refused to register the in-process DevTools observer");
        state.registration.replace(Some(registration));
        let session = Self { host, state };

        // CEF attaches the DevTools agent in response to the first protocol
        // message. Probe it immediately so page readiness never depends on a
        // later user command.
        drop(session.execute_raw("Browser.getVersion", &json!({})));

        session
    }

    #[cfg(feature = "chromium")]
    #[expect(
        clippy::future_not_send,
        reason = "CEF DevTools sessions are confined to the UI thread"
    )]
    pub(crate) async fn wait_until_attached(&self) -> Result<(), CefCdpError> {
        if self.state.closed.get() {
            return Err(CefCdpError::Detached);
        }
        if self.state.attached.get() {
            return Ok(());
        }
        let (sender, receiver) = oneshot::channel();
        self.state.attachment_waiters.borrow_mut().push(sender);
        receiver.await.unwrap_or(Err(CefCdpError::Detached))
    }

    pub(crate) fn watch_events(&self, watcher: impl Fn(&CefCdpEvent) + 'static) {
        self.state
            .event_watchers
            .borrow_mut()
            .push(Rc::new(watcher));
    }

    #[cfg(feature = "chromium")]
    pub(crate) fn watch_attachment(&self, watcher: impl Fn() + 'static) {
        if self.state.attached.get() {
            watcher();
        } else {
            self.state
                .attachment_watchers
                .borrow_mut()
                .push(Box::new(watcher));
        }
    }

    pub(crate) fn execute_raw(
        &self,
        method: &str,
        params: &Value,
    ) -> impl Future<Output = Result<Value, CefCdpError>> + 'static + use<> {
        let method = method.to_string();
        let (sender, receiver) = oneshot::channel();
        if self.state.closed.get() {
            let _ = sender.send(Err(CefCdpError::PageClosed {
                method: method.clone(),
            }));
        } else {
            let id = self.state.next_message_id();
            let message = serde_json::to_vec(&json!({
                "id": id,
                "method": method,
                "params": params,
            }))
            .expect("CDP request values must serialize");
            self.state.pending.borrow_mut().insert(
                id,
                PendingCommand {
                    method: method.clone(),
                    response: sender,
                },
            );
            if self.host.send_dev_tools_message(Some(&message)) != 1 {
                let pending = self
                    .state
                    .pending
                    .borrow_mut()
                    .remove(&id)
                    .expect("CDP pending command must exist after a send failure");
                let _ = pending.response.send(Err(CefCdpError::PageClosed {
                    method: pending.method,
                }));
            }
        }
        async move {
            receiver
                .await
                .unwrap_or(Err(CefCdpError::PageClosed { method }))
        }
    }

    #[cfg(feature = "chromium")]
    pub(crate) fn close(&self) -> impl Future<Output = Result<(), CefCdpError>> + 'static {
        self.state.close();
        core::future::ready(Ok(()))
    }
}

#[cfg(feature = "chromium")]
impl From<CefCdpError> for waterui_chromium::CdpError {
    fn from(error: CefCdpError) -> Self {
        match error {
            CefCdpError::Protocol {
                method,
                code,
                message,
            } => Self::Protocol {
                method,
                code,
                message,
            },
            CefCdpError::PageClosed { method } => Self::PageClosed { method },
            CefCdpError::Detached => Self::Detached,
        }
    }
}

#[cfg(feature = "chromium")]
impl waterui_chromium::CustomCdpSession for CefCdpSession {
    fn execute_raw(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, waterui_chromium::CdpError>> + 'static {
        let future = Self::execute_raw(self, method, &params);
        async move { future.await.map_err(Into::into) }
    }

    fn subscribe(&self, method: &str) -> async_channel::Receiver<waterui_chromium::CdpEvent> {
        assert!(
            !self.state.closed.get(),
            "cannot subscribe to a closed CEF CDP session"
        );
        let method = method.to_string();
        let (sender, receiver) = async_channel::unbounded();
        self.watch_events(move |event| {
            if event.method == method {
                let _ = sender.try_send(waterui_chromium::CdpEvent {
                    method: event.method.clone().into(),
                    params: event.params.clone(),
                });
            }
        });
        receiver
    }

    fn close(&self) -> impl Future<Output = Result<(), waterui_chromium::CdpError>> + 'static {
        let future = Self::close(self);
        async move { future.await.map_err(Into::into) }
    }
}
