use alloc::rc::Rc;

use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::{Id, Mapping};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Binding, View, raw_view};

use crate::NavigationView;

fn empty_placeholder() {}

#[derive(Clone)]
pub struct NavigationSplitDetailBuilder(Rc<dyn Fn(Id) -> NavigationView>);

impl core::fmt::Debug for NavigationSplitDetailBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NavigationSplitDetailBuilder")
    }
}

impl NavigationSplitDetailBuilder {
    fn mapped<T>(mapping: Mapping<T>, detail: impl Fn(T) -> NavigationView + 'static) -> Self
    where
        T: Clone + Ord + 'static,
    {
        Self(Rc::new(move |selected| {
            detail(
                mapping
                    .to_data(selected)
                    .expect("NavigationSplitView selected id must exist in mapping"),
            )
        }))
    }

    #[must_use]
    pub fn build(&self, selected: Id) -> NavigationView {
        (self.0)(selected)
    }
}

fn stable_view_builder<V>(view: V) -> AnyViewBuilder<AnyView>
where
    V: View + Clone,
{
    AnyViewBuilder::new(move || AnyView::new(view.clone()))
}

/// Internal native split container consumed by platform backends.
#[must_use]
pub struct NavigationSplitLayout {
    pub(crate) sidebar: AnyViewBuilder<AnyView>,
    pub(crate) placeholder: AnyViewBuilder<AnyView>,
    pub(crate) selection: Binding<Option<Id>>,
    pub(crate) detail: NavigationSplitDetailBuilder,
    pub(crate) sidebar_width: f32,
}

waterui_core::impl_debug!(NavigationSplitLayout);
raw_view!(NavigationSplitLayout, StretchAxis::Both);

impl NavigationSplitLayout {
    fn new<Sidebar, Placeholder>(
        sidebar: Sidebar,
        placeholder: Placeholder,
        selection: Binding<Option<Id>>,
        detail: NavigationSplitDetailBuilder,
        sidebar_width: f32,
    ) -> Self
    where
        Sidebar: View + Clone,
        Placeholder: View + Clone,
    {
        Self {
            sidebar: stable_view_builder(sidebar),
            placeholder: stable_view_builder(placeholder),
            selection,
            detail,
            sidebar_width,
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        AnyViewBuilder<AnyView>,
        AnyViewBuilder<AnyView>,
        Binding<Option<Id>>,
        NavigationSplitDetailBuilder,
        f32,
    ) {
        (
            self.sidebar,
            self.placeholder,
            self.selection,
            self.detail,
            self.sidebar_width,
        )
    }

    #[doc(hidden)]
    #[must_use]
    pub fn sidebar(&self) -> &AnyViewBuilder<AnyView> {
        &self.sidebar
    }

    #[doc(hidden)]
    #[must_use]
    pub fn placeholder(&self) -> &AnyViewBuilder<AnyView> {
        &self.placeholder
    }

    #[doc(hidden)]
    #[must_use]
    pub fn selection(&self) -> &Binding<Option<Id>> {
        &self.selection
    }

    #[doc(hidden)]
    #[must_use]
    pub fn detail_builder(&self) -> &NavigationSplitDetailBuilder {
        &self.detail
    }

    #[doc(hidden)]
    #[must_use]
    pub const fn sidebar_width(&self) -> f32 {
        self.sidebar_width
    }
}

/// Adaptive master-detail navigation container.
///
/// `NavigationSplitView` renders sidebar + detail side-by-side on regular widths and
/// collapses to sidebar/detail drill-in presentation on compact widths.
#[must_use]
pub struct NavigationSplitView<T: 'static, Sidebar, Detail, Placeholder = fn() -> ()> {
    selection: Binding<Option<T>>,
    sidebar: Sidebar,
    detail: Detail,
    placeholder: Placeholder,
    sidebar_width: f32,
}

impl<T, Sidebar, Detail> NavigationSplitView<T, Sidebar, Detail, fn() -> ()> {
    /// Creates a split view driven by a caller-owned optional selection binding.
    pub fn new(selection: &Binding<Option<T>>, sidebar: Sidebar, detail: Detail) -> Self {
        Self {
            selection: selection.clone(),
            sidebar,
            detail,
            placeholder: empty_placeholder,
            sidebar_width: 320.0,
        }
    }
}

impl<T, Sidebar, Detail, Placeholder> NavigationSplitView<T, Sidebar, Detail, Placeholder> {
    /// Sets the placeholder displayed on regular-width layouts with no selection.
    #[must_use]
    pub fn placeholder<NewPlaceholder>(
        self,
        placeholder: NewPlaceholder,
    ) -> NavigationSplitView<T, Sidebar, Detail, NewPlaceholder> {
        NavigationSplitView {
            selection: self.selection,
            sidebar: self.sidebar,
            detail: self.detail,
            placeholder,
            sidebar_width: self.sidebar_width,
        }
    }

    /// Sets the preferred sidebar width for regular-width layouts.
    #[must_use]
    pub fn sidebar_width(mut self, sidebar_width: f32) -> Self {
        assert!(
            sidebar_width.is_finite() && sidebar_width > 0.0,
            "NavigationSplitView sidebar width must be finite and positive"
        );
        self.sidebar_width = sidebar_width;
        self
    }
}

impl<T, Sidebar, Detail, Placeholder> View for NavigationSplitView<T, Sidebar, Detail, Placeholder>
where
    T: Clone + Ord + 'static,
    Sidebar: View + Clone,
    Detail: 'static + Fn(T) -> NavigationView,
    Placeholder: View + Clone,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let mapping = Mapping::new();
        let selection = mapping.optional_binding(&self.selection);
        let detail = NavigationSplitDetailBuilder::mapped(mapping, self.detail);

        NavigationSplitLayout::new(
            self.sidebar,
            self.placeholder,
            selection,
            detail,
            self.sidebar_width,
        )
    }
}
