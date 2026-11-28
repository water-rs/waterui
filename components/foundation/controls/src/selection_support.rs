//! Shared selection-theming and id-carrying `Views` helpers used by both
//! `List` and `Table`.

use alloc::rc::Rc;
use core::any::type_name;
use core::fmt;
use core::marker::PhantomData;
use core::ops::{Range, RangeBounds};

use nami::collection::CollectionChange;
use nami::watcher::Context;
use nami::{Computed, SignalExt as _};
use waterui_core::resolve::Resolvable as _;
use waterui_core::views::{ViewSnapshot, Views};
use waterui_core::{Environment, Metadata, OneOf2, View};

use waterui_color::WorkingColor;

use crate::theme::{color, install_color_signal};

/// A row's or cell's content re-themed to `SelectionForeground` while it is
/// selected.
///
/// A selected row is filled with the theme's `SelectionContainer`, so
/// foreground, muted-foreground, and accent tokens flip to
/// `SelectionForeground` reactively without rebuilding the subtree.
pub struct SelectionThemed<C> {
    content: C,
    /// Whether the row is selected; `None` when not selectable or when the list
    /// style draws an unemphasised selection fill that keeps row colours intact.
    selected: Option<Computed<bool>>,
}

impl<C> SelectionThemed<C> {
    /// Wraps `content` so its foreground tokens flip when `selected` is true.
    #[cfg(any(feature = "list", feature = "table"))]
    pub(crate) const fn new(content: C, selected: Option<Computed<bool>>) -> Self {
        Self { content, selected }
    }
}

impl<C> fmt::Debug for SelectionThemed<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectionThemed")
            .field("selectable", &self.selected.is_some())
            .finish_non_exhaustive()
    }
}

impl<C: View> View for SelectionThemed<C> {
    fn body(self, env: &Environment) -> impl View {
        let Some(selected) = self.selected else {
            return OneOf2::A(self.content);
        };
        let mut env = env.clone();
        let on_selection = color::SelectionForeground.resolve(&env).computed();
        let flip = |normal: Computed<WorkingColor>| {
            selected
                .clone()
                .zip(&normal)
                .zip(&on_selection)
                .map(
                    |((is_selected, normal), selection)| {
                        if is_selected { selection } else { normal }
                    },
                )
                .computed()
        };
        let foreground = flip(color::Foreground.resolve(&env).computed());
        let muted = flip(color::MutedForeground.resolve(&env).computed());
        let accent = flip(color::Accent.resolve(&env).computed());
        install_color_signal::<color::Foreground>(&mut env, foreground);
        install_color_signal::<color::MutedForeground>(&mut env, muted);
        install_color_signal::<color::Accent>(&mut env, accent);
        OneOf2::B(Metadata::new(self.content, env))
    }
}

/// `Views` adapter that hands each element's id to the mapping closure — the
/// row-keyed sibling of `Map`, so selection theming can derive a row's
/// selected state from the selection binding and the row's own id.
pub struct WithId<C, F> {
    contents: C,
    transform: Rc<F>,
}

impl<C, F> WithId<C, F> {
    /// Creates an id-aware mapping adapter over `contents`.
    pub fn new(contents: C, transform: F) -> Self {
        Self {
            contents,
            transform: Rc::new(transform),
        }
    }
}

/// The immutable snapshot a [`WithId`] captures: the source snapshot plus a
/// shared handle to the transform, so the id and the view a row reports come
/// from the same captured source state.
pub struct WithIdSnapshot<S, F, V> {
    contents: S,
    transform: Rc<F>,
    _marker: PhantomData<fn() -> V>,
}

impl<S: Clone, F, V> Clone for WithIdSnapshot<S, F, V> {
    fn clone(&self) -> Self {
        Self {
            contents: self.contents.clone(),
            transform: Rc::clone(&self.transform),
            _marker: PhantomData,
        }
    }
}

impl<S, F, V> fmt::Debug for WithIdSnapshot<S, F, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<S, F, V> ViewSnapshot for WithIdSnapshot<S, F, V>
where
    S: ViewSnapshot,
    F: Fn(S::Id, S::View) -> V,
    V: View,
{
    type Id = S::Id;
    type View = V;

    fn range(&self) -> Range<usize> {
        self.contents.range()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.contents.get_id(index)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        Some((self.transform)(
            self.contents.get_id(index)?,
            self.contents.get_view(index)?,
        ))
    }
}

impl<V, C, F> Views for WithId<C, F>
where
    V: View,
    C: Views,
    F: 'static + Fn(C::Id, C::View) -> V,
{
    type Id = C::Id;
    type Guard = C::Guard;
    type View = V;
    type Snapshot = WithIdSnapshot<C::Snapshot, F, V>;

    fn len(&self) -> Computed<usize> {
        self.contents.len()
    }

    fn snapshot(&self) -> Self::Snapshot {
        WithIdSnapshot {
            contents: self.contents.snapshot(),
            transform: Rc::clone(&self.transform),
            _marker: PhantomData,
        }
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        let transform = Rc::clone(&self.transform);
        self.contents.watch(range, move |ctx, change| {
            let snapshot = ctx.map(|contents| WithIdSnapshot {
                contents,
                transform: Rc::clone(&transform),
                _marker: PhantomData,
            });
            watcher(snapshot, change);
        })
    }
}
