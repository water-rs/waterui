use crate::{component::AnyView, Environment, View, ViewExt};

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

    pub fn env<T: 'static>(mut self, value: T) -> Self {
        self._env.insert(value);
        self
    }

    pub fn ready(&self) -> AppIgniter {
        AppIgniter {
            #[cfg(feature = "async")]
            executor: self._env.executor(),
        }
    }
}

pub struct AppIgniter {
    #[cfg(feature = "async")]
    executor: crate::env::SharedExecutor,
}

impl AppIgniter {
    #[cfg(feature = "async")]
    pub fn task<Fut>(&self, fut: Fut) -> smol::Task<Fut::Output>
    where
        Fut: core::future::Future + 'static,
        Fut::Output: 'static,
    {
        self.executor.spawn(fut)
    }

    pub fn ignite(&self) {
        #[cfg(feature = "async")]
        smol::block_on(async move {
            loop {
                self.executor.tick().await;
            }
        })
    }
}
