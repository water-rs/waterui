use crate::{view::IntoView, BoxView};

#[derive(Debug)]
pub struct NavigationLink {
    view: BoxView,
}

impl NavigationLink {
    pub fn new(view: impl IntoView) -> Self {
        Self {
            view: view.into_boxed_view(),
        }
    }
}

#[derive(Debug)]
pub struct NavigationView {
    view: BoxView,
    title: String,
}

impl NavigationView {
    pub fn new(view: BoxView, title: String) -> Self {
        Self { view, title }
    }
}
