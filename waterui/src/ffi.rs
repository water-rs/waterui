use std::{mem::ManuallyDrop, ptr::slice_from_raw_parts_mut};

use waterui_reactive::{Binding, Compute, Computed};

use crate::{component, Environment, View, ViewExt};

#[repr(C)]
pub struct Data {
    head: *mut u8,
    len: usize,
}

macro_rules! ffi_opaque {
    ($from:ty,$to:ident,$word:expr) => {
        #[repr(C)]
        pub struct $to {
            inner: [usize; $word],
            _marker: std::marker::PhantomData<(*const (), std::marker::PhantomPinned)>,
        }

        impl From<$from> for $to {
            fn from(value: $from) -> Self {
                unsafe {
                    Self {
                        inner: std::mem::transmute(value),
                        _marker: std::marker::PhantomData,
                    }
                }
            }
        }

        impl From<$to> for $from {
            fn from(value: $to) -> Self {
                unsafe { std::mem::transmute(value.inner) }
            }
        }
    };
}

#[repr(C)]
pub struct TypeId {
    inner: [u64; 2],
    _marker: std::marker::PhantomData<(*const (), std::marker::PhantomPinned)>,
}

impl From<std::any::TypeId> for TypeId {
    fn from(value: std::any::TypeId) -> Self {
        unsafe {
            Self {
                inner: std::mem::transmute(value),
                _marker: std::marker::PhantomData,
            }
        }
    }
}
impl From<TypeId> for std::any::TypeId {
    fn from(value: TypeId) -> Self {
        unsafe { std::mem::transmute(value.inner) }
    }
}

ffi_opaque!(crate::component::AnyView, AnyView, 2);

impl From<Vec<u8>> for Data {
    fn from(value: Vec<u8>) -> Self {
        let len = value.len();
        let head = Box::into_raw(value.into_boxed_slice()) as *mut u8;

        Self { head, len }
    }
}

#[repr(C)]
pub struct Utf8Data {
    inner: Data,
}

impl From<Data> for Vec<u8> {
    fn from(val: Data) -> Self {
        unsafe { Box::from_raw(slice_from_raw_parts_mut(val.head, val.len)).into_vec() }
    }
}

impl From<Utf8Data> for String {
    fn from(val: Utf8Data) -> Self {
        unsafe { String::from_utf8_unchecked(val.inner.into()) }
    }
}

impl From<String> for Utf8Data {
    fn from(value: String) -> Self {
        Self {
            inner: Data::from(value.into_bytes()),
        }
    }
}

macro_rules! impl_computed {
    ($read:ident,$subscribe:ident,$unsubscribe:ident,$computed_ty:ident,$ty:ty,$output_ty:ty) => {
        ffi_opaque!(Computed<$ty>, $computed_ty, 2);

        #[no_mangle]
        unsafe extern "C" fn $read(computed: $computed_ty) -> $output_ty {
            let computed = ManuallyDrop::new(Computed::from(computed));
            computed.compute().into()
        }

        #[no_mangle]
        unsafe extern "C" fn $subscribe(computed: $computed_ty, subscriber: Subscriber) -> usize {
            let computed = ManuallyDrop::new(Computed::from(computed));
            computed.register_subscriber(Box::new(move || subscriber.call()))
        }

        #[no_mangle]
        unsafe extern "C" fn $unsubscribe(computed: $computed_ty, id: usize) {
            let computed = ManuallyDrop::new(Computed::from(computed));
            computed.cancel_subscriber(id);
        }
    };
}

macro_rules! impl_binding {
    ($read:ident,$write:ident,$subscribe:ident,$unsubscribe:ident,$binding_ty:ident,$ty:ty,$output_ty:ty) => {
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
            binding.set(value.into());
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
    };
}

impl_computed!(
    waterui_read_computed_data,
    waterui_subscribe_computed_data,
    waterui_unsubscribe_computed_data,
    ComputedData,
    Vec<u8>,
    Data
);

impl_computed!(
    waterui_read_computed_str,
    waterui_subscribe_computed_str,
    waterui_unsubscribe_computed_str,
    ComputedUtf8Data,
    String,
    Utf8Data
);

impl_binding!(
    waterui_read_binding_str,
    waterui_write_binding_str,
    waterui_subscribe_binding_str,
    waterui_unsubscribe_binding_str,
    BindingUtf8Data,
    String,
    Utf8Data
);

type Int = isize;

impl_binding!(
    waterui_read_binding_int,
    waterui_write_binding_int,
    waterui_subscribe_binding_int,
    waterui_unsubscribe_binding_int,
    BindingInt,
    Int,
    Int
);

impl_computed!(
    waterui_read_computed_int,
    waterui_subscribe_computed_int,
    waterui_unsubscribe_computed_int,
    ComputedInt,
    Int,
    Int
);

#[repr(C)]
pub struct Subscriber {
    data: *mut (),
    call: unsafe extern "C" fn(*const ()),
    free: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for Subscriber {}
unsafe impl Sync for Subscriber {}

impl Subscriber {
    pub fn call(&self) {
        unsafe { (self.call)(self.data) }
    }
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        unsafe { (self.free)(self.data) }
    }
}

#[no_mangle]
unsafe extern "C" fn waterui_view_id(view: AnyView) -> TypeId {
    let view = ManuallyDrop::new(component::AnyView::from(view));
    view.inner_type_id().into()
}

#[no_mangle]
unsafe extern "C" fn waterui_call_view(view: AnyView) -> AnyView {
    let view = component::AnyView::from(view);
    view.body(Environment::builder().build()).anyview().into()
}

#[repr(C)]
pub struct Text {
    content: ComputedUtf8Data,
}

impl From<component::Text> for Text {
    fn from(value: component::Text) -> Self {
        Self {
            content: value._content.into(),
        }
    }
}

impl From<Text> for component::Text {
    fn from(value: Text) -> Self {
        Self::new(Computed::from(value.content))
    }
}

macro_rules! impl_view {
    ($ty:ident,$force_as:ident,$id:ident,$free:ident) => {
        #[no_mangle]
        unsafe extern "C" fn $force_as(view: AnyView) -> $ty {
            let view: component::AnyView = view.into();
            (*view.downcast_unchecked::<component::$ty>()).into()
        }

        #[no_mangle]
        unsafe extern "C" fn $free(text: $ty) {
            let _ = component::$ty::from(text);
        }

        #[no_mangle]
        unsafe extern "C" fn $id() -> TypeId {
            std::any::TypeId::of::<component::$ty>().into()
        }
    };
}

impl_view!(
    Text,
    waterui_view_force_as_text,
    waterui_view_text_id,
    waterui_view_free_text
);
impl_view!(
    AnyView,
    waterui_view_force_as_anyview,
    waterui_view_anyview_id,
    waterui_view_free_anyview
);
