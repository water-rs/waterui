use nami::collection::Collection;
use waterui_core::{AnyView, id::Identifable};

use crate::views::{AnyViews, ForEach, Views};

// A vertical scrollable view that lazily loads its contents.
#[derive(Debug)]
pub struct Lazy {
    contents: AnyViews<AnyView>,
}

impl Lazy {
    /// Creates a new `Lazy` view with the given contents.
    pub fn new(contents: impl Views<View = AnyView> + 'static) -> Self {
        Self {
            contents: AnyViews::new(contents),
        }
    }

    /// Creates a new `Lazy` view by iterating over a collection and generating views.
    pub fn for_each<C, F>(collection: C, generator: F) -> Self
    where
        C: Collection,
        C::Item: Identifable,
        F: 'static + Fn(C::Item) -> AnyView,
    {
        Self::new(ForEach::new(collection, generator))
    }

    #[must_use] 
    pub fn into_inner(self) -> AnyViews<AnyView> {
        self.contents
    }
}
