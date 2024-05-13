use core::cell::RefCell;

use alloc::{collections::VecDeque, rc::Rc};

type Buf<T> = RefCell<VecDeque<T>>;

pub struct Sender<T>(Rc<Buf<T>>);

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct Receiver<T>(Rc<Buf<T>>);

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Option<T> {
        self.0.borrow_mut().pop_front()
    }
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) {
        self.0.borrow_mut().push_back(value);
    }
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let buf: Rc<Buf<T>> = Rc::default();
    (Sender(buf.clone()), Receiver(buf))
}
