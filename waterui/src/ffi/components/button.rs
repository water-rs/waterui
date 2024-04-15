use crate::ffi::{Action, ViewObject};

#[repr(C)]
pub struct Button {
    label: ViewObject,
    action: Action,
}

impl From<crate::component::Button> for Button {
    fn from(value: crate::component::Button) -> Self {
        Self {
            label: value._label.into(),
            action: value._action.into(),
        }
    }
}

impl From<Button> for crate::component::Button {
    fn from(value: Button) -> Self {
        let f: Box<dyn Fn() + Send + Sync> = value.action.into();
        Self::action(f).label(crate::component::AnyView::from(value.label))
    }
}

impl_view!(Button, waterui_view_force_as_button, waterui_view_button_id);
