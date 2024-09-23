use crate::Computed;
use crate::{utils::Color, ViewExt};
use alloc::{boxed::Box, vec::Vec};
use waterui_core::{components::Text, raw_view, AnyView, Environment, View};

#[derive(Debug)]
pub struct NavigationView {
    pub bar: Bar,
    pub content: AnyView,
}

#[derive(Debug, Default)]
pub struct Bar {
    pub title: Text,
    pub color: Computed<Color>,
    pub hidden: Computed<bool>,
}

pub type NavigationPath = Vec<NavigationView>;

pub struct NavigationLink {
    pub label: AnyView,
    pub view: Box<dyn Fn(Environment) -> NavigationView>,
}

impl NavigationLink {
    pub fn new(label: impl View, destination: impl 'static + Fn() -> NavigationView) -> Self {
        Self {
            label: label.anyview(),
            view: Box::new(move |_| destination()),
        }
    }
}

raw_view!(NavigationLink);

raw_view!(NavigationView);

impl NavigationView {
    pub fn new(title: impl Into<Text>, content: impl View) -> Self {
        let bar = Bar {
            title: title.into(),
            ..Default::default()
        };

        Self {
            bar,
            content: content.anyview(),
        }
    }
}

pub fn navigation(title: impl Into<Text>, view: impl View) -> NavigationView {
    NavigationView::new(title, view)
}

pub fn navigate(
    label: impl View,
    destination: impl 'static + Fn() -> NavigationView,
) -> NavigationLink {
    NavigationLink::new(label, destination)
}
