use waterui_reactive::{binding::BindingBool, compute::ComputeStr, Binding};

use crate::{AnyView, View, ViewExt};

use super::Text;

#[derive(Debug)]
pub struct Toggle<Label> {
    label: Label,
    toggle: BindingBool,
    style: ToggleStyle,
}

#[derive(Debug)]
#[non_exhaustive]
pub struct RawToggle {
    pub _label: AnyView,
    pub _toggle: BindingBool,
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

impl_label!(Toggle);

impl Toggle<()> {
    pub fn new(toggle: &Binding<bool>) -> Self {
        Self {
            label: (),
            toggle: toggle.clone(),
            style: ToggleStyle::Default,
        }
    }
}

impl<Label: View> Toggle<Label> {
    pub fn label(self, label: impl ComputeStr) -> Toggle<Text> {
        self.label_view(Text::new(label))
    }
    pub fn label_view<V: View>(self, label: V) -> Toggle<V> {
        Toggle {
            label,
            toggle: self.toggle,
            style: self.style,
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

pub fn toggle(label: impl ComputeStr, toggle: &BindingBool) -> Toggle<Text> {
    Toggle::new(toggle).label(label)
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
