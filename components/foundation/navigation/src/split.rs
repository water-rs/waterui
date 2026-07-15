//! Split-view navigation primitives.

use alloc::rc::Rc;

use nami::{Binding, Computed, SignalExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::{Id, Mapping};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, IntoSignal, View, raw_view};

use crate::NavigationView;

const fn empty_placeholder() {}

/// Width constraints for a resizable navigation split column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnWidth {
    min: f32,
    ideal: f32,
    max: f32,
}

impl ColumnWidth {
    /// Creates native column width constraints.
    ///
    /// # Panics
    ///
    /// Panics unless every width is finite, positive, and `min <= ideal <= max`.
    #[must_use]
    pub fn new(min: f32, ideal: f32, max: f32) -> Self {
        assert!(
            min.is_finite() && ideal.is_finite() && max.is_finite(),
            "navigation split column widths must be finite"
        );
        assert!(
            min > 0.0 && min <= ideal && ideal <= max,
            "navigation split column widths must satisfy 0 < min <= ideal <= max"
        );
        Self { min, ideal, max }
    }

    /// Returns the minimum width.
    #[must_use]
    pub const fn min(self) -> f32 {
        self.min
    }

    /// Returns the preferred width.
    #[must_use]
    pub const fn ideal(self) -> f32 {
        self.ideal
    }

    /// Returns the maximum width.
    #[must_use]
    pub const fn max(self) -> f32 {
        self.max
    }
}

impl Default for ColumnWidth {
    fn default() -> Self {
        Self::new(240.0, 320.0, 480.0)
    }
}

/// Visible columns requested from the native adaptive split container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum NavigationSplitColumnVisibility {
    /// The platform chooses visibility from the current window configuration.
    #[default]
    Automatic = 0,
    /// Show all columns that fit.
    All = 1,
    /// Prefer the two trailing columns.
    DoubleColumn = 2,
    /// Show only the detail column.
    DetailOnly = 3,
}

/// Native split presentation selected by a public split style.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NativeNavigationSplitStyle {
    /// Platform default adaptive presentation.
    Automatic = 0,
    /// Keep the leading columns visually prominent where supported.
    Balanced = 1,
    /// Give the detail column visual priority where supported.
    ProminentDetail = 2,
}

/// Extensible public style contract for native split containers.
pub trait NavigationSplitStyle: 'static {
    /// Resolves the native split capability.
    #[doc(hidden)]
    fn into_native(self) -> NativeNavigationSplitStyle;
}

/// Built-in navigation split styles.
pub mod split_style {
    use super::{NativeNavigationSplitStyle, NavigationSplitStyle};

    /// Platform-default adaptive split style.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Automatic;

    impl NavigationSplitStyle for Automatic {
        fn into_native(self) -> NativeNavigationSplitStyle {
            NativeNavigationSplitStyle::Automatic
        }
    }

    /// Balanced leading and detail columns.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Balanced;

    impl NavigationSplitStyle for Balanced {
        fn into_native(self) -> NativeNavigationSplitStyle {
            NativeNavigationSplitStyle::Balanced
        }
    }

    /// Detail-prominent split style.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct ProminentDetail;

    impl NavigationSplitStyle for ProminentDetail {
        fn into_native(self) -> NativeNavigationSplitStyle {
            NativeNavigationSplitStyle::ProminentDetail
        }
    }

    /// Returns the platform-default adaptive split style.
    #[must_use]
    pub const fn automatic() -> Automatic {
        Automatic
    }

    /// Returns the balanced split style.
    #[must_use]
    pub const fn balanced() -> Balanced {
        Balanced
    }

    /// Returns the detail-prominent split style.
    #[must_use]
    pub const fn prominent_detail() -> ProminentDetail {
        ProminentDetail
    }
}

/// Type-erased selected-column builder used by native split navigation backends.
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

    /// Builds a column view for a selected identifier.
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
    pub(crate) primary_selection: Binding<Option<Id>>,
    pub(crate) content: Option<NavigationSplitDetailBuilder>,
    pub(crate) secondary_selection: Option<Binding<Option<Id>>>,
    pub(crate) detail: NavigationSplitDetailBuilder,
    pub(crate) column_visibility: Computed<NavigationSplitColumnVisibility>,
    pub(crate) sidebar_width: ColumnWidth,
    pub(crate) style: NativeNavigationSplitStyle,
}

waterui_core::impl_debug!(NavigationSplitLayout);
raw_view!(NavigationSplitLayout, StretchAxis::Both);

impl NavigationSplitLayout {
    /// Returns the stable primary-column builder.
    #[doc(hidden)]
    #[must_use]
    pub const fn primary_builder(&self) -> &AnyViewBuilder<AnyView> {
        &self.sidebar
    }

    /// Returns the empty-detail placeholder builder.
    #[doc(hidden)]
    #[must_use]
    pub const fn placeholder_builder(&self) -> &AnyViewBuilder<AnyView> {
        &self.placeholder
    }

    /// Returns the primary-column selection.
    #[doc(hidden)]
    #[must_use]
    pub const fn primary_selection(&self) -> &Binding<Option<Id>> {
        &self.primary_selection
    }

    /// Returns the optional middle-column builder.
    #[doc(hidden)]
    #[must_use]
    pub const fn content_builder(&self) -> Option<&NavigationSplitDetailBuilder> {
        self.content.as_ref()
    }

