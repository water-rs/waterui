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
        let container: RefCell<Option<AnyView>> = RefCell::new(None);
        let binding = Binding::from_fn_with_state(
            container,
            move |container| {
                container
                    .borrow_mut()
                    .take()
                    .unwrap_or_else(|| AnyView::new(default_view()))
            },
            |container, view| {
                let _ = container.borrow_mut().insert(view);
            },
        );

        (Self::from_compute(binding), handle)
    }
}

impl View for DynamicView {
    fn body(self, _env: crate::Environment) -> impl View {
        self.inner
    }
}
