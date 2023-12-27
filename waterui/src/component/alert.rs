use crate::{reactive::IntoReactive, view::IntoView, AttributedString, BoxView, Reactive};

pub struct Alert {
    show: Binding<bool>,
    title: Reactive<String>,
    body: BoxView,
    actions: Vec<Action>,
}

struct Action {
    label: String,
    action: Box<dyn Send + Sync + Fn()>,
}

impl Alert {
    pub fn new(title: impl IntoReactive<String>, body: impl IntoView) -> Self {
        Self {
            show: Reactive::new(true),
            title: title.into_reactive(),
            body: body.into_boxed_view(),
            actions: Vec::new(),
        }
    }

    pub fn action(
        mut self,
        label: impl Into<String>,
        action: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        self.actions.push(Action {
            label: label.into(),
            action: Box::new(action),
        });
        self
    }
}
