use alloc::string::String;
use waterui_reactive::Binding;

use super::Text;
use crate::{AnyView, Compute, View, ViewExt};

#[derive(Debug)]
pub struct Toggle<Label> {
    label: Label,
    toggle: Binding<bool>,
    style: ToggleStyle,
}

#[derive(Debug)]
#[non_exhaustive]
pub struct RawToggle {
    pub _label: AnyView,
    pub _toggle: Binding<bool>,
    pub _style: ToggleStyle,
}

#[derive(Debug)]
#[repr(C)]
#[non_exhaustive]
pub enum ToggleStyle {
    Default,
    CheckBox,
    Switch,
}

impl Default for ToggleStyle {
    fn default() -> Self {
        Self::Default
    }
}
impl Toggle<Text> {
    pub fn new(label: impl Compute<Output = String>, toggle: &Binding<bool>) -> Self {
        Self::label(Text::new(label), toggle)
    }
}

impl_label!(Toggle);

impl Toggle<()> {
    pub fn binding(toggle: &Binding<bool>) -> Self {
        Self::label((), toggle)
    }
}

impl<Label: View + 'static> Toggle<Label> {
    pub fn label(label: Label, toggle: &Binding<bool>) -> Self {
        Self {
            label,
            toggle: toggle.clone(),
            style: ToggleStyle::default(),
        }
    }
}

raw_view!(RawToggle);

impl<Label: View + 'static> View for Toggle<Label> {
    fn body(self, _env: crate::Environment) -> impl View {
        RawToggle {
            _label: self.label.anyview(),
            _toggle: self.toggle,
            _style: self.style,
        }
    }
}

pub fn toggle(label: impl Compute<Output = String>, toggle: &Binding<bool>) -> Toggle<Text> {
    Toggle::new(label, toggle)
}

mod ffi {
    use waterui_ffi::{binding::BindingBool, ffi_view, AnyView, IntoFFI};

    use super::ToggleStyle;

    #[repr(C)]
    pub struct Toggle {
        label: AnyView,
        toggle: BindingBool,
        style: ToggleStyle,
    }

    impl IntoFFI for super::RawToggle {
        type FFI = Toggle;
        fn into_ffi(self) -> Self::FFI {
            Toggle {
                label: self._label.into_ffi(),
                toggle: self._toggle.into_ffi(),
                style: self._style,
            }
        }
    }

    ffi_view!(
        super::RawToggle,
        Toggle,
        waterui_view_force_as_toggle,
        waterui_view_toggle_id
    );
}
