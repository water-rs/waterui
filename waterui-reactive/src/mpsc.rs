use core::cell::RefCell;

use alloc::{collections::VecDeque, rc::Rc};

use crate::{subscriber::SubscriberManager, Reactive};

struct Buf<T> {
    buf: RefCell<VecDeque<T>>,
    subscribers: SubscriberManager,
}

impl<T> Default for Buf<T> {
    fn default() -> Self {
        Self {
            buf: RefCell::default(),
            subscribers: SubscriberManager::default(),
        }
    }
}

pub struct Sender<T>(Rc<Buf<T>>);

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct Receiver<T>(Rc<Buf<T>>);

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Option<T> {
        self.0.buf.borrow_mut().pop_front()
    }
}

impl<T> Reactive for Receiver<T> {
    fn register_subscriber(
        &self,
        subscriber: crate::subscriber::BoxSubscriber,
    ) -> Option<core::num::NonZeroUsize> {
        Some(self.0.subscribers.register(subscriber))
    }

    fn cancel_subscriber(&self, id: core::num::NonZeroUsize) {
        self.0.subscribers.cancel(id)
    }

    fn notify(&self) {
        self.0.subscribers.notify();
    }
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) {
        self.0.buf.borrow_mut().push_back(value);
        self.0.subscribers.notify();
    }
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let buf: Rc<Buf<T>> = Rc::default();
    (Sender(buf.clone()), Receiver(buf))
}
