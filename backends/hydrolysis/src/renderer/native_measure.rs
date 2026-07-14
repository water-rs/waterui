//! Native-leaf measurement: the `HydroNativeView` measure trait, the native-view
//! type list, and the measure-path entry points the layout system uses to size
//! arbitrary sub-views.

use super::*;

/// The measure half of a native leaf view. Rendering is owned by the retained
/// [`RenderNode`](crate::renderer::tree::RenderNode) tree; this trait only sizes a
/// leaf so the layout system can measure arbitrary sub-views through it.
pub(crate) trait HydroNativeView: View + Sized + 'static {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize;
    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        _proposal: ProposalSize,
    ) -> ViewDimensions {
        ViewDimensions::new(Self::intrinsic(state, view, env))
    }
}

pub(crate) fn dimensions_for_native<V: HydroNativeView>(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> Option<ViewDimensions> {
    view.downcast_ref::<V>()
        .map(|native| V::dimensions(state, native, env, proposal))
}

macro_rules! hydro_native_view_types {
    ($macro:ident) => {
        $macro!(Native<()>);
        $macro!(Native<Spacer>);
        $macro!(Native<TextConfig>);
        $macro!(Native<FixedContainer>);
        $macro!(Native<LazyContainer>);
        $macro!(Native<ScrollView>);
        $macro!(Native<NavigationView>);
        $macro!(Native<NavigationSplitLayout>);
        $macro!(Native<NavigationStack<(), ()>>);
        $macro!(Native<Tabs>);
        $macro!(Native<BadgeConfig>);
        $macro!(Native<ListConfig>);
        $macro!(Native<TableConfig>);
        $macro!(Native<ButtonConfig>);
        $macro!(Native<ResolvedMenu>);
        $macro!(Native<ToggleConfig>);
        $macro!(Native<SliderConfig>);
        $macro!(Native<StepperConfig>);
        $macro!(Native<ProgressConfig>);
        $macro!(Native<ColorPickerConfig>);
        $macro!(Native<DatePickerConfig>);
        $macro!(Native<ResolvedTextFieldConfig>);
        $macro!(Native<SecureFieldConfig>);
        $macro!(Native<PickerConfig>);
        $macro!(Native<Dynamic>);
        $macro!(Native<SystemIcon>);
        $macro!(Native<GpuSurface>);
        $macro!(Native<SceneView>);
        $macro!(Native<ViewEffectErased>);
        $macro!(Native<Color>);
        $macro!(Native<ResolvedColor>);
        $macro!(Native<ResolvedGradient>);
        $macro!(Native<ResolvedShape>);
        $macro!(Native<ResolvedMorphShape>);
        $macro!(Native<MapConfig>);
        $macro!(WebView);
    };
}

pub(crate) fn is_hydro_native_view(view: &AnyView) -> bool {
    macro_rules! check_native_view {
        ($ty:ty) => {
            if view.downcast_ref::<$ty>().is_some() {
                return true;
            }
        };
    }
    hydro_native_view_types!(check_native_view);
    false
}

pub(crate) fn dimensions_for_known_native_views(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> Option<ViewDimensions> {
    macro_rules! try_native_dimensions {
        ($ty:ty) => {
            if let Some(dimensions) = dimensions_for_native::<$ty>(view, proposal, state, env) {
                return Some(dimensions);
            }
        };
    }
    hydro_native_view_types!(try_native_dimensions);
    None
}
