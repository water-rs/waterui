pub mod tab;

use crate::component::Text;
use crate::Computed;
use crate::{color::Color, ViewExt};
use alloc::{boxed::Box, vec::Vec};
use waterui_core::{raw_view, AnyView, Environment, View};
#[derive(Debug)]
#[must_use]
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

#[must_use]
pub struct NavigationLink {
    pub label: AnyView,
    pub content: Box<dyn Fn(Environment) -> NavigationView>,
}

impl_debug!(NavigationLink);

impl NavigationLink {
    pub fn new(label: impl View, destination: impl 'static + Fn() -> NavigationView) -> Self {
        Self {
            label: label.anyview(),
            content: Box::new(move |_| destination()),
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
