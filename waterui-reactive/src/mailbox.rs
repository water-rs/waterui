use crate::compute::ComputeExt;
use crate::{compute::ComputeResult, watcher::WatcherGuard, Binding};
use async_channel::unbounded;
use pin_project_lite::pin_project;
use waterui_task::{MainValue, Stream};

pub struct Mailbox<T: ComputeResult> {
    binding: MainValue<Binding<T>>,
}

impl<T: ComputeResult> Mailbox<T> {
    pub fn new(binding: &Binding<T>) -> Self {
        Self {
            binding: MainValue::new(binding.clone()),
        }
    }

    pub async fn get<V: Send + 'static + From<T>>(&self) -> V {
        self.binding.handle(|v| V::from(v.get())).await
    }

    pub async fn set<V: Send + 'static + Into<T>>(&self, value: V) {
        self.binding
            .handle(|v| {
                v.set(value.into());
            })
            .await;
    }

    pub async fn watch(&self, watcher: impl Fn(T) + Send + 'static) -> MainValue<WatcherGuard> {
        self.binding
            .handle(move |v| MainValue::new(v.watch(watcher)))
            .await
    }

    pub async fn receive<V: Send + From<T> + 'static>(&self) -> Receiver<V> {
        let (sender, receiver) = unbounded();
        let guard = self
            .watch(move |v| {
                sender.send_blocking(V::from(v)).unwrap();
            })
            .await;

        Receiver { receiver, guard }
    }
}

pin_project! {
    pub struct Receiver<V> {
        #[pin]
        receiver: async_channel::Receiver<V>,
        guard: MainValue<WatcherGuard>,
    }
}

impl<V> Stream for Receiver<V> {
    type Item = V;
    fn poll_next(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Option<Self::Item>> {
        self.project().receiver.poll_next(cx)
    }
}
