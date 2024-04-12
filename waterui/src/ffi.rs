use std::{any::TypeId, collections::HashMap, mem::ManuallyDrop};

use waterui_reactive::{Binding, Compute, ComputeExt, Computed};

use crate::component::{self, AnyView};

#[repr(C)]
pub struct Data {
    head: *mut u8,
    len: usize,
    capacity: usize,
}

impl From<Vec<u8>> for Data {
    fn from(mut value: Vec<u8>) -> Self {
        let head = value.as_mut_ptr();
        let len = value.len();
        let capacity = value.capacity();

        Self {
            head,
            len,
            capacity,
        }
    }
}

#[repr(C)]
pub struct Utf8Data {
    inner: Data,
}

impl From<Data> for Vec<u8> {
    fn from(val: Data) -> Self {
        unsafe { Vec::from_raw_parts(val.head, val.len, val.capacity) }
    }
}

impl From<Utf8Data> for String {
    fn from(val: Utf8Data) -> Self {
        unsafe { String::from_raw_parts(val.inner.head, val.inner.len, val.inner.capacity) }
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
    ($read:ident,$subscribe:ident,$computed_ty:ident,$ty:ty,$output_ty:ty) => {
        #[repr(C)]
        pub struct $computed_ty {
            pointer: *mut $ty,
        }

        impl From<Computed<$ty>> for $computed_ty {
            fn from(value: Computed<$ty>) -> Self {
                Self {
                    pointer: value.into_raw(),
                }
            }
        }

        impl From<$computed_ty> for Computed<$ty> {
            fn from(value: $computed_ty) -> Self {
                unsafe { Self::from_raw(value.pointer) }
            }
        }

        #[no_mangle]
        unsafe extern "C" fn $read(computed: $computed_ty) -> $output_ty {
            let computed = ManuallyDrop::new(Computed::from_raw(computed.pointer));
            computed.compute().into()
        }

        #[no_mangle]
        unsafe extern "C" fn $subscribe(computed: $computed_ty, subscriber: Subscriber) {
            let computed = ManuallyDrop::new(Computed::from_raw(computed.pointer));
            computed.subscribe(move || subscriber.call());
        }
    };
}

macro_rules! impl_binding {
    ($read:ident,$write:ident,$subscribe:ident,$binding_ty:ident,$ty:ty,$output_ty:ty) => {
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
        unsafe extern "C" fn $subscribe(binding: $binding_ty, subscriber: Subscriber) {
            let binding = ManuallyDrop::new(Binding::from_raw(binding.pointer));
            binding.subscribe(move || subscriber.call());
        }
    };
}

impl_computed!(
    waterui_read_computed_data,
    waterui_subscribe_computed_data,
    ComputedData,
    Vec<u8>,
    Data
);

impl_computed!(
    waterui_read_computed_str,
    waterui_subscribe_computed_str,
    ComputedUtf8Data,
    String,
    Utf8Data
);

impl_binding!(
    waterui_read_binding_str,
    waterui_write_binding_str,
    waterui_subscribe_binding_str,
    BindingUtf8Data,
    String,
    Utf8Data
);

type Int = isize;

impl_binding!(
    waterui_read_binding_int,
    waterui_write_binding_int,
    waterui_subscribe_binding_int,
    BindingInt,
    Int,
    Int
);

impl_computed!(
    waterui_read_computed_int,
    waterui_subscribe_computed_int,
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

#[repr(C)]
struct ViewHandler {
    data: *const (),
    f: unsafe extern "C" fn(*const (), *mut AnyView),
}

impl ViewHandler {
    pub fn call(&self, view: Box<AnyView>) {
        unsafe { (self.f)(self.data, Box::into_raw(view)) }
    }
}

struct ViewMap {
    map: HashMap<TypeId, ViewHandler>,
}
#[no_mangle]
unsafe extern "C" fn waterui_insert_view_map(
    map: *mut ViewMap,
    view: *mut AnyView,
    f: ViewHandler,
) {
    let mut map = ManuallyDrop::new(Box::from_raw(map));
    let view = Box::from_raw(view);
    map.map.insert(view.inner_type_id(), f);
}

#[no_mangle]
unsafe extern "C" fn waterui_call_view_map(map: *const ViewMap, view: *mut AnyView) {
    let map = ManuallyDrop::new(Box::from_raw(map as *mut ViewMap));
    let view = Box::from_raw(view);

    let handler = map.map.get(&view.inner_type_id()).unwrap();
    handler.call(view);
}
#[no_mangle]
unsafe extern "C" fn waterui_free_view_map(map: *mut ViewMap) {
    let _ = Box::from_raw(map);
}

#[no_mangle]
unsafe extern "C" fn waterui_view_text(view: *mut ()) -> *mut TypeId {
    let view = ManuallyDrop::new(Box::from_raw(view as *mut AnyView));
    Box::into_raw(Box::new(view.inner_type_id()))
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

#[no_mangle]
unsafe extern "C" fn waterui_view_force_as_text(view: *mut (), text: *mut Text) {
    let view = *Box::from_raw(view as *mut AnyView);
    *text = (*view.downcast_unchecked::<component::Text>()).into();
}

#[no_mangle]
unsafe extern "C" fn waterui_view_free_text(text: Text) {
    let _ = component::Text::from(text);
}
