//! Split-view navigation primitives.

use alloc::rc::Rc;
use core::fmt::{self, Debug};
use core::marker::PhantomData;

use nami::{Binding, Computed, SignalExt as _};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::id::{Id, Mapping};
use waterui_core::layout::StretchAxis;
use waterui_core::{
    AnyView, Environment, IntoSignal, Native, NativeView, View, contract_violation,
};

pub use crate::tab::SlotBuilder;
use crate::transition::TransitionKeyRegistry;
use crate::{NavigationToolbarItems, NavigationView, ResolvedNavigationView};

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
    /// # Contract violation
    ///
    /// Every width must be finite, positive, and `min <= ideal <= max`.
    /// Otherwise a debug build panics; a release build logs it and returns
    /// [`ColumnWidth::default`].
    #[must_use]
    pub fn new(min: f32, ideal: f32, max: f32) -> Self {
        if !(min.is_finite() && ideal.is_finite() && max.is_finite()) {
            contract_violation!(
                "navigation split column widths must be finite; the default column width is used"
            );
            return Self::default();
        }
        if !(min > 0.0 && min <= ideal && ideal <= max) {
            contract_violation!(
                "navigation split column widths must satisfy 0 < min <= ideal <= max; the default column width is used"
            );
            return Self::default();
        }
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
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each column visibility variant by variant; a new column visibility must fail their build instead of falling into a wildcard arm"
)]
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

// A constant visibility is a signal too, so `column_visibility` takes a plain
// value as readily as a binding.
nami::impl_constant!(NavigationSplitColumnVisibility);

/// Native split presentation selected by a public split style.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each split style variant by variant; a new split style must fail their build instead of falling into a wildcard arm"
)]
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

/// Selected-column builder used by native split navigation backends, keeping
/// each built destination's content and toolbar in the backend's child slot `S`.
#[derive(Clone)]
pub struct NavigationSplitDetailBuilder<S>(Rc<dyn Fn(Id) -> ResolvedNavigationView<S>>);

impl<S> core::fmt::Debug for NavigationSplitDetailBuilder<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NavigationSplitDetailBuilder")
    }
}

impl<S: ChildSlot> NavigationSplitDetailBuilder<S> {
    fn mapped<T, D, Tb>(
        mapping: Mapping<T>,
        detail: impl Fn(T) -> NavigationView<D, Tb> + 'static,
    ) -> Self
    where
        T: Clone + Ord + 'static,
        D: View,
        Tb: NavigationToolbarItems,
    {
        Self(Rc::new(move |selected| {
            detail(
                mapping
                    .to_data(selected)
                    .expect("NavigationSplitView selected id must exist in mapping"),
            )
            .into_resolved::<S>()
        }))
    }
}

impl<S> NavigationSplitDetailBuilder<S> {
    /// Builds a column view for a selected identifier.
    pub fn build(&self, selected: Id) -> ResolvedNavigationView<S> {
        (self.0)(selected)
    }
}

impl<S: 'static> NavigationSplitDetailBuilder<S> {
    /// Maps each built column's content and toolbar through `f` into slot `T`.
    #[must_use]
    pub fn map_slot<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> NavigationSplitDetailBuilder<T> {
        NavigationSplitDetailBuilder(Rc::new(move |selected| {
            MapSlots::map_slots(self.build(selected), f.clone())
        }))
    }
}

fn stable_slot_builder<V, S>(view: V) -> SlotBuilder<S>
where
    V: View + Clone,
    S: ChildSlot,
{
    SlotBuilder::new(move || S::from_view(view.clone()))
}

/// Internal native split container consumed by platform backends, with its
/// columns in the backend's child slot `S`.
#[must_use]
pub struct NavigationSplitLayout<S> {
    pub(crate) sidebar: SlotBuilder<S>,
    pub(crate) placeholder: SlotBuilder<S>,
    pub(crate) primary_selection: Binding<Option<Id>>,
    pub(crate) content: Option<NavigationSplitDetailBuilder<S>>,
    pub(crate) secondary_selection: Option<Binding<Option<Id>>>,
    pub(crate) detail: NavigationSplitDetailBuilder<S>,
    pub(crate) column_visibility: Computed<NavigationSplitColumnVisibility>,
    pub(crate) sidebar_width: ColumnWidth,
    pub(crate) style: NativeNavigationSplitStyle,
}

impl<S> core::fmt::Debug for NavigationSplitLayout<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigationSplitLayout")
            .field("column_visibility", &self.column_visibility)
            .field("sidebar_width", &self.sidebar_width)
            .field("style", &self.style)
            .finish_non_exhaustive()
    }
}

