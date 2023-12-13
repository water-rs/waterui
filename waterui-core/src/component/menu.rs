use crate::{view::IntoView, AttributedString, BoxView};

pub struct Menu {
    pub(crate) label: BoxView,
    pub(crate) actions: Vec<Action>,
}

pub struct Action {
    pub(crate) label: AttributedString,
    pub(crate) action: Box<dyn Fn()>,
}

impl Action {
    pub fn new(label: impl Into<AttributedString>, action: impl Fn() + 'static) -> Self {
        Self {
            label: label.into(),
            action: Box::new(action),
        }
    }
}

impl Menu {
    pub fn new(label: impl IntoView, actions: impl Into<Vec<Action>>) -> Self {
        Self {
            label: label.into_boxed_view(),
            actions: actions.into(),
        }
    }
}

raw_view!(Menu);
