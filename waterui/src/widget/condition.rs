use crate::{view, view::IntoView, Binding, BoxView, View};

#[derive(Debug)]
#[view(use_core)]
pub struct Condition<ContentBuilder, OrBuilder> {
    #[state]
    condition: bool,
    content: ContentBuilder,
    or: OrBuilder,
}

impl<ContentBuilder, Content> Condition<ContentBuilder, fn()>
where
    ContentBuilder: Fn() -> Content,
    Content: IntoView,
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
    Content: IntoView,
    Or: IntoView,
{
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
    ContentBuilder: Fn() -> Content,
    OrBuilder: Fn() -> Or,
    Content: IntoView,
    Or: IntoView,
{
    fn view(&self) -> BoxView {
        if *self.condition.get() {
            (self.content)().into_boxed_view()
        } else {
            (self.or)().into_boxed_view()
        }
    }
}

pub fn when<ContentBuilder, Content>(
    condition: impl Into<Binding<bool>>,
    content: ContentBuilder,
) -> Condition<ContentBuilder, fn()>
where
    ContentBuilder: Fn() -> Content,
    Content: IntoView,
{
    Condition::new(condition, content)
}
