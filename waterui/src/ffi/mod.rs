#[macro_use]
mod array;
pub use array::Buf;
pub mod utils;

use crate::{
    component::{self, stack::StackMode, AnyView},
    env::EnvironmentBuilder,
    layout::Frame,
    modifier::{self, Display, ViewModifier},
    view::View,
    Binding, Environment, Reactive, ViewExt,
};
use std::{
    mem::{forget, ManuallyDrop},
    ops::Deref,
    ptr::{null, write},
};
use waterui_reactive::Subscriber;

use self::utils::{EventObject, ViewObject};

/// # Safety
/// `EventObject` must be valid
#[no_mangle]
pub unsafe extern "C" fn waterui_call_event_object(object: EventObject) {
    (object.as_ref())();
}

/// # Safety
/// Must be valid `Reactive<String>`.
#[no_mangle]
pub unsafe extern "C" fn waterui_get_reactive_string(reactive: *const ()) -> Buf {
    let reactive: ManuallyDrop<Reactive<String>> =
        ManuallyDrop::new(Reactive::from_raw(reactive as *mut String));
    let reactive = reactive.get();
    reactive.deref().to_string().into()
}

macro_rules! impl_reactive_subscribe {
    ($(($ident:ident,$ty:ty)),*) => {
        $(
            /// # Safety
            /// Must be valid `Reactive`
            #[no_mangle]
            pub unsafe extern "C" fn $ident(
                reactive: *const (),
                subscriber: Subscriber,
            ) {
                let reactive: ManuallyDrop<Reactive<$ty>> =
                    ManuallyDrop::new(Reactive::from_raw(reactive as *mut $ty));
                reactive.subscribe(subscriber);
            }
        )*
    };
}

macro_rules! impl_binding_subscribe {
    ($(($ident:ident,$ty:ty)),*) => {
        $(
            /// # Safety
            /// Must be valid `Binding`
            #[no_mangle]
            pub unsafe extern "C" fn $ident(
                binding: *const (),
                subscriber: Subscriber,
            ) {
                let binding: ManuallyDrop<Binding<$ty>> =
                    ManuallyDrop::new(Binding::from_raw(binding as *mut $ty));
                binding.subscribe(subscriber);
            }
        )*
    };
}

impl_reactive_subscribe![
    (waterui_subscribe_reactive_string, String),
    (waterui_subscribe_reactive_view, AnyView),
    (waterui_subscribe_reactive_bool, bool)
];

impl_binding_subscribe![
    (waterui_subscribe_binding_string, String),
    (waterui_subscribe_binding_bool, bool),
    (waterui_subscribe_binding_int, i64)
];

/// # Safety
/// Must be valid `Binding<String>`.
#[no_mangle]
pub unsafe extern "C" fn waterui_get_binding_string(binding: *const ()) -> Buf {
    let binding: ManuallyDrop<Binding<String>> =
        ManuallyDrop::new(Binding::from_raw(binding as *mut String));
    let binding = binding.get();
    binding.deref().to_string().into()
}

/// # Safety
/// Must be valid `Binding<i64>`.
#[no_mangle]
pub unsafe extern "C" fn waterui_get_binding_int(binding: *const ()) -> i64 {
    let binding = ManuallyDrop::new(Binding::from_raw(binding as *mut i64));
    let guard = binding.get();
    *guard
}

/// # Safety
/// Must be valid `Binding<u64>`.
#[no_mangle]
pub unsafe extern "C" fn waterui_increment_binding_int(binding: *const (), num: i64) {
    let binding: ManuallyDrop<Binding<i64>> =
        ManuallyDrop::new(Binding::from_raw(binding as *mut i64));
    binding.increment(num);
}

/// # Safety
/// `Binding<String>` must be valid, and `Buf` must be valid UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn waterui_set_binding_string(binding: *const (), string: Buf) {
    let binding: ManuallyDrop<Binding<String>> =
        ManuallyDrop::new(Binding::from_raw(binding as *mut String));
    binding.set(String::from_utf8_unchecked(string.to_vec()))
}

