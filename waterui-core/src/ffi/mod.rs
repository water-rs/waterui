#[macro_use]
mod array;
pub use array::Buf;
pub mod utils;

use crate::binding::SubscriberBuilderObject;
use crate::layout::Frame;
use crate::modifier::Modifier;
use crate::view::View;
use crate::{component, view::BoxView};
use std::ptr::read;
use std::ptr::write;

use self::utils::{EventObject, ViewObject};

/// # Safety
/// `EventObject` must be valid
#[no_mangle]
pub unsafe extern "C" fn waterui_call_event_object(object: EventObject) {
    (object.as_ref())();
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
    (waterui_view_to_tap_gesture, TapGesture),
    (waterui_view_to_menu, Menu)
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
pub unsafe extern "C" fn waterui_call_view(view: ViewObject) -> ViewObject {
    view.as_ref().view().into()
}

/// # Safety
/// `EventObject` must be valid
#[no_mangle]
pub unsafe extern "C" fn waterui_add_subscriber(
    view: ViewObject,
    subscriber: SubscriberBuilderObject,
) {
    view.as_ref().subscribe(subscriber);
}

#[repr(C)]
pub struct Text {
    buf: Buf,
    selectable: bool,
}

#[repr(C)]
pub struct TapGesture {
    view: ViewObject,
    event: EventObject,
}

impl From<component::TapGesture> for TapGesture {
    fn from(value: component::TapGesture) -> Self {
        Self {
            view: value.view.into(),
            event: value.event.into(),
        }
    }
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
pub struct Menu {
    label: ViewObject,
    actions: Actions,
}

#[repr(C)]
pub struct Action {
    label: Buf,
    action: EventObject,
}

impl From<component::Action> for Action {
    fn from(value: component::Action) -> Self {
        Self {
            label: value.label.into_plain().into(),
            action: value.action.into(),
        }
    }
}

impl_array!(Actions, component::Action, Action);

impl From<component::Menu> for Menu {
    fn from(value: component::Menu) -> Self {
        Self {
            label: value.label.into(),
            actions: value.actions.into(),
        }
    }
}

#[repr(C)]
pub struct Image {
    data: Buf,
}

impl From<component::RawImage> for Image {
    fn from(value: component::RawImage) -> Self {
        Self {
            data: value.data.into(),
        }
    }
}

impl From<component::Text> for Text {
    fn from(value: component::Text) -> Self {
        let text: String = value.text.into_plain();
        Self {
            buf: text.into(),
            selectable: value.selectable,
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
}
