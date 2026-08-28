use std::any::Any;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use chromiumoxide_types::{Command, EventMessage, MethodType};
use futures::Stream;
use serde::de::DeserializeOwned;
use serde_json::Value;
use waterui_core::impl_debug;

/// A raw CDP event emitted by Chromium.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdpEvent {
    /// Fully qualified protocol method, such as `Page.loadEventFired`.
    pub method: Cow<'static, str>,
    /// Event parameter object.
    pub params: Value,
}

/// Errors returned by CDP commands and event decoding.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum CdpError {
    /// Chromium rejected a protocol command.
    #[error("CDP command `{method}` failed ({code}): {message}")]
    Protocol {
        /// Fully qualified protocol method.
        method: String,
        /// Chromium protocol error code.
        code: i64,
        /// Chromium protocol error text.
        message: String,
    },
    /// A command or event payload did not match the generated protocol schema.
    #[error("CDP payload for `{method}` is invalid: {message}")]
    InvalidPayload {
        /// Fully qualified protocol method.
        method: String,
        /// Serde error text.
        message: String,
    },
    /// The page closed while an operation was pending.
    #[error("Chromium page closed while CDP operation `{method}` was pending")]
    PageClosed {
        /// Fully qualified protocol method.
        method: String,
    },
    /// The CEF `DevTools` agent detached.
    #[error("Chromium DevTools agent detached")]
    Detached,
}

/// Backend CDP transport implemented by the Chromium engine.
///
/// This public trait keeps generic, allocation-free method signatures. Dynamic
/// dispatch is isolated in [`CdpSession`].
pub trait CustomCdpSession: Clone + 'static {
    /// Executes a raw CDP method.
    fn execute_raw(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, CdpError>> + 'static;

    /// Subscribes to every event with the given method identifier.
    fn subscribe(&self, method: &str) -> async_channel::Receiver<CdpEvent>;

    /// Closes the `DevTools` session after all accepted commands complete.
    fn close(&self) -> impl Future<Output = Result<(), CdpError>> + 'static;
}

trait CdpSessionImpl: Any {
    fn execute_raw(
        &self,
        method: String,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CdpError>>>>;
    fn subscribe(&self, method: &str) -> async_channel::Receiver<CdpEvent>;
    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), CdpError>>>>;
}

impl<T: CustomCdpSession> CdpSessionImpl for T {
    fn execute_raw(
        &self,
        method: String,
        params: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CdpError>>>> {
        Box::pin(CustomCdpSession::execute_raw(self, &method, params))
    }

    fn subscribe(&self, method: &str) -> async_channel::Receiver<CdpEvent> {
        CustomCdpSession::subscribe(self, method)
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), CdpError>>>> {
        Box::pin(CustomCdpSession::close(self))
    }
}

/// A cloneable CDP session shared by visible and headless Chromium pages.
#[derive(Clone)]
pub struct CdpSession {
    inner: Rc<dyn CdpSessionImpl>,
}

impl_debug!(CdpSession);

impl CdpSession {
    /// Erases a backend CDP session behind the public client.
    #[must_use]
    pub fn new(session: impl CustomCdpSession) -> Self {
        Self {
            inner: Rc::new(session),
        }
    }

