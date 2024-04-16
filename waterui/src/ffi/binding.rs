use super::{Int, Subscriber, Utf8Data};
use alloc::{boxed::Box, string::String};
use core::mem::ManuallyDrop;
use waterui_reactive::Binding;

macro_rules! impl_binding {
    ($read:ident,$write:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident,$binding_ty:ident,$ty:ty,$output_ty:ty) => {
        #[repr(C)]
        pub struct $binding_ty {
            pointer: *const $ty,
        }

        impl From<Binding<$ty>> for $binding_ty {
            fn from(value: Binding<$ty>) -> Self {
                Self {
                    pointer: value.into_raw(),
                }
            }
        }

        impl From<$binding_ty> for Binding<$ty> {
            fn from(value: $binding_ty) -> Self {
                unsafe { Self::from_raw(value.pointer) }
            }
        }

        #[no_mangle]
        unsafe extern "C" fn $read(binding: $binding_ty) -> $output_ty {
            let binding = ManuallyDrop::new(Binding::from_raw(binding.pointer));
            binding.get().into()
        }

        #[no_mangle]
        unsafe extern "C" fn $write(binding: $binding_ty, value: $output_ty) {
            let binding = ManuallyDrop::new(Binding::from_raw(binding.pointer));
            binding.set(value);
        }

        #[no_mangle]
        unsafe extern "C" fn $subscribe(binding: $binding_ty, subscriber: Subscriber) -> usize {
            let binding = ManuallyDrop::new(Binding::from_raw(binding.pointer));
            binding.register_subscriber(Box::new(move || subscriber.call()))
        }

        #[no_mangle]
        unsafe extern "C" fn $unsubscribe(binding: $binding_ty, id: usize) {
            let binding = ManuallyDrop::new(Binding::from_raw(binding.pointer));
            binding.cancel_subscriber(id);
        }

        #[no_mangle]
        unsafe extern "C" fn $drop(binding: $binding_ty) {
            let _ = Binding::from_raw(binding.pointer);
        }
    };
}

impl_binding!(
    waterui_read_binding_str,
    waterui_write_binding_str,
    waterui_subscribe_binding_str,
    waterui_unsubscribe_binding_str,
    waterui_drop_binding_str,
    BindingStr,
    String,
    Utf8Data
);

impl_binding!(
    waterui_read_binding_int,
    waterui_write_binding_int,
    waterui_subscribe_binding_int,
    waterui_unsubscribe_binding_int,
    waterui_drop_binding_int,
    BindingInt,
    Int,
    Int
);
