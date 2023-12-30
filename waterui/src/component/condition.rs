use waterui_reactive::reactive::IntoReactive;

use crate::{modifier::ViewModifier, view::IntoView, Environment, Reactive, View};

use super::AnyView;

#[derive(Debug)]
pub struct Condition<ContentBuilder, OrBuilder> {
    condition: Reactive<bool>,
    content: ContentBuilder,
    or: OrBuilder,
}

impl<ContentBuilder, Content> Condition<ContentBuilder, ()>
where
    ContentBuilder: Fn() -> Content,
    Content: IntoView,
{
    pub fn new(condition: impl IntoReactive<bool>, content: ContentBuilder) -> Self {
        Self {
            condition: condition.into_reactive(),
            content,
            or: (),
        }
    }

    pub fn or<F, V>(self, or: F) -> Condition<ContentBuilder, F>
    where
        F: Fn() -> V,
        V: IntoView,
    {
        Condition {
            condition: self.condition,
            content: self.content,
            or,
        }
    }
}

impl<Content, Or, ContentBuilder, OrBuilder> View for Condition<ContentBuilder, OrBuilder>
where
    ContentBuilder: 'static + Send + Sync + Fn() -> Content,
    OrBuilder: 'static + Send + Sync + Fn() -> Or,
    Content: IntoView,
    Or: IntoView,
{
    fn body(self, _env: Environment) -> impl View {
        let output: Reactive<AnyView> = self.condition.to(move |condition| {
            if condition {
                (self.content)().into_anyview()
            } else {
                (self.or)().into_anyview()
            }
        });
        output
    }
}

impl<Content, ContentBuilder> View for Condition<ContentBuilder, ()>
where
    ContentBuilder: 'static + Send + Sync + Fn() -> Content,
    Content: IntoView,
{
    fn body(self, _env: Environment) -> impl View {
        let result: Reactive<AnyView> = self.condition.to(move |condition| {
            if condition {
                (self.content)().into_anyview()
            } else {
                ().into_anyview()
            }
        });
        result
    }
}

pub fn when<ContentBuilder, Content>(
    condition: impl IntoReactive<bool>,
    content: ContentBuilder,
) -> Condition<ContentBuilder, ()>
where
    ContentBuilder: Fn() -> Content,
    Content: IntoView,
{
    Condition::new(condition, content)
}

#[derive(Clone)]
pub struct Display(bool);
impl ViewModifier for Display {}
