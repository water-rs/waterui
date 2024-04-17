use crate::{impl_constant, Binding, Compute, Computed};
use alloc::string::{String, ToString};
impl<T: Clone + 'static> Compute for Binding<T> {
    type Output = T;

    fn compute(&self) -> Self::Output {
        self.get()
    }

    fn register_subscriber(&self, subscriber: crate::Subscriber) -> usize {
        Binding::register_subscriber(self, subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        Binding::cancel_subscriber(self, id)
    }

    fn computed(self) -> Computed<Self::Output> {
        Computed::new(self.clone())
    }
}

impl_constant!(String, u64, i64, f64, bool);

impl Compute for &str {
    type Output = String;

    fn compute(&self) -> Self::Output {
        self.to_string()
    }

    fn register_subscriber(&self, _subscriber: crate::Subscriber) -> usize {
        0
    }

    fn cancel_subscriber(&self, _id: usize) {}

    fn computed(self) -> Computed<Self::Output> {
        Computed::new(self.to_string())
    }
}