impl<S: 'static> NativeView for NavigationSplitLayout<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl<S: 'static> NavigationSplitLayout<S> {
    /// Maps every column builder's slot through `f`.
    pub fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> NavigationSplitLayout<T> {
        let Self {
            sidebar,
            placeholder,
            primary_selection,
            content,
            secondary_selection,
            detail,
            column_visibility,
            sidebar_width,
            style,
        } = self;
        NavigationSplitLayout {
            sidebar: sidebar.map_slot(f.clone()),
            placeholder: placeholder.map_slot(f.clone()),
            primary_selection,
            content: content.map(|builder| builder.map_slot(f.clone())),
            secondary_selection,
            detail: detail.map_slot(f),
            column_visibility,
            sidebar_width,
            style,
        }
    }
}

impl<S: 'static> MapSlots<S> for NavigationSplitLayout<S> {
    type With<T: 'static> = NavigationSplitLayout<T>;

    fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> NavigationSplitLayout<T> {
        Self::map_slots(self, f)
    }
}

impl<S> NavigationSplitLayout<S> {
    /// Returns the stable primary-column builder.
    #[doc(hidden)]
    #[must_use]
    pub const fn primary_builder(&self) -> &SlotBuilder<S> {
        &self.sidebar
    }

    /// Returns the empty-detail placeholder builder.
    #[doc(hidden)]
    #[must_use]
    pub const fn placeholder_builder(&self) -> &SlotBuilder<S> {
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
    pub const fn content_builder(&self) -> Option<&NavigationSplitDetailBuilder<S>> {
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
    pub const fn detail_builder(&self) -> &NavigationSplitDetailBuilder<S> {
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
    #[expect(
        clippy::type_complexity,
        reason = "backends consume the complete split semantic model"
    )]
    pub fn into_parts(
        self,
    ) -> (
        SlotBuilder<S>,
        SlotBuilder<S>,
        Binding<Option<Id>>,
        Option<NavigationSplitDetailBuilder<S>>,
        Option<Binding<Option<Id>>>,
        NavigationSplitDetailBuilder<S>,
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

/// Selected-column mapping paired with its typed builder closure.
#[doc(hidden)]
pub struct MappedSplitColumn<T, F, V, Tb = ()> {
    mapping: Mapping<T>,
    build: F,
    view: PhantomData<fn() -> (V, Tb)>,
}

impl<T, F, V, Tb> Debug for MappedSplitColumn<T, F, V, Tb> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MappedSplitColumn")
    }
}

impl<T, F, V, Tb> MappedSplitColumn<T, F, V, Tb>
where
    T: Clone + Ord + 'static,
    F: Fn(T) -> NavigationView<V, Tb> + 'static,
    V: View,
    Tb: NavigationToolbarItems,
{
    fn into_builder<S: ChildSlot>(self) -> NavigationSplitDetailBuilder<S> {
        NavigationSplitDetailBuilder::mapped(self.mapping, self.build)
    }
}

/// Optional middle-column builder carried by [`NavigationSplitView`].
#[doc(hidden)]
pub trait IntoOptionalSplitDetailBuilder: 'static {
    /// Converts the middle-column specification into an optional builder in slot `S`.
    fn into_detail_builder<S: ChildSlot>(self) -> Option<NavigationSplitDetailBuilder<S>>;
}

impl IntoOptionalSplitDetailBuilder for () {
    fn into_detail_builder<S: ChildSlot>(self) -> Option<NavigationSplitDetailBuilder<S>> {
        None
    }
}

impl<T, F, V, Tb> IntoOptionalSplitDetailBuilder for MappedSplitColumn<T, F, V, Tb>
where
    T: Clone + Ord + 'static,
    F: Fn(T) -> NavigationView<V, Tb> + 'static,
    V: View,
    Tb: NavigationToolbarItems,
{
    fn into_detail_builder<S: ChildSlot>(self) -> Option<NavigationSplitDetailBuilder<S>> {
        Some(self.into_builder::<S>())
    }
}

/// Trailing detail-column builder carried by [`NavigationSplitView`].
#[doc(hidden)]
pub trait IntoSplitDetailBuilder: 'static {
    /// Converts the detail-column specification into a builder in slot `S`.
    fn into_detail_builder<S: ChildSlot>(self) -> NavigationSplitDetailBuilder<S>;
}

impl<T, F, V, Tb> IntoSplitDetailBuilder for MappedSplitColumn<T, F, V, Tb>
where
    T: Clone + Ord + 'static,
    F: Fn(T) -> NavigationView<V, Tb> + 'static,
    V: View,
    Tb: NavigationToolbarItems,
{
    fn into_detail_builder<S: ChildSlot>(self) -> NavigationSplitDetailBuilder<S> {
        self.into_builder::<S>()
    }
}