/// # Safety
/// Must be valid `Reactive<BoxView>`.
#[no_mangle]
pub unsafe extern "C" fn waterui_get_reactive_view(binding: *const ()) -> ViewObject {
    let binding = ManuallyDrop::new(Reactive::from_raw(binding as *const AnyView));
    binding.take().into()
}

/// # Safety
/// Must be valid `Binding<bool>`
#[no_mangle]
pub unsafe extern "C" fn waterui_get_binding_bool(binding: *const ()) -> bool {
    let binding = ManuallyDrop::new(Binding::from_raw(binding as *const bool));
    let guard = binding.get();
    guard.to_owned()
}

/// # Safety
/// Must be valid `Reactive<bool>`
#[no_mangle]
pub unsafe extern "C" fn waterui_get_reactive_bool(reactive: *const ()) -> bool {
    let reactive = ManuallyDrop::new(Reactive::from_raw(reactive as *const bool));
    reactive.take()
}

/// # Safety
/// Must be valid `Binding<bool>`
#[no_mangle]
pub unsafe extern "C" fn waterui_set_binding_bool(binding: *const (), bool: bool) {
    let binding = ManuallyDrop::new(Binding::from_raw(binding as *const bool));
    binding.set(bool);
}

macro_rules! impl_component{
    ($(($ident:ident,$ty:tt)),*) => {
        $(
            /// # Safety
            /// `EventObject` must be valid
            #[no_mangle]
            pub unsafe extern "C" fn $ident(view: ViewObject,value:*mut $ty) -> i8{
                if let Some(component) =  downcast::<component::$ty>(view){
                    write(value,component.into());
                    0
                }
                else{
                    -1
                }
            }
        )*
    };
}

macro_rules! impl_modifier{
    ($(($ident:ident,$modifier:ty)),*) => {
        $(
            /// # Safety
            /// `EventObject` must be valid
            #[no_mangle]
            pub unsafe extern "C" fn $ident(view: ViewObject,value:*mut Modifier) -> i8{
                if let Some(modifier) =  downcast::<modifier::Modifier<$modifier>>(view){
                    write(value,modifier.into());
                    0
                }
                else{
                    -1
                }
            }
        )*
    };
}

/// # Safety
/// Must be valid `Reactive<BoxView>`.
#[no_mangle]
pub unsafe extern "C" fn waterui_view_to_reactive_view(view: ViewObject) -> *const () {
    downcast::<Reactive<AnyView>>(view)
        .map(|v| v.into_raw() as *const ())
        .unwrap_or(null())
}

#[no_mangle]
pub unsafe extern "C" fn waterui_unwrap_anyview(mut view: ViewObject) -> ViewObject {
    while let Some(anyview) = downcast::<AnyView>(view) {
        view = anyview.into();
    }
    view
}

unsafe fn downcast<T: 'static>(view: ViewObject) -> Option<T> {
    let view = view.into_anyview();
    match view.downcast::<T>() {
        Ok(output) => Some(*output),
        Err(view) => {
            forget(view);
            None
        }
    }
}

/// # Safety
/// `EventObject` must be valid
#[no_mangle]
pub unsafe extern "C" fn waterui_view_to_empty(view: ViewObject) -> i8 {
    if downcast::<()>(view).is_some() {
        0
    } else {
        -1
    }
}

impl_component!(
    (waterui_view_to_text, Text),
    (waterui_view_to_button, Button),
    (waterui_view_to_image, Image),
    (waterui_view_to_text_field, TextField),
    (waterui_view_to_stack, Stack),
    (waterui_view_to_toggle, Toggle),
    (waterui_view_to_stepper, Stepper)
);

impl_modifier!(
    (waterui_view_to_frame_modifier, Frame),
    (waterui_view_to_display_modifier, Display)
);

