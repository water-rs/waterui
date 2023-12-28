#[macro_use]
mod array;
pub use array::Buf;
use waterui_reactive::binding::Binding;
pub mod utils;

use crate::env::EnvironmentBuilder;
use crate::layout::Frame;
use crate::modifier::Modifier;
use crate::view::View;
use crate::Environment;
use crate::{component, view::BoxView, Reactive};
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::ptr::write;
use std::ptr::{null, read};
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

/// # Safety
/// Must be valid `Reactive<String>`
#[no_mangle]
pub unsafe extern "C" fn waterui_subscribe_reactive_string(
    reactive: *const (),
    subscriber: Subscriber,
) {
    let reactive: ManuallyDrop<Reactive<String>> =
        ManuallyDrop::new(Reactive::from_raw(reactive as *mut String));
    reactive.on_update(subscriber);
}

/// # Safety
/// Must be valid `Reactive<BoxView>`.
#[no_mangle]
pub unsafe extern "C" fn waterui_get_reactive_view(binding: *const ()) -> ViewObject {
    let binding = ManuallyDrop::new(Reactive::from_raw(binding as *const BoxView));
    binding.take().into()
}

/// # Safety
/// Must be valid `Reactive<BoxView>`.
#[no_mangle]
pub unsafe extern "C" fn waterui_subscribe_reactive_view(
    reactive: *const (),
    subscriber: Subscriber,
) {
    let reactive = ManuallyDrop::new(Reactive::from_raw(reactive as *const BoxView));
    reactive.on_update(subscriber)
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
/// Must be valid `Binding<bool>`
#[no_mangle]
pub unsafe extern "C" fn waterui_set_binding_bool(binding: *const (), bool: bool) {
    let binding = ManuallyDrop::new(Binding::from_raw(binding as *const bool));
    binding.set(bool);
}

/// # Safety
/// Must be valid `Binding<bool>`
#[no_mangle]
pub unsafe extern "C" fn waterui_subscribe_binding_bool(
    binding: *const (),
    subscriber: Subscriber,
) {
    let binding = ManuallyDrop::new(Binding::from_raw(binding as *const bool));
    binding.subscribe(subscriber);
}

macro_rules! impl_component{
    ($(($ident:ident,$ty:tt)),*) => {
        $(
            /// # Safety
            /// `EventObject` must be valid
            #[no_mangle]
            pub unsafe extern "C" fn $ident(view: ViewObject,value:*mut $ty) -> i8{
                let mut view = view.into_ptr();

                try_unwrap_boxed_view(&mut view);

                if (*view).is::<component::$ty>(){
                    write(value,read(view as *const component::$ty).into());
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
    ($(($ident:ident,$modifier:ty,$ty:ty)),*) => {
        $(
            /// # Safety
            /// `EventObject` must be valid
            #[no_mangle]
            pub unsafe extern "C" fn $ident(view: ViewObject,value:*mut $ty) -> i8{
                let mut view = view.into_ptr();

                try_unwrap_boxed_view(&mut view);

                if (*view).is::<Modifier<$modifier>>(){
                    write(value,read(view as *const Modifier<$modifier>).into());
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
    let mut view = view.into_ptr();
    try_unwrap_boxed_view(&mut view);
    if (*view).is::<Reactive<BoxView>>() {
        let reactive = read(view as *const Reactive<BoxView>);
        reactive.into_raw() as *const ()
    } else {
        null()
    }
}

unsafe fn try_unwrap_boxed_view(view: *mut *const dyn View) {
    let mut new_view = *view;
    loop {
        if (*new_view).is::<BoxView>() {
            new_view = read(new_view as *const *const dyn View)
        } else {
            break;
        }
    }
    *view = new_view
}

/// # Safety
/// `EventObject` must be valid
#[no_mangle]
pub unsafe extern "C" fn waterui_view_to_empty(view: ViewObject) -> i8 {
    let mut view = view.into_ptr();

    try_unwrap_boxed_view(&mut view);

    if (*view).is::<()>() {
        0
    } else {
        -1
    }
}

impl_component!(
    (waterui_view_to_text, Text),
    (waterui_view_to_button, Button),
    (waterui_view_to_text_field, TextField)
);

impl_modifier!((waterui_view_to_frame_modifier, Frame, FrameModifier));

/// # Safety
/// `EventObject` must be valid
#[no_mangle]
pub unsafe extern "C" fn waterui_view_to_stack(view: ViewObject, value: *mut Stack) -> i8 {
    let mut view = view.into_ptr();

    if (*view).is::<component::VStack>() {
        *value = read(view as *const component::VStack).into();
        return 0;
    }

    if (*view).is::<component::HStack>() {
        *value = read(view as *const component::HStack).into();
        return 0;
    }

    try_unwrap_boxed_view(&mut view);
    -1
}

/// # Safety
/// `EventObject` must be valid
#[no_mangle]
pub unsafe extern "C" fn waterui_call_view(view: ViewObject, env: *const ()) -> ViewObject {
    view.into_box()
        .body(Environment::from_raw(env as *const EnvironmentBuilder))
        .into()
}

#[repr(C)]
pub struct Text {
    text: *const (),
    selectable: *const (),
}

#[repr(C)]
pub struct FrameModifier {
    frame: Frame,
    view: ViewObject,
}

impl From<Modifier<Frame>> for FrameModifier {
    fn from(value: Modifier<Frame>) -> Self {
        Self {
            frame: value.modifier,
            view: value.content.into(),
        }
    }
}

impl_array!(Views, BoxView, ViewObject);
#[repr(C)]
pub struct Stack {
    mode: StackMode,
    contents: Views,
}

#[repr(C)]
pub enum StackMode {
    Vertical,
    Horizonal,
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

impl From<component::VStack> for Stack {
    fn from(value: component::VStack) -> Self {
        Self {
            mode: StackMode::Vertical,
            contents: value.contents.into(),
        }
    }
}

impl From<component::HStack> for Stack {
    fn from(value: component::HStack) -> Self {
        Self {
            mode: StackMode::Horizonal,
            contents: value.contents.into(),
        }
    }
}

extern "C" {
    pub fn waterui_create_window(title: Buf, content: ViewObject) -> usize;
    pub fn waterui_window_closeable(id: usize, is: bool);
    pub fn waterui_close_window(id: usize);
    pub fn waterui_main() -> ViewObject;
    pub fn waterui_init_environment() -> *const ();
}
