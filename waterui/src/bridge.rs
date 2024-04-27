use waterui_view::Environment;

pub type Closure = alloc::boxed::Box<dyn Send + Sync + Fn()>;

#[derive(Clone)]
pub struct Bridge {
    sender: async_channel::Sender<Closure>,
}

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
