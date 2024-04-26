use core::{cell::RefCell, ops::Deref};

use crate::{AnyView, View};
use alloc::rc::Rc;
use waterui_reactive::ComputeExt;
use waterui_reactive::{
    subscriber::{SharedSubscriberManager, SubscriberManager},
    Binding, Compute, Computed,
};

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
    pub fn from_compute(compute: impl Compute<Output = AnyView> + 'static) -> Self {
        Self {
            inner: compute.computed(),
        }
    }

    pub fn new<Output: View + 'static>(
        default_view: impl 'static + Fn() -> Output,
    ) -> (Self, DynamicViewHandle) {
        let subscribers = Rc::new(SubscriberManager::new());
        let handle = DynamicViewHandle::empty(subscribers.clone());
        let binding: Binding<Option<AnyView>> = Binding::new(None);

        let compute = binding.to_compute(move |v| {
            v.get_mut()
                .take()
                .unwrap_or_else(|| AnyView::new(default_view()))
        });

        (Self::from_compute(compute), handle)
    }
}

impl View for DynamicView {
    fn body(self, _env: crate::Environment) -> impl View {
        self.inner
    }
}
