use crate::{array::waterui_str, closure::waterui_closure, IntoFFI, IntoRust};
use alloc::{borrow::Cow, boxed::Box, string::String, vec::Vec};
use core::{marker::PhantomData, mem::ManuallyDrop, ptr::drop_in_place};
use waterui_reactive::{Binding, Compute, Int, Reactive};
// WARNING: Binding<T> must be called on the Rust thread!!!

#[repr(C)]
pub struct waterui_binding<T> {
    _priv: [u8; 0],
    _marker: PhantomData<*const T>,
}

pub type waterui_binding_str = waterui_binding<Cow<'static, str>>;
pub type waterui_binding_int = waterui_binding<Int>;
pub type waterui_binding_bool = waterui_binding<bool>;
pub type waterui_binding_data = waterui_binding<Vec<u8>>;

impl<T> IntoFFI for Binding<T> {
    type FFI = *const waterui_binding<T>;
    fn into_ffi(self) -> Self::FFI {
        self.into_raw() as *const waterui_binding<T>
    }
}

impl<T> IntoRust for *const waterui_binding<T> {
    type Rust = Binding<T>;
    unsafe fn into_rust(self) -> Self::Rust {
        Binding::from_raw(self as *const T)
    }
}

macro_rules! impl_binding {
    ($ty:ty,$ffi:ty,$read:ident,$write:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $read(binding: *const waterui_binding<$ty>) -> $ffi {
            let binding = ManuallyDrop::new(binding.into_rust());
            binding.compute().into_ffi()
        }

        #[no_mangle]
        pub unsafe extern "C" fn $write(binding: *const waterui_binding<$ty>, value: $ffi) {
            let binding = ManuallyDrop::new(binding.into_rust());

            binding.set(value.into_rust());
        }

        #[no_mangle]
        pub unsafe extern "C" fn $subscribe(
            binding: *const waterui_binding<$ty>,
            subscriber: waterui_closure,
        ) -> Int {
            let binding = ManuallyDrop::new(binding.into_rust());
            binding
                .register_subscriber(Box::new(move || subscriber.call()))
                .map(|v| v.get() as Int)
                .unwrap_or(-1)
        }

        #[no_mangle]
        pub unsafe extern "C" fn $unsubscribe(binding: *const waterui_binding<$ty>, id: usize) {
            let binding = ManuallyDrop::new(binding.into_rust());

            let id = core::num::NonZeroUsize::new(id).unwrap();
            binding.cancel_subscriber(id);
        }

        #[no_mangle]
        pub unsafe extern "C" fn $drop(binding: *mut waterui_binding<$ty>) {
            drop_in_place(binding);
        }
    };
}

impl_binding!(
    String,
    waterui_str,
    waterui_read_binding_str,
    waterui_write_binding_str,
    waterui_subscribe_binding_str,
    waterui_unsubscribe_binding_str,
    waterui_drop_binding_str
);

impl_binding!(
    Int,
    Int,
    waterui_read_binding_int,
    waterui_write_binding_int,
    waterui_subscribe_binding_int,
    waterui_unsubscribe_binding_int,
    waterui_drop_binding_int
);

impl_binding!(
    bool,
    bool,
    waterui_read_binding_bool,
    waterui_write_binding_bool,
    waterui_subscribe_binding_bool,
    waterui_unsubscribe_binding_bool,
    waterui_drop_binding_bool
);
