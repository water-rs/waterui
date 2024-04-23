use waterui_reactive::{
    binding::BindingStr,
    compute::{ComputeStr, ComputedStr},
};

use crate::{AnyView, Compute, View, ViewExt};

use super::Text;

pub struct TextField<Label> {
    label: Label,
    value: BindingStr,
    prompt: ComputedStr,
    style: TextFieldStyle,
}

#[non_exhaustive]
pub struct RawTextField {
    pub _label: AnyView,
    pub _value: BindingStr,
    pub _prompt: ComputedStr,
    pub _style: TextFieldStyle,
}

#[derive(Debug)]
#[repr(C)]
#[non_exhaustive]
pub enum TextFieldStyle {
    Default,
    Plain,
    Outlined,
    Underlined,
}

impl Default for TextFieldStyle {
    fn default() -> Self {
        Self::Default
    }
}

raw_view!(RawTextField);

impl TextField<()> {
    pub fn new(value: &BindingStr) -> Self {
        Self {
            label: (),
            value: value.clone(),
            prompt: "".computed(),
            style: TextFieldStyle::default(),
        }
    }
}

impl<Label> TextField<Label> {
    pub fn label(self, label: impl ComputeStr) -> TextField<Text> {
        self.label_view(Text::new(label))
    }

    pub fn label_view<V: 'static>(self, label: V) -> TextField<V> {
        TextField {
            label,
            value: self.value,
            prompt: self.prompt,
            style: self.style,
        }
    }
}

impl_label!(TextField);

impl<V: View> TextField<V> {
    pub fn prompt(mut self, prompt: impl ComputeStr) -> Self {
        self.prompt = prompt.computed();
        self
    }
}

impl<V: View + 'static> View for TextField<V> {
    fn body(self, _env: crate::Environment) -> impl View {
        RawTextField {
            _label: self.label.anyview(),
            _value: self.value,
            _prompt: self.prompt,
            _style: self.style,
        }
    }
}

pub fn field(label: impl ComputeStr, value: &BindingStr) -> TextField<Text> {
    TextField::new(value).label(label)
}

mod ffi {
    use waterui_ffi::{binding::BindingStr, computed::ComputedStr, ffi_view, AnyView, IntoFFI};

    #[repr(C)]
    pub struct TextField {
        label: AnyView,
        value: BindingStr,
        prompt: ComputedStr,
    }

    impl IntoFFI for super::RawTextField {
        type FFI = TextField;
        fn into_ffi(self) -> Self::FFI {
            TextField {
                label: self._label.into_ffi(),
                value: self._value.into_ffi(),
                prompt: self._prompt.into_ffi(),
            }
        }
    }

    ffi_view!(
        super::RawTextField,
        TextField,
        waterui_view_force_as_field,
        waterui_view_field_id
    );
}
