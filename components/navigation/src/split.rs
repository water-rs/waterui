use waterui_core::dynamic::Dynamic;
use waterui_core::handler::{AnyViewBuilder, BoxedAction, ViewBuilder, boxed_action_with_env};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Binding, View, raw_view};

use crate::NavigationView;

fn empty_placeholder() {}

/// Internal native split container consumed by platform backends.
#[must_use]
pub struct NavigationSplitLayout {
    /// Sidebar content.
    pub sidebar: AnyViewBuilder<AnyView>,
    /// Placeholder content used on regular-width layouts without selection.
    pub placeholder: AnyViewBuilder<AnyView>,
    /// Active detail view when a selection exists.
    pub detail: Option<NavigationView>,
    /// Preferred sidebar width on regular-width layouts.
    pub sidebar_width: f32,
    /// Action that clears the current selection on compact layouts.
    pub clear_selection: BoxedAction<()>,
}

waterui_core::impl_debug!(NavigationSplitLayout);
raw_view!(NavigationSplitLayout, StretchAxis::Both);

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
    T: Clone + 'static,
    Sidebar: ViewBuilder,
    Sidebar::Output: View,
    Detail: 'static + Clone + Fn(T) -> NavigationView,
    Placeholder: ViewBuilder,
    Placeholder::Output: View,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let selection = self.selection;
        let detail = self.detail;
        let sidebar_width = self.sidebar_width;
        let sidebar = AnyViewBuilder::new(self.sidebar).erase();
        let placeholder = AnyViewBuilder::new(self.placeholder).erase();

        Dynamic::watch(selection.clone(), move |selected| {
            let clear_selection = boxed_action_with_env({
                let selection = selection.clone();
                move |_env| {
                    selection.set(None);
                }
            });

            NavigationSplitLayout {
                sidebar: sidebar.clone(),
                placeholder: placeholder.clone(),
                detail: selected.map(|value| detail(value)),
                sidebar_width,
                clear_selection,
            }
        })
    }
}
