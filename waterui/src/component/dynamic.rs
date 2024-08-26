use core::cell::RefCell;

use alloc::{boxed::Box, rc::Rc};
use waterui_core::{raw_view, AnyView, View};

#[derive(Default)]
pub struct Dynamic(Rc<RefCell<DyanmicInner>>);

raw_view!(Dynamic);

pub struct DynamicHandler(Rc<RefCell<DyanmicInner>>);

#[derive(Default)]
struct DyanmicInner {
    receiver: Option<Box<dyn Fn(AnyView)>>,
    tmp: Option<AnyView>,
}

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

    pub fn connect(self, receiver: impl Fn(AnyView) + 'static) {
        let mut this = self.0.borrow_mut();
        if let Some(view) = this.tmp.take() {
            receiver(view);
        }
        this.receiver = Some(Box::new(receiver));
    }
}