/// Adaptive two- or three-column native navigation container.
#[must_use]
pub struct NavigationSplitView<Sidebar, Placeholder, Content, Detail> {
    sidebar: Sidebar,
    placeholder: Placeholder,
    primary_selection: Binding<Option<Id>>,
    content: Content,
    secondary_selection: Option<Binding<Option<Id>>>,
    detail: Detail,
    column_visibility: Computed<NavigationSplitColumnVisibility>,
    sidebar_width: ColumnWidth,
    style: NativeNavigationSplitStyle,
}

impl<Sidebar, Placeholder, Content, Detail> core::fmt::Debug
    for NavigationSplitView<Sidebar, Placeholder, Content, Detail>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigationSplitView")
            .field("column_visibility", &self.column_visibility)
            .field("sidebar_width", &self.sidebar_width)
            .field("style", &self.style)
            .finish_non_exhaustive()
    }
}

/// Three-column split view type returned by [`NavigationSplitView::three_column`].
#[doc(hidden)]
pub type ThreeColumnSplitView<Sidebar, S, C, Content, Detail, CV, CTb, DV, DTb> =
    NavigationSplitView<
        Sidebar,
        (),
        MappedSplitColumn<S, Content, CV, CTb>,
        MappedSplitColumn<C, Detail, DV, DTb>,
    >;

impl NavigationSplitView<(), (), (), ()> {
    /// Creates a two-column split driven by a caller-owned optional selection.
    ///
    /// At compact widths exactly one column is visible: the detail while
    /// `selection` is `Some`, and the sidebar while it is `None`. The platform
    /// back control (including a completed back gesture) writes `None` into
    /// the selection. Start with `None` to open on the sidebar; the binding is
    /// the single source of truth for the visible compact column.
    pub fn new<T, Sidebar, Detail, D, DTb>(
        selection: &Binding<Option<T>>,
        sidebar: Sidebar,
        detail: Detail,
    ) -> NavigationSplitView<Sidebar, (), (), MappedSplitColumn<T, Detail, D, DTb>>
    where
        T: Clone + Ord + 'static,
        Sidebar: View + Clone,
        Detail: 'static + Fn(T) -> NavigationView<D, DTb>,
        D: View,
        DTb: NavigationToolbarItems,
    {
        let mapping = Mapping::new();
        NavigationSplitView {
            sidebar,
            placeholder: (),
            primary_selection: mapping.optional_binding(selection),
            content: (),
            secondary_selection: None,
            detail: MappedSplitColumn {
                mapping,
                build: detail,
                view: PhantomData,
            },
            column_visibility: Computed::constant(NavigationSplitColumnVisibility::Automatic),
            sidebar_width: ColumnWidth::default(),
            style: NativeNavigationSplitStyle::Automatic,
        }
    }

    /// Creates a three-column split with independently selected content and detail columns.
    pub fn three_column<S, C, Sidebar, Content, Detail, CV, CTb, DV, DTb>(
        sidebar_selection: &Binding<Option<S>>,
        content_selection: &Binding<Option<C>>,
        sidebar: Sidebar,
        content: Content,
        detail: Detail,
    ) -> ThreeColumnSplitView<Sidebar, S, C, Content, Detail, CV, CTb, DV, DTb>
    where
        S: Clone + Ord + 'static,
        C: Clone + Ord + 'static,
        Sidebar: View + Clone,
        Content: 'static + Fn(S) -> NavigationView<CV, CTb>,
        Detail: 'static + Fn(C) -> NavigationView<DV, DTb>,
        CV: View,
        CTb: NavigationToolbarItems,
        DV: View,
        DTb: NavigationToolbarItems,
    {
        let sidebar_mapping = Mapping::new();
        let content_mapping = Mapping::new();
        NavigationSplitView {
            sidebar,
            placeholder: (),
            primary_selection: sidebar_mapping.optional_binding(sidebar_selection),
            content: MappedSplitColumn {
                mapping: sidebar_mapping,
                build: content,
                view: PhantomData,
            },
            secondary_selection: Some(content_mapping.optional_binding(content_selection)),
            detail: MappedSplitColumn {
                mapping: content_mapping,
                build: detail,
                view: PhantomData,
            },
            column_visibility: Computed::constant(NavigationSplitColumnVisibility::Automatic),
            sidebar_width: ColumnWidth::default(),
            style: NativeNavigationSplitStyle::Automatic,
        }
    }
}

