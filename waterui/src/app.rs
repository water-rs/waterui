use core::future::Future;

use crate::{AnyView, Environment, View, ViewExt};

pub struct App {
    pub _content: AnyView,
    pub _env: Environment,
}

#[cfg(feature = "async")]
type Closure = alloc::boxed::Box<dyn Send + Sync + Fn()>;

#[cfg(feature = "async")]
#[derive(Clone)]
pub struct Bridge {
    sender: async_channel::Sender<Closure>,
}

#[cfg(feature = "async")]
impl Bridge {
    pub fn new(env: &mut Environment) -> Self {
        let (sender, receiver) = async_channel::bounded(64);

        let bridge = Self { sender };
        env.task(async move {
            loop {
                if let Ok(f) = receiver.recv().await {
                    f()
                }
            }
        })
        .detach();
        bridge
    }

    pub async fn send(
        &self,
        f: impl Fn() + Send + Sync + 'static,
    ) -> Result<(), async_channel::SendError<Closure>> {
        self.sender.send(alloc::boxed::Box::new(f)).await
    }

    pub fn send_blocking(
        &self,
        f: impl Fn() + Send + Sync + 'static,
    ) -> Result<(), async_channel::SendError<Closure>> {
        self.sender.send_blocking(alloc::boxed::Box::new(f))
    }
}

impl App {
    pub fn new(content: impl View + 'static) -> Self {
        Self {
            _content: content.anyview(),
            _env: Environment::new(),
        }
    }

    pub fn run<F, Fut>(self, start: F)
    where
        F: FnOnce(Self) -> Fut,
        Fut: Future<Output = ()> + 'static,
    {
        let env = self._env.clone();
        env.task(start(self)).detach();
        smol::block_on(env.executor().run());
    }
}
