use core::{cell::RefCell, ops::Deref};

use alloc::rc::Rc;
use waterui_reactive::{
    subscriber::{SharedSubscriberManager, SubscriberManager},
    Compute, Computed,
};

use crate::{AnyView, View};

pub struct DynamicView {
    inner: Computed<AnyView>,
}

#[derive(Clone)]
pub struct DynamicViewHandle {
    inner: Rc<RefCell<Option<AnyView>>>,
    subscribers: SharedSubscriberManager,
}

impl DynamicViewHandle {
    fn empty(subscribers: SharedSubscriberManager) -> Self {
        Self {
            inner: Rc::new(RefCell::new(None)),
            subscribers,
        }
    }

    pub fn set(&self, view: impl View + 'static) {
        let _ = self.inner.deref().borrow_mut().insert(AnyView::new(view));
        self.subscribers.notify();
    }
}

impl DynamicView {
    pub fn from_compute(compute: impl Compute<Output = AnyView>) -> Self {
        Self {
            inner: compute.computed(),
        }
    }

    pub fn new<Output: View + 'static>(
        default_view: impl 'static + Fn() -> Output,
    ) -> (Self, DynamicViewHandle) {
        let subscribers = Rc::new(SubscriberManager::new());
        let handle = DynamicViewHandle::empty(subscribers.clone());

        let computed = Computed::from_fn_with_subscribers(
            {
                let handle = handle.clone();
                move || {
                    handle
                        .inner
                        .deref()
                        .borrow_mut()
                        .take()
                        .unwrap_or(AnyView::new(default_view()))
                }
            },
            subscribers,
        );

        (Self::from_compute(computed), handle)
    }
}

impl View for DynamicView {
    fn body(self, _env: crate::Environment) -> impl View {
        self.inner
    }
}
