//! The one trait every mainline backend implements: a method for each
//! native component and each metadata key that needs the platform.
//!
//! The methods have no defaults, so a backend that misses one does not
//! compile. A component behind a facade feature has its method only when
//! the application enabled that feature (`waterui_feature` cfgs, set by
//! this crate's build script from the facade's `DEP_WATERUI_FEATURES`).
//!
//! A backend implements [`NativeBackend`] and realizes a tree through
//! [`Realize`], the core [`Backend`] that routes each payload to its
//! method. The routing compares `TypeId`s, which fold to constants after
//! monomorphization: a payload type reaches exactly one method through a
//! direct call, and a method no reached payload names is never
//! instantiated. An [`AnyView`] is the one place a tree erases; its leaves
//! come back through the same routing, by `TypeId` at run time.

use alloc::boxed::Box;
use core::any::{Any, type_name};

use waterui_core::backend::{Backend, DynBackend, cast, take};
use waterui_core::metadata::MetadataKey;
use waterui_core::{AnyView, Environment, Metadata, NativeView, View, contract_violation};

/// Declares [`NativeBackend`] and both routers from one list, so the trait
/// and the routing never disagree.
macro_rules! native_backend {
    (
        natives {
            $( $(#[cfg($ncfg:meta)])* $(#[doc = $ndoc:literal])* $native:ident: $native_ty:ty, )*
        }
        metadata {
            $( $(#[cfg($mcfg:meta)])* $(#[doc = $mdoc:literal])* $meta:ident: $meta_ty:ty, )*
        }
    ) => {
        /// A backend's realization of every native component and every
        /// metadata key that needs the platform.
        ///
        /// Each `natives` method receives its payload by value; each
        /// metadata method receives the wrapper with its content, which it
        /// realizes through [`Realize::child`].
        pub trait NativeBackend: Sized {
            /// What one view becomes.
            type Output: 'static;
            /// What a parent hands each child it realizes.
            type Ctx: 'static;

            $(
                $(#[cfg($ncfg)])*
                $(#[doc = $ndoc])*
                fn $native(&mut self, native: $native_ty, env: &Environment, cx: Self::Ctx) -> Self::Output;
            )*

            $(
                $(#[cfg($mcfg)])*
                $(#[doc = $mdoc])*
                fn $meta(&mut self, metadata: Metadata<$meta_ty>, env: &Environment, cx: Self::Ctx) -> Self::Output;
            )*

            /// Realizes a payload no method names: one that reached the
            /// backend outside the components `WaterUI` defines.
            ///
            /// This is a contract violation. A backend reports it and
            /// returns the output of an empty view.
            fn unrealized(&mut self, type_name: &'static str, env: &Environment, cx: Self::Ctx) -> Self::Output;
        }

        fn route_native<B: NativeBackend, T: NativeView + 'static>(
            backend: &mut B,
            native: T,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            $(
                $(#[cfg($ncfg)])*
                let native = match cast::<T, $native_ty>(native) {
                    Ok(native) => return backend.$native(native, env, cx),
                    Err(native) => native,
                };
            )*
            drop(native);
            unrealized::<B, T>(backend, env, cx)
        }

        fn route_metadata<B: NativeBackend, K: MetadataKey>(
            backend: &mut B,
            metadata: Metadata<K>,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            $(
                $(#[cfg($mcfg)])*
                let metadata = match cast::<Metadata<K>, Metadata<$meta_ty>>(metadata) {
                    Ok(metadata) => return backend.$meta(metadata, env, cx),
                    Err(metadata) => metadata,
                };
            )*
            drop(metadata);
            unrealized::<B, K>(backend, env, cx)
        }

        fn route_native_dyn<B: NativeBackend>(
            backend: &mut B,
            native: &mut dyn Any,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            $(
                $(#[cfg($ncfg)])*
                if let Some(native) = take::<$native_ty>(native) {
                    return backend.$native(native, env, cx);
                }
            )*
            backend.unrealized("an erased native payload", env, cx)
        }

        fn route_metadata_dyn<B: NativeBackend>(
            backend: &mut B,
            metadata: &mut dyn Any,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            $(
                $(#[cfg($mcfg)])*
                if let Some(metadata) = take::<Metadata<$meta_ty>>(metadata) {
                    return backend.$meta(metadata, env, cx);
                }
            )*
            backend.unrealized("an erased metadata key", env, cx)
        }

        #[cfg(test)]
        impl NativeBackend for tests::Recorder {
            type Output = ();
            type Ctx = ();

            $(
                $(#[cfg($ncfg)])*
                fn $native(&mut self, _native: $native_ty, _env: &Environment, (): ()) {
                    self.reached.push(stringify!($native));
                }
            )*

            $(
                $(#[cfg($mcfg)])*
                fn $meta(&mut self, metadata: Metadata<$meta_ty>, env: &Environment, (): ()) {
                    self.reached.push(stringify!($meta));
                    Realize::child(self, metadata.content, env, ());
                }
            )*

            fn unrealized(&mut self, type_name: &'static str, _env: &Environment, (): ()) {
                self.reached.push(type_name);
            }
        }
    };
}

fn unrealized<B: NativeBackend, T>(backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
    let name = type_name::<T>();
    contract_violation!("{name} reached a backend, but `NativeBackend` names no method for it");
    backend.unrealized(name, env, cx)
}

/// The core [`Backend`] a [`NativeBackend`] realizes a tree through.
#[derive(Debug)]
pub struct Realize<'a, B> {
    backend: &'a mut B,
}

impl<'a, B: NativeBackend> Realize<'a, B> {
    /// Wraps `backend`.
    pub const fn new(backend: &'a mut B) -> Self {
        Self { backend }
    }

    /// Realizes `view` through `backend`.
    pub fn child<V: View>(backend: &mut B, view: V, env: &Environment, cx: B::Ctx) -> B::Output {
        view.dispatch(&mut Realize::new(backend), env, cx)
    }
}

impl<B: NativeBackend> Backend for Realize<'_, B> {
    type Output = B::Output;
    type Ctx = B::Ctx;

    fn native<T: NativeView + 'static>(
        &mut self,
        native: T,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        route_native(self.backend, native, env, cx)
    }

    fn metadata<K: MetadataKey>(
        &mut self,
        metadata: Metadata<K>,
        env: &Environment,
        cx: Self::Ctx,
    ) -> Self::Output {
        route_metadata(self.backend, metadata, env, cx)
    }

    fn erased(&mut self, view: AnyView, env: &Environment, cx: Self::Ctx) -> Self::Output {
        let output = view.dispatch_dyn(&mut RealizeDyn(self.backend), env, Box::new(cx));
        *output
            .downcast::<B::Output>()
            .expect("the erased path returns the backend's own output")
    }
}

/// The object-safe face of a [`NativeBackend`], for the leaves of an
/// [`AnyView`].
struct RealizeDyn<'a, B>(&'a mut B);

impl<B: NativeBackend> RealizeDyn<'_, B> {
    fn cx(cx: Box<dyn Any>) -> B::Ctx {
        *cx.downcast::<B::Ctx>()
            .expect("the erased path carries the backend's own context")
    }
}

impl<B: NativeBackend> DynBackend for RealizeDyn<'_, B> {
    fn native_dyn(
        &mut self,
        native: &mut dyn Any,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(route_native_dyn(self.0, native, env, Self::cx(cx)))
    }

    fn metadata_dyn(
        &mut self,
        metadata: &mut dyn Any,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(route_metadata_dyn(self.0, metadata, env, Self::cx(cx)))
    }

    fn erased_dyn(&mut self, view: AnyView, env: &Environment, cx: Box<dyn Any>) -> Box<dyn Any> {
        Box::new(Realize::new(self.0).erased(view, env, Self::cx(cx)))
    }
}

native_backend! {
    natives {
        /// The empty view.
        empty: (),
        /// A run of styled text.
        text: waterui::text::TextConfig,
        /// A plain string label.
        label: waterui_core::Str,
        /// A solid color fill.
        color: waterui::Color,
        /// A flexible gap in a stack.
        spacer: waterui::layout::Spacer,
        /// A container with a fixed child list and a layout.
        fixed_container: waterui::layout::container::FixedContainer,
        /// A container whose children are produced lazily.
        lazy_container: waterui::layout::container::LazyContainer,
        /// A scroll view.
        scroll: waterui::layout::scroll::ScrollView,
        /// A view whose content a signal replaces.
        dynamic: waterui_core::Dynamic,
        /// A resolved shape.
        shape: waterui::shape::ResolvedShape,
        /// A system icon.
        icon: waterui::icon::SystemIcon,
        /// A gradient fill.
        gradient: waterui::graphics::Gradient,
        /// A button.
        button: waterui::component::button::ButtonConfig,
        /// A badge.
        badge: waterui::component::badge::BadgeConfig,
        /// A progress indicator.
        progress: waterui::component::progress::ProgressConfig,
        #[cfg(waterui_feature = "toggle")]
        /// A toggle.
        toggle: waterui::component::toggle::ToggleConfig,
        #[cfg(waterui_feature = "slider")]
        /// A slider.
        slider: waterui::component::slider::SliderConfig,
        #[cfg(waterui_feature = "stepper")]
        /// A stepper.
        stepper: waterui::component::stepper::StepperConfig,
        #[cfg(waterui_feature = "text-field")]
        /// A text field.
        text_field: waterui::component::text_field::ResolvedTextFieldConfig,
        #[cfg(waterui_feature = "secure-field")]
        /// A secure text field.
        secure_field: waterui::form::secure::SecureFieldConfig,
        #[cfg(waterui_feature = "picker")]
        /// A picker.
        picker: waterui::form::picker::PickerConfig,
        #[cfg(waterui_feature = "color-picker")]
        /// A color picker.
        color_picker: waterui::form::picker::color::ColorPickerConfig,
        #[cfg(waterui_feature = "date-picker")]
        /// A date picker.
        date_picker: waterui::form::picker::date::DatePickerConfig,
        #[cfg(waterui_feature = "multi-date-picker")]
        /// A picker of several dates.
        multi_date_picker: waterui::form::picker::multi_date::MultiDatePickerConfig,
        #[cfg(waterui_feature = "calendar")]
        /// A calendar.
        calendar: waterui::form::calendar::CalendarConfig,
        #[cfg(waterui_feature = "menu")]
        /// A menu.
        menu: waterui::component::menu::ResolvedMenu,
        #[cfg(waterui_feature = "list")]
        /// A list.
        list: waterui::component::list::ListConfig,
        #[cfg(waterui_feature = "list")]
        /// One row of a list.
        list_item: waterui::component::list::ListItem,
        #[cfg(waterui_feature = "table")]
        /// A table.
        table: waterui::component::table::TableConfig,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation bar around its content.
        navigation_view: waterui::navigation::NavigationView,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation stack.
        navigation_stack: waterui::navigation::NavigationStack<(), ()>,
        #[cfg(waterui_feature = "navigation")]
        /// A split navigation layout.
        navigation_split: waterui::navigation::split::NavigationSplitLayout,
        #[cfg(waterui_feature = "navigation")]
        /// Tabs.
        tabs: waterui::navigation::tab::TabsLayout,
        #[cfg(waterui_feature = "canvas")]
        /// A canvas.
        canvas: waterui::canvas::CanvasView,
        #[cfg(waterui_feature = "effects")]
        /// A view drawn through a filter.
        filtered: waterui::graphics::backend::FilteredView,
        #[cfg(waterui_feature = "gpu")]
        /// GPU content.
        gpu: waterui::GpuView,
        #[cfg(waterui_feature = "gpu")]
        /// Frames from an external source.
        external_frame: waterui::graphics::gpu::external::ExternalFrameView,
        #[cfg(waterui_feature = "gpu")]
        /// A retained scene.
        scene: waterui::graphics::backend::SceneView,
        #[cfg(waterui_feature = "gpu")]
        /// A recorded picture.
        picture: waterui::graphics::backend::Picture,
        #[cfg(waterui_feature = "video")]
        /// A video without controls.
        video: waterui::video::video::NativeVideoConfig,
        #[cfg(waterui_feature = "video")]
        /// A video player.
        video_player: waterui::video::video::NativeVideoPlayerConfig,
        #[cfg(waterui_feature = "webview")]
        /// A web view.
        web_view: waterui::webview::WebView,
        #[cfg(waterui_feature = "map")]
        /// A map.
        map: waterui::map::MapConfig,
    }
    metadata {
        /// An environment scope over its content.
        environment: Environment,
        /// A value kept alive with its content.
        retain: waterui_core::metadata::Retain,
        /// A layout priority.
        layout_priority: waterui_core::layout::LayoutPriority,
        /// A lifecycle hook.
        lifecycle_hook: waterui_core::event::LifeCycleHook,
        /// An event handler.
        on_event: waterui_core::event::OnEvent,
        /// A key-press handler.
        on_key_press: waterui_core::key::OnKeyPress,
        /// A gesture observer.
        gesture_observer: waterui_core::gesture::GestureObserver,
        /// Selection state.
        selected: waterui_core::interaction::Selected,
        /// The accessibility children.
        accessibility_children: waterui_core::accessibility::AccessibilityChildren,
        /// The accessibility geometry.
        accessibility_geometry: waterui_core::accessibility::AccessibilityGeometry,
        /// Hidden from accessibility.
        accessibility_hidden: waterui_core::accessibility::AccessibilityHidden,
        /// An accessibility identifier.
        accessibility_identifier: waterui_core::accessibility::AccessibilityIdentifier,
        /// An accessibility label.
        accessibility_label: waterui_core::accessibility::AccessibilityLabel,
        /// An accessibility role.
        accessibility_role: waterui_core::accessibility::AccessibilityRole,
        /// Accessibility state.
        accessibility_state: waterui_core::accessibility::AccessibilityState,
        /// Accessibility state from a signal.
        accessibility_state_signal: waterui_core::accessibility::AccessibilityStateSignal,
        /// An accessibility value.
        accessibility_value: waterui_core::accessibility::AccessibilityValue,
        /// A structural transition.
        transition: waterui_structural_transition::TransitionSpec,
        /// Opacity.
        opacity: waterui::filter::Opacity,
        /// A scale transform.
        scale: waterui::style::Scale,
        /// A rotation transform.
        rotation: waterui::style::Rotation,
        /// An offset.
        offset: waterui::style::Offset,
        /// A shadow.
        shadow: waterui::style::Shadow,
        /// A border.
        border: waterui::border::Border,
        /// A clip shape.
        clip_shape: waterui::shape::ClipShape,
        /// A background.
        background: waterui::background::Background,
        /// A material background.
        material_background: waterui::background::MaterialBackground,
        /// A glass background.
        glass_background: waterui::background::GlassBackground,
        /// A group sharing one backdrop capture.
        material_group: waterui::background::MaterialGroup,
        /// Safe-area edges the content ignores.
        ignore_safe_area: waterui::layout::safe_area::IgnoreSafeArea,
        /// A cursor.
        cursor: waterui::cursor::Cursor,
        /// A drag source.
        draggable: waterui::drag_drop::Draggable,
        /// A drop destination.
        drop_destination: waterui::drag_drop::DropDestination,
        /// Hit-testing.
        hittable: waterui::interaction::Hittable,
        /// Focus.
        focused: waterui::component::focus::Focused,
        /// Secure content.
        secure: waterui::metadata::secure::Secure,
        /// Standard dynamic range.
        standard_dynamic_range: waterui::metadata::secure::StandardDynamicRange,
        /// High dynamic range.
        high_dynamic_range: waterui::metadata::secure::HighDynamicRange,
        /// An anchored overlay.
        anchored_overlay: waterui::metadata::anchored_overlay::AnchoredOverlay,
        #[cfg(waterui_feature = "menu")]
        /// A context menu.
        context_menu: waterui::metadata::context_menu::ContextMenu,
        #[cfg(waterui_feature = "menu")]
        /// A resolved context menu.
        resolved_context_menu: waterui::metadata::context_menu::ResolvedContextMenu,
        #[cfg(waterui_feature = "dialog")]
        /// A dialog.
        dialog: waterui::dialog::Dialog,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation link hint.
        navigation_link_hint: waterui::navigation::NavigationLinkHint,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation transition source.
        navigation_transition_source: waterui::navigation::NavigationTransitionSource,
        #[cfg(waterui_feature = "navigation")]
        /// A navigation transition destination.
        navigation_transition_destination: waterui::navigation::NavigationTransitionDestination,
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use waterui::layout::Spacer;
    use waterui_core::layout::LayoutPriority;

    use super::*;

    /// Records the method each payload reaches.
    #[derive(Debug, Default)]
    pub(super) struct Recorder {
        pub(super) reached: Vec<&'static str>,
    }

    fn tree() -> impl View {
        Metadata::new(Spacer::flexible(), LayoutPriority::new(1))
    }

    #[test]
    fn each_payload_reaches_its_method() {
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, tree(), &Environment::new(), ());
        assert_eq!(recorder.reached, ["layout_priority", "spacer"]);
    }

    #[test]
    fn an_erased_tree_reaches_the_same_methods() {
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, AnyView::new(tree()), &Environment::new(), ());
        assert_eq!(recorder.reached, ["layout_priority", "spacer"]);
    }
}
