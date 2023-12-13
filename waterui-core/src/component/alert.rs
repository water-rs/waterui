use crate::{view::IntoView, AttributedString, BoxView};

pub struct Alert {
    title: BoxView,
    body: BoxView,
    actions: Vec<Action>,
}

struct Action {
    label: AttributedString,
    action: Box<dyn Fn()>,
}

impl Alert {
    pub fn new(title: impl IntoView, body: impl IntoView) -> Self {
        Self {
            title: title.into_boxed_view(),
            body: body.into_boxed_view(),
            actions: Vec::new(),
        }
    }

    pub fn action(
        mut self,
        label: impl Into<AttributedString>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.actions.push(Action {
            label: label.into(),
            action: Box::new(action),
        });
        self
    }
}