    /// Executes a raw JSON CDP method.
    #[expect(
        clippy::future_not_send,
        reason = "CEF DevTools sessions are bound to the browser UI thread"
    )]
    pub fn execute_raw(
        &self,
        method: impl Into<String>,
        params: Value,
    ) -> impl Future<Output = Result<Value, CdpError>> + 'static {
        self.inner.execute_raw(method.into(), params)
    }

    /// Executes a generated, strongly typed CDP command.
    ///
    /// # Errors
    ///
    /// Returns a protocol error or a typed payload decoding error.
    #[expect(
        clippy::future_not_send,
        reason = "CEF DevTools sessions are bound to the browser UI thread"
    )]
    pub async fn execute<C>(&self, command: C) -> Result<C::Response, CdpError>
    where
        C: Command + 'static,
    {
        let method = command.identifier().into_owned();
        let params = serde_json::to_value(command).map_err(|error| CdpError::InvalidPayload {
            method: method.clone(),
            message: error.to_string(),
        })?;
        let response = self.execute_raw(method.clone(), params).await?;
        C::response_from_value(response).map_err(|error| CdpError::InvalidPayload {
            method,
            message: error.to_string(),
        })
    }

    /// Subscribes to raw events with the exact method identifier.
    #[must_use]
    pub fn events_raw(&self, method: &str) -> async_channel::Receiver<CdpEvent> {
        self.inner.subscribe(method)
    }

    /// Subscribes to one generated CDP event type.
    pub fn events<E>(&self) -> impl Stream<Item = Result<E, CdpError>> + use<E>
    where
        E: EventMessage + MethodType + DeserializeOwned + 'static,
    {
        let method = E::method_id().into_owned();
        let receiver = self.inner.subscribe(&method);
        futures::stream::unfold((receiver, method), |(receiver, method)| async move {
            let event = receiver.recv().await.ok()?;
            let decoded =
                serde_json::from_value(event.params).map_err(|error| CdpError::InvalidPayload {
                    method: method.clone(),
                    message: error.to_string(),
                });
            Some((decoded, (receiver, method)))
        })
    }

    /// Closes this `DevTools` session after accepted commands complete.
    #[expect(
        clippy::future_not_send,
        reason = "CEF DevTools sessions are bound to the browser UI thread"
    )]
    pub fn close(&self) -> impl Future<Output = Result<(), CdpError>> + 'static {
        self.inner.close()
    }

    /// Downcasts to the concrete backend session.
    #[must_use]
    pub fn downcast_ref<T: CustomCdpSession>(&self) -> Option<&T> {
        (self.inner.as_ref() as &dyn Any).downcast_ref::<T>()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::future::ready;
    use std::rc::Rc;

    use chromiumoxide_cdp::cdp::browser_protocol::page::GetNavigationHistoryParams;
    use serde_json::{Value, json};

    use super::{CdpError, CdpEvent, CdpSession, CustomCdpSession};

    #[derive(Clone, Default)]
    struct TestSession {
        calls: Rc<RefCell<Vec<(String, Value)>>>,
        subscribers: Rc<RefCell<HashMap<String, Vec<async_channel::Sender<CdpEvent>>>>>,
    }

    impl TestSession {
        fn emit(&self, event: &CdpEvent) {
            if let Some(subscribers) = self.subscribers.borrow().get(event.method.as_ref()) {
                for subscriber in subscribers {
                    subscriber
                        .try_send(event.clone())
                        .expect("test CDP subscriber must remain open");
                }
            }
        }
    }

    impl CustomCdpSession for TestSession {
        fn execute_raw(
            &self,
            method: &str,
            params: Value,
        ) -> impl Future<Output = Result<Value, CdpError>> + 'static {
            self.calls.borrow_mut().push((method.to_string(), params));
            ready(Ok(json!({
                "currentIndex": 0,
                "entries": []
            })))
        }

        fn subscribe(&self, method: &str) -> async_channel::Receiver<CdpEvent> {
            let (sender, receiver) = async_channel::unbounded();
            self.subscribers
                .borrow_mut()
                .entry(method.to_string())
                .or_default()
                .push(sender);
            receiver
        }

        fn close(&self) -> impl Future<Output = Result<(), CdpError>> + 'static {
            ready(Ok(()))
        }
    }

    #[test]
    fn raw_and_generated_commands_share_one_transport() {
        let transport = TestSession::default();
        let session = CdpSession::new(transport.clone());

        let raw = futures::executor::block_on(
            session.execute_raw("Page.getNavigationHistory", json!({})),
        )
        .expect("raw CDP command should succeed");
        assert_eq!(raw["currentIndex"], 0);

        let typed =
            futures::executor::block_on(session.execute(GetNavigationHistoryParams::default()))
                .expect("typed CDP command should decode");
        assert_eq!(typed.current_index, 0);
        assert!(typed.entries.is_empty());

        let calls = transport.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(
            calls
                .iter()
                .all(|(method, _)| method == "Page.getNavigationHistory")
        );
    }

    #[test]
    fn raw_event_subscriptions_are_method_scoped() {
        let transport = TestSession::default();
        let session = CdpSession::new(transport.clone());
        let receiver = session.events_raw("Page.loadEventFired");

        transport.emit(&CdpEvent {
            method: "Page.loadEventFired".into(),
            params: json!({"timestamp": 1.5}),
        });

        let event = receiver
            .try_recv()
            .expect("matching event should be delivered");
        assert_eq!(event.params["timestamp"], 1.5);
    }
}
