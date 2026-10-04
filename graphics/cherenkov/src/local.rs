//! One serial executor on the owning browser thread. No JS-owned value crosses
//! a thread boundary. Enqueuing does not borrow the renderer, including while
//! an operation awaits the browser; drops and signal notifications stay legal.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::mpsc::{RecvError, SendError};
use std::task::{Context, Poll, Waker};

type Task = Pin<Box<dyn Future<Output = bool>>>;
type Handler<T> = Box<dyn Fn(T) -> Task>;

struct Queue<T> {
    messages: VecDeque<T>,
    handler: Option<Handler<T>>,
    running: bool,
}

/// Sender to the serial executor on this JS thread.
pub struct Sender<T>(Rc<RefCell<Queue<T>>>);

/// The queued message type need not format.
impl<T> std::fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sender").finish_non_exhaustive()
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: 'static> Sender<T> {
    pub(crate) fn new(handler: impl Fn(T) -> Task + 'static) -> Self {
        Self(Rc::new(RefCell::new(Queue {
            messages: VecDeque::new(),
            handler: Some(Box::new(handler)),
            running: false,
        })))
    }

    pub(crate) fn send(&self, message: T) -> Result<(), SendError<T>> {
        {
            let mut queue = self.0.borrow_mut();
            if queue.handler.is_none() {
                return Err(SendError(message));
            }
            queue.messages.push_back(message);
            if queue.running {
                return Ok(());
            }
            queue.running = true;
        }
        let shared = Rc::clone(&self.0);
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let task = {
                    let mut queue = shared.borrow_mut();
                    let Some(message) = queue.messages.pop_front() else {
                        queue.running = false;
                        return;
                    };
                    queue.handler.as_ref().expect("live executor")(message)
                };
                if !task.await {
                    // Release the renderer here, even when resource/surface
                    // handles outlive Engine. Later sends fail like native.
                    let (handler, messages) = {
                        let mut queue = shared.borrow_mut();
                        queue.running = false;
                        (queue.handler.take(), std::mem::take(&mut queue.messages))
                    };
                    drop(messages);
                    drop(handler);
                    return;
                }
            }
        });
        Ok(())
    }
}

struct Reply<T> {
    value: Option<T>,
    closed: bool,
    waker: Option<Waker>,
}

/// A single reply from the local render executor.
pub struct ReplySender<T>(Rc<RefCell<Reply<T>>>);

/// The in-flight reply type need not format.
impl<T> std::fmt::Debug for ReplySender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplySender").finish_non_exhaustive()
    }
}

/// Receives the reply a [`ReplySender`] produces.
pub struct Receiver<T>(Rc<RefCell<Reply<T>>>);

/// The in-flight reply type need not format.
impl<T> std::fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Receiver").finish_non_exhaustive()
    }
}

pub fn channel<T>() -> (ReplySender<T>, Receiver<T>) {
    let shared = Rc::new(RefCell::new(Reply {
        value: None,
        closed: false,
        waker: None,
    }));
    (ReplySender(Rc::clone(&shared)), Receiver(shared))
}

impl<T> ReplySender<T> {
    pub(crate) fn send(self, value: T) -> Result<(), SendError<T>> {
        let waker = {
            let mut reply = self.0.borrow_mut();
            if reply.closed {
                return Err(SendError(value));
            }
            reply.value = Some(value);
            reply.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }
}

impl<T> Drop for ReplySender<T> {
    fn drop(&mut self) {
        let waker = {
            let mut reply = self.0.borrow_mut();
            reply.closed = true;
            reply.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> Receiver<T> {
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub(crate) async fn recv(self) -> Result<T, RecvError> {
        self.await
    }
}

impl<T> Future for Receiver<T> {
    type Output = Result<T, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut reply = self.0.borrow_mut();
        if let Some(value) = reply.value.take() {
            Poll::Ready(Ok(value))
        } else if reply.closed {
            Poll::Ready(Err(RecvError))
        } else {
            reply.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.0.borrow_mut().closed = true;
    }
}
