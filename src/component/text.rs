use crate::{
    attributed_string::AttributedString,
    reactive::{IntoRef, Ref},
    view::Frame,
};

pub struct Text {
    frame: Frame,
    pub text: Ref<AttributedString>,
}

impl Text {
    pub fn new(text: impl IntoRef<AttributedString>) -> Self {
        Self {
            text: text.into_ref(),
            frame: Default::default(),
        }
    }
}

native_implement_with_frame!(Text);

pub fn text(text: impl IntoRef<AttributedString>) -> Text {
    Text::new(text)
}

mod test {
    use crate::reactive::reactive;

    use super::text;

    fn test() {
        let name = reactive("Lexo");
        text(&name);
        name.set("value");
    }
}
