use waterui_core::{Binding, BoxView, View};

use crate::{view, ViewExt};

#[derive(Debug)]
#[view(use_core)]
pub struct Condition<ContentBuilder, OrBuilder> {
    #[state]
    condition: bool,
    content: ContentBuilder,
    or: OrBuilder,
}

impl<ContentBuilder, V> Condition<ContentBuilder, fn()>
where
    ContentBuilder: Fn() -> V,
    V: View + 'static,
{
    pub fn new(condition: impl Into<Binding<bool>>, content: ContentBuilder) -> Self {
        Self {
            condition: condition.into(),
            content,
            or: || {},
        }
    }
}

impl<Content, Or, ContentBuilder, OrBuilder> Condition<ContentBuilder, OrBuilder>
where
    ContentBuilder: Fn() -> Content,
    OrBuilder: Fn() -> Or,
    Content: View + 'static,
    Or: View + 'static,
{
    pub fn or<V>(self, or: V) -> Condition<ContentBuilder, V> {
        Condition {
            condition: self.condition,
            content: self.content,
            or,
        }
    }
}

impl<Content, Or, ContentBuilder, OrBuilder> View for Condition<ContentBuilder, OrBuilder>
where
    ContentBuilder: Fn() -> Content,
    OrBuilder: Fn() -> Or,
    Content: View + 'static,
    Or: View + 'static,
{
    fn view(&self) -> BoxView {
        if *self.condition.get() {
            (self.content)().boxed()
        } else {
            (self.or)().boxed()
        }
    }
}

pub fn when<ContentBuilder, Content>(
    condition: impl Into<Binding<bool>>,
    content: ContentBuilder,
) -> Condition<ContentBuilder, fn()>
where
    ContentBuilder: Fn() -> Content,
    Content: View + 'static,
{
    Condition::new(condition, content)
}
