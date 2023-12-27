use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

use crate::{reactive::IntoReactive, view::IntoView, BoxView, Reactive, View};

pub struct Each<T: 'static, IdBuilder, ContentBuilder> {
    data: Reactive<Vec<T>>,
    id_builder: IdBuilder,
    builder: ContentBuilder,
}

impl<T, ContentBuilder> Each<T, (), ContentBuilder> {
    pub fn new<Content>(data: impl IntoReactive<Vec<T>>, builder: ContentBuilder) -> Self
    where
        ContentBuilder: Fn(&T) -> Content,
        Content: IntoView,
    {
        Each {
            data: data.into_reactive(),
            id_builder: (),
            builder,
        }
    }

    pub fn id<IdBuilder>(self, builder: IdBuilder) -> Each<T, IdBuilder, ContentBuilder> {
        Each {
            data: self.data,
            id_builder: builder,
            builder: self.builder,
        }
    }
}

impl<T: Send + Sync, IdBuilder, Id, ContentBuilder, Content> View
    for Each<T, IdBuilder, ContentBuilder>
where
    IdBuilder: 'static + Send + Sync + Fn(&T) -> Id,
    Id: ToString,
    Content: IntoView,
    ContentBuilder: 'static + Send + Sync + Fn(&T) -> Content,
{
    fn body(self) -> BoxView {
        Box::new(self.data.to(move |iter| {
            let mut views = Vec::new();
            for element in iter {
                views.push(ViewWithID {
                    view: (self.builder)(&element).into_boxed_view(),
                    id: (self.id_builder)(&element).to_string(),
                });
            }
            views
        }))
    }
}

pub struct ViewWithID {
    view: BoxView,
    id: String,
}

raw_view!(Reactive<Vec<ViewWithID>>);
