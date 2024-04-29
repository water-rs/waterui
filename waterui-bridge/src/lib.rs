#![no_std]
extern crate alloc;

use alloc::boxed::Box;
use async_channel::{bounded, Sender};
use waterui_core::Environment;

pub type Closure = Box<dyn Send + Sync + Fn()>;

#[derive(Clone)]
pub struct Bridge {
    sender: Sender<Closure>,
}

impl Bridge {
    pub fn new(env: &mut Environment) -> Self {
        let (sender, receiver) = bounded(64);

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
