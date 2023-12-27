use waterui_reactive::{binding::Binding, reactive::IntoReactive};

use crate::{layout::Size, Reactive};

use crate::attributed_string::{AttributedString, Font};

#[derive(Debug, Clone)]
pub struct Text {
    pub(crate) text: Reactive<AttributedString>,
    pub(crate) selectable: Binding<bool>,
}

raw_view!(Text);

impl Text {
    pub fn new(text: impl IntoReactive<AttributedString>) -> Self {
        Self {
            text: text.into_reactive(),
            selectable: Binding::new(true),
        }
    }

    pub fn disable_select(self) -> Self {
        *self.selectable.get_mut() = false;
        self
    }

    pub fn bold(mut self) -> Self {
        self.text = self.text.to(|mut string| {
            string.set_attribute(.., Font::bold());
            string
        });

        self
    }

    pub fn size(mut self, size: impl Into<Size>) -> Self {
        let size = size.into();
        self.text = self.text.to(move |mut string| {
            string.set_attribute(.., Font::new().size(size));
            string
        });
        self
    }
}
