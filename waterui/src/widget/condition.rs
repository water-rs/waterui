use crate::{view::IntoView, BoxView, Reactive, View, ViewExt};

#[derive(Debug)]
pub struct Condition<ContentBuilder, OrBuilder> {
    condition: Reactive<bool>,
    content: ContentBuilder,
    or: OrBuilder,
}

impl<ContentBuilder, Content> Condition<ContentBuilder, fn()>
where
    ContentBuilder: Fn() -> Content,
    Content: IntoView,
{
    pub fn new(condition: Reactive<bool>, content: ContentBuilder) -> Self {
        Self {
            condition,
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
    ContentBuilder: 'static + Fn() -> Content,
    OrBuilder: 'static + Fn() -> Or,
    Content: IntoView,
    Or: IntoView,
{
    fn view(self) -> BoxView {
        let result: Reactive<BoxView> = self.condition.to(move |condition| {
            if *condition {
                (self.content)().into_boxed_view()
            } else {
                (self.or)().into_boxed_view()
            }
        });
        result.boxed()
    }
}
