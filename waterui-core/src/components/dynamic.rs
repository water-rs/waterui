use core::cell::RefCell;

use crate::{raw_view, AnyView, View};
use alloc::{boxed::Box, rc::Rc};
use waterui_reactive::Compute;

#[derive(Default)]
pub struct Dynamic(Rc<RefCell<DyanmicInner>>);

raw_view!(Dynamic);

pub struct DynamicHandler(Rc<RefCell<DyanmicInner>>);

#[derive(Default)]
struct DyanmicInner {
    receiver: Option<Box<dyn Fn(AnyView)>>,
    tmp: Option<AnyView>,
}

impl_debug!(Dynamic);
impl_debug!(DynamicHandler);

impl DynamicHandler {
    pub fn set(&self, view: impl View) {
        let view = AnyView::new(view);
        let mut this = self.0.borrow_mut();
        if let Some(ref receiver) = this.receiver {
            receiver(view)
        } else {
            this.tmp = Some(view);
        }
    }
}

impl Dynamic {
    pub fn new() -> (DynamicHandler, Self) {
        let inner = Rc::new(RefCell::new(DyanmicInner::default()));
        (DynamicHandler(inner.clone()), Self(inner))
    }

    pub fn watch<T, V: View>(
        value: impl Compute<Output = T>,
        f: impl Fn(T) -> V + 'static,
    ) -> Self {
        let (handle, dyanmic) = Self::new();
        handle.set(f(value.compute()));
        value.watch(move |value| handle.set(f(value))).leak();
        dyanmic
    }

    pub fn connect(self, receiver: impl Fn(AnyView) + 'static) {
        let mut this = self.0.borrow_mut();
        if let Some(view) = this.tmp.take() {
            receiver(view);
        }
        this.receiver = Some(Box::new(receiver));
    }
}
