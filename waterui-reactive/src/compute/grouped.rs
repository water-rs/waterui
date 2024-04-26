use core::num::NonZeroUsize;

use alloc::boxed::Box;

use crate::{
    subscriber::{BoxSubscriber, SharedSubscriberManager},
    Compute, Reactive,
};

pub struct GroupedCompute<C, const LEN: usize>
where
    C: SubscribeManage<LEN>,
{
    computes: Option<C>,
    guards: [Option<NonZeroUsize>; LEN],
    subscribers: SharedSubscriberManager,
}

impl<V1, V2> GroupedCompute<(V1, V2), 2>
where
    V1: Compute,
    V2: Compute,
{
    pub fn new(v1: V1, v2: V2) -> Self {
        let subscribers = SharedSubscriberManager::default();
        let computes = (v1, v2);
        let guards = computes.register_subscribers(|| {
            let subscribers = subscribers.clone();
            Box::new(move || subscribers.notify())
        });

        Self {
            computes: Some(computes),
            subscribers,
            guards,
        }
    }
}

impl<C, const LEN: usize> Drop for GroupedCompute<C, LEN>
where
    C: SubscribeManage<LEN>,
{
    fn drop(&mut self) {
        self.computes
            .as_ref()
            .inspect(|c| c.cancel_subscribers(self.guards));
    }
}

impl<C1, C2> Compute for GroupedCompute<(C1, C2), 2>
where
    C1: Compute,
    C2: Compute,
    C1::Output: 'static,
    C2::Output: 'static,
{
    type Output = (C1::Output, C2::Output);
    fn compute(&self) -> Self::Output {
        let computes = self.computes.as_ref().unwrap();
        (computes.0.compute(), computes.1.compute())
    }
}

impl<C, const LEN: usize> Reactive for GroupedCompute<C, LEN>
where
    C: SubscribeManage<LEN>,
{
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
        Some(self.subscribers.register(subscriber))
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.subscribers.cancel(id)
    }

    fn notify(&self) {
        self.subscribers.notify();
    }
}

#[doc(hidden)]
pub trait SubscribeManage<const LEN: usize> {
    fn register_subscribers(
        &self,
        subscriber: impl Fn() -> BoxSubscriber,
    ) -> [Option<NonZeroUsize>; LEN];
    fn cancel_subscribers(&self, guard: [Option<NonZeroUsize>; LEN]);
}

impl<C1, C2> SubscribeManage<2> for (C1, C2)
where
    C1: Compute,
    C2: Compute,
{
    fn register_subscribers(
        &self,
        subscriber: impl Fn() -> BoxSubscriber,
    ) -> [Option<NonZeroUsize>; 2] {
        [
            self.0.register_subscriber(subscriber()),
            self.0.register_subscriber(subscriber()),
        ]
    }

    fn cancel_subscribers(&self, guard: [Option<NonZeroUsize>; 2]) {
        guard[0].inspect(|id| self.0.cancel_subscriber(*id));
        guard[1].inspect(|id| self.1.cancel_subscriber(*id));
    }
}