/// # Safety
/// `EventObject` must be valid
#[no_mangle]
pub unsafe extern "C" fn waterui_call_view(view: ViewObject, env: *const ()) -> ViewObject {
    view.into_anyview()
        //.body(Environment::from_raw(env as *const EnvironmentBuilder))
        .body(Environment::builder().build())
        .anyview()
        .into()
}

#[repr(C)]
pub struct Text {
    text: *const (),
    selectable: *const (),
}

#[repr(C)]
pub struct Modifier {
    modifier: *const (),
    view: ViewObject,
}

impl<T: ViewModifier> From<modifier::Modifier<T>> for Modifier {
    fn from(value: modifier::Modifier<T>) -> Self {
        Self {
            modifier: value.modifier.into_raw() as *const (),
            view: value.content.into(),
        }
    }
}

impl_array!(Views, AnyView, ViewObject);
#[repr(C)]
pub struct Stack {
    mode: StackMode,
    contents: Views,
}

#[repr(C)]
pub struct Button {
    label: ViewObject,
    action: EventObject,
}

#[repr(C)]
pub struct TextField {
    label: *const (),
    value: *const (),
    prompt: *const (),
}

impl From<component::TextField> for TextField {
    fn from(value: component::TextField) -> Self {
        Self {
            label: value.label.into_raw() as *const (),
            value: value.value.into_raw() as *const (),
            prompt: value.prompt.into_raw() as *const (),
        }
    }
}

#[repr(C)]
pub struct Image {
    data: Buf,
}

#[repr(C)]
pub struct App {
    view: ViewObject,
    env: *const (),
}

impl From<crate::App> for App {
    fn from(value: crate::App) -> Self {
        Self {
            view: value.view.into(),
            env: value.environment.into_raw() as *const (),
        }
    }
}

impl From<component::Image> for Image {
    fn from(value: component::Image) -> Self {
        Self {
            data: value.data.into(),
        }
    }
}

impl From<component::Text> for Text {
    fn from(value: component::Text) -> Self {
        let text: Reactive<String> = value.text.to(|v| v.clone().into_plain());

        Self {
            text: text.into_raw() as *const (),
            selectable: value.selectable.into_raw() as *const (),
        }
    }
}

impl From<component::Button> for Button {
    fn from(value: component::Button) -> Self {
        Self {
            label: value.label.into(),
            action: value.action.into(),
        }
    }
}

impl From<component::Stack> for Stack {
    fn from(value: component::Stack) -> Self {
        Self {
            mode: StackMode::Vertical,
            contents: value.contents.into(),
        }
    }
}

#[repr(C)]
pub struct Toggle {
    label: ViewObject,
    toggle: *const (),
}

impl From<component::Toggle> for Toggle {
    fn from(value: component::Toggle) -> Self {
        Self {
            label: value.label.into(),
            toggle: value.toggle.into_raw() as *const (),
        }
    }
}

#[repr(C)]
pub struct Stepper {
    text: ViewObject,
    value: *const (),
    step: u64,
}

impl From<component::Stepper> for Stepper {
    fn from(value: component::Stepper) -> Self {
        Self {
            text: value.text.into(),
            value: value.value.into_raw() as *const (),
            step: value.step,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn waterui_env_increment_count(env: *const ()) {
    let env = ManuallyDrop::new(Environment::from_raw(env as *const EnvironmentBuilder));
    let _ = env.clone();
}

#[no_mangle]
pub unsafe extern "C" fn waterui_env_decrement_count(env: *const ()) {
    Environment::from_raw(env as *const EnvironmentBuilder);
}

extern "C" {
    /*
    pub fn waterui_create_window(title: Buf, content: ViewObject) -> usize;
    pub fn waterui_window_closeable(id: usize, is: bool);
    pub fn waterui_close_window(id: usize);*/
    pub fn waterui_main() -> App;
}
