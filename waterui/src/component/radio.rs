use waterui_reactive::Binding;

use crate::{View, ViewExt};

use super::AnyView;

#[derive(Debug)]
pub struct RadioGroup {
    pub _radios: Vec<Radio>,
    pub _chosen: Binding<String>,
}

#[derive(Debug)]
pub struct Radio {
    pub _value: String,
    pub _label: AnyView,
}

impl Radio {
    pub fn new(value: impl Into<String>, label: impl View + 'static) -> Self {
        Self {
            _value: value.into(),
            _label: label.anyview(),
        }
    }
}

impl RadioGroup {
    pub fn new(radios: impl Into<Vec<Radio>>, chosen: &Binding<String>) -> Self {
        Self {
            _radios: radios.into(),
            _chosen: chosen.clone(),
        }
    }
}

raw_view!(RadioGroup);
