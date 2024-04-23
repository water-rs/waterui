use waterui_view::Environment;

#[cfg(feature = "async")]
pub type Closure = alloc::boxed::Box<dyn Send + Sync + Fn()>;

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

mod ffi {
    use waterui_ffi::{ffi_clone, ffi_opaque, Closure, IntoFFI};

    use super::Environment;

    ffi_opaque!(Bridge, super::Bridge, 1, waterui_drop_bridge);

    #[no_mangle]
    unsafe extern "C" fn waterui_send_to_bridge(bridge: *const Bridge, f: Closure) -> i8 {
        if (*bridge).send_blocking(move || f.call()).is_ok() {
            0
        } else {
            -1
        }
    }

    #[no_mangle]
    unsafe extern "C" fn waterui_create_bridge(env: *mut Environment) -> Bridge {
        super::Bridge::new(&mut *env).into_ffi()
    }

    ffi_clone!(waterui_clone_bridge, Bridge);
}