impl<Sidebar, Placeholder, Content, Detail>
    NavigationSplitView<Sidebar, Placeholder, Content, Detail>
{
    /// Sets the placeholder displayed when the active selection has no destination.
    pub fn placeholder<P: View + Clone>(
        self,
        placeholder: P,
    ) -> NavigationSplitView<Sidebar, P, Content, Detail> {
        NavigationSplitView {
            sidebar: self.sidebar,
            placeholder,
            primary_selection: self.primary_selection,
            content: self.content,
            secondary_selection: self.secondary_selection,
            detail: self.detail,
            column_visibility: self.column_visibility,
            sidebar_width: self.sidebar_width,
            style: self.style,
        }
    }

    /// Sets reactive native column visibility.
    pub fn column_visibility(
        mut self,
        visibility: impl IntoSignal<NavigationSplitColumnVisibility> + 'static,
    ) -> Self {
        self.column_visibility = visibility.into_signal().computed();
        self
    }

    /// Sets native sidebar resize constraints.
    pub const fn sidebar_width(mut self, width: ColumnWidth) -> Self {
        self.sidebar_width = width;
        self
    }

    /// Sets the native split presentation style.
    pub fn style(mut self, style: impl NavigationSplitStyle) -> Self {
        self.style = style.into_native();
        self
    }
}

impl<Sidebar, Placeholder, Content, Detail>
    NavigationSplitView<Sidebar, Placeholder, Content, Detail>
where
    Sidebar: View + Clone,
    Placeholder: View + Clone,
    Content: IntoOptionalSplitDetailBuilder,
    Detail: IntoSplitDetailBuilder,
{
    /// Converts this split container into [`NavigationSplitLayout`] with every
    /// column in the child slot `S`.
    #[doc(hidden)]
    pub fn into_layout<S: ChildSlot>(self) -> NavigationSplitLayout<S> {
        NavigationSplitLayout {
            sidebar: stable_slot_builder(self.sidebar),
            placeholder: stable_slot_builder(self.placeholder),
            primary_selection: self.primary_selection,
            content: self.content.into_detail_builder::<S>(),
            secondary_selection: self.secondary_selection,
            detail: self.detail.into_detail_builder::<S>(),
            column_visibility: self.column_visibility,
            sidebar_width: self.sidebar_width,
            style: self.style,
        }
    }
}

impl<Sidebar, Placeholder, Content, Detail> View
    for NavigationSplitView<Sidebar, Placeholder, Content, Detail>
where
    Sidebar: View + Clone,
    Placeholder: View + Clone,
    Content: IntoOptionalSplitDetailBuilder,
    Detail: IntoSplitDetailBuilder,
{
    /// The erased form, for code that expands views by hand.
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.into_layout::<AnyView>())
    }

    /// Reaches the backend with every column built in the backend's child slot.
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let split_env = TransitionKeyRegistry::scoped(env);
        backend.native(self.into_layout::<B::Slot>(), &split_env, cx)
    }
}

#[cfg(test)]
mod tests {
    use nami::{Binding, Signal as _};
    use waterui_core::{AnyView, Str};

    use super::{ColumnWidth, NavigationSplitView};
    use crate::NavigationView;

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(
            expected = "navigation split column widths must satisfy 0 < min <= ideal <= max"
        )
    )]
    fn unordered_column_widths_fall_back_to_the_default() {
        // Release outcome: the default column width is used.
        assert_eq!(
            ColumnWidth::new(300.0, 200.0, 400.0),
            ColumnWidth::default()
        );
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "navigation split column widths must be finite")
    )]
    fn non_finite_column_widths_fall_back_to_the_default() {
        assert_eq!(
            ColumnWidth::new(f32::NAN, 200.0, 400.0),
            ColumnWidth::default()
        );
    }

    #[test]
    fn into_layout_preserves_concrete_column_view_types() {
        let primary = Binding::new(Some(1_i32));
        let secondary = Binding::new(Some(2_i32));
        let split = NavigationSplitView::three_column(
            &primary,
            &secondary,
            Str::from("sidebar"),
            |id| {
                NavigationView::new(
                    "Content",
                    Str::from(if id == 1 { "content" } else { "other" }),
                )
            },
            |id| {
                NavigationView::new(
                    "Detail",
                    Str::from(if id == 2 { "detail" } else { "other" }),
                )
            },
        )
        .placeholder(Str::from("placeholder"));

        let layout = split.into_layout::<AnyView>();
        let primary_id = layout.primary_selection().snapshot().expect("primary id");
        let secondary_id = layout
            .secondary_selection()
            .expect("secondary selection")
            .snapshot()
            .expect("secondary id");

        assert!(layout.primary_builder().build().downcast::<Str>().is_ok());
        assert!(
            layout
                .placeholder_builder()
                .build()
                .downcast::<Str>()
                .is_ok()
        );
        assert!(
            layout
                .content_builder()
                .expect("content builder")
                .build(primary_id)
                .content
                .downcast::<Str>()
                .is_ok()
        );
        assert!(
            layout
                .detail_builder()
                .build(secondary_id)
                .content
                .downcast::<Str>()
                .is_ok()
        );
    }
}
