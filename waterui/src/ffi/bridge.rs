use core::future::Future;

use async_channel::{bounded, Sender};

use crate::Environment;

use super::Closure;

#[derive(Debug, Clone)]
pub struct Bridge {
    sender: Sender<Closure>,
}

impl Bridge {
    pub fn new() -> (Self, impl Future) {
        let (sender, receiver) = bounded(64);
        let bridge = Self { sender };
        let fut = async move {
            while let Ok(f) = receiver.recv().await {
                f.call();
            }
        };
        (bridge, fut)
    }
}

#[no_mangle]
unsafe extern "C" fn waterui_run_on_rust(env: *const Environment, f: Closure) {
    let bridge = (*env).bridge();
    let mut result = bridge.sender.try_send(f);
    loop {
        if let Err(error) = result {
            if error.is_full() {
                result = bridge.sender.try_send(error.into_inner());
                continue;
            }
        }
        break;
    }
}