    /// Returns the optional middle-column selection.
    #[doc(hidden)]
    #[must_use]
    pub const fn secondary_selection(&self) -> Option<&Binding<Option<Id>>> {
        self.secondary_selection.as_ref()
    }

    /// Returns the detail-column builder.
    #[doc(hidden)]
    #[must_use]
    pub const fn detail_builder(&self) -> &NavigationSplitDetailBuilder {
        &self.detail
    }

    /// Returns the reactive native column visibility.
    #[doc(hidden)]
    #[must_use]
    pub const fn column_visibility_signal(&self) -> &Computed<NavigationSplitColumnVisibility> {
        &self.column_visibility
    }

    /// Returns the primary-column width constraints.
    #[doc(hidden)]
    #[must_use]
    pub const fn sidebar_width_constraints(&self) -> ColumnWidth {
        self.sidebar_width
    }

    /// Returns the resolved native split style.
    #[doc(hidden)]
    #[must_use]
    pub const fn native_style(&self) -> NativeNavigationSplitStyle {
        self.style
    }

    #[doc(hidden)]
    #[must_use]
    #[allow(
        clippy::type_complexity,
        reason = "FFI consumes the complete split semantic model"
    )]
    pub fn into_parts(
        self,
    ) -> (
        AnyViewBuilder<AnyView>,
        AnyViewBuilder<AnyView>,
        Binding<Option<Id>>,
        Option<NavigationSplitDetailBuilder>,
        Option<Binding<Option<Id>>>,
        NavigationSplitDetailBuilder,
        Computed<NavigationSplitColumnVisibility>,
        ColumnWidth,
        NativeNavigationSplitStyle,
    ) {
        (
            self.sidebar,
            self.placeholder,
            self.primary_selection,
            self.content,
            self.secondary_selection,
            self.detail,
            self.column_visibility,
            self.sidebar_width,
            self.style,
        )
    }
}

/// Adaptive two- or three-column native navigation container.
#[must_use]
pub struct NavigationSplitView {
    layout: NavigationSplitLayout,
}

waterui_core::impl_debug!(NavigationSplitView);

impl NavigationSplitView {
    /// Creates a two-column split driven by a caller-owned optional selection.
    pub fn new<T, Sidebar, Detail>(
        selection: &Binding<Option<T>>,
        sidebar: Sidebar,
        detail: Detail,
    ) -> Self
    where
        T: Clone + Ord + 'static,
        Sidebar: View + Clone,
        Detail: 'static + Fn(T) -> NavigationView,
    {
        let mapping = Mapping::new();
        Self {
            layout: NavigationSplitLayout {
                sidebar: stable_view_builder(sidebar),
                placeholder: stable_view_builder(empty_placeholder),
                primary_selection: mapping.optional_binding(selection),
                content: None,
                secondary_selection: None,
                detail: NavigationSplitDetailBuilder::mapped(mapping, detail),
                column_visibility: Computed::constant(NavigationSplitColumnVisibility::Automatic),
                sidebar_width: ColumnWidth::default(),
                style: NativeNavigationSplitStyle::Automatic,
            },
        }
    }

    /// Creates a three-column split with independently selected content and detail columns.
    pub fn three_column<S, C, Sidebar, Content, Detail>(
        sidebar_selection: &Binding<Option<S>>,
        content_selection: &Binding<Option<C>>,
        sidebar: Sidebar,
        content: Content,
        detail: Detail,
    ) -> Self
    where
        S: Clone + Ord + 'static,
        C: Clone + Ord + 'static,
        Sidebar: View + Clone,
        Content: 'static + Fn(S) -> NavigationView,
        Detail: 'static + Fn(C) -> NavigationView,
    {
        let sidebar_mapping = Mapping::new();
        let content_mapping = Mapping::new();
        Self {
            layout: NavigationSplitLayout {
                sidebar: stable_view_builder(sidebar),
                placeholder: stable_view_builder(empty_placeholder),
                primary_selection: sidebar_mapping.optional_binding(sidebar_selection),
                content: Some(NavigationSplitDetailBuilder::mapped(
                    sidebar_mapping,
                    content,
                )),
                secondary_selection: Some(content_mapping.optional_binding(content_selection)),
                detail: NavigationSplitDetailBuilder::mapped(content_mapping, detail),
                column_visibility: Computed::constant(NavigationSplitColumnVisibility::Automatic),
                sidebar_width: ColumnWidth::default(),
                style: NativeNavigationSplitStyle::Automatic,
            },
        }
    }

    /// Sets the placeholder displayed when the active selection has no destination.
    pub fn placeholder(mut self, placeholder: impl View + Clone) -> Self {
        self.layout.placeholder = stable_view_builder(placeholder);
        self
    }

    /// Sets reactive native column visibility.
    pub fn column_visibility(
        mut self,
        visibility: impl IntoSignal<NavigationSplitColumnVisibility> + 'static,
    ) -> Self {
        self.layout.column_visibility = visibility.into_signal().computed();
        self
    }

    /// Sets native sidebar resize constraints.
    pub const fn sidebar_width(mut self, width: ColumnWidth) -> Self {
        self.layout.sidebar_width = width;
        self
    }

    /// Sets the native split presentation style.
    pub fn style(mut self, style: impl NavigationSplitStyle) -> Self {
        self.layout.style = style.into_native();
        self
    }
}

impl View for NavigationSplitView {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.layout
    }
}
