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
use core::any::Any;

use waterui_core::backend::{Backend, DynBackend, ErasedPayload, cast};
use waterui_core::metadata::MetadataKey;
use waterui_core::{AnyView, Environment, Metadata, NativeView, View, contract_violation};

use crate::child::{Child, ChildView, NativeBackendFamily};

/// Declares [`NativeBackend`] and both routers from one list, so the trait
/// and the routing never disagree.
macro_rules! native_backend {
    (
        natives {
            $( $(#[cfg($ncfg:meta)])* $(#[doc = $ndoc:literal])* $native:ident: $native_ty:ty, )*
        }
        parents {
            $( $(#[cfg($pcfg:meta)])* $(#[doc = $pdoc:literal])* $parent:ident: $($parent_ty:ident)::+, )*
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
            /// This backend type with its lifetimes left open, for the
            /// [`Child`] views of the payloads it receives.
            type Family: NativeBackendFamily<Output = Self::Output, Ctx = Self::Ctx>;

            /// Runs `f` on this backend as its family names it. Every
            /// implementation is `f(self)`; the signature is what ties the
            /// backend to its family.
            fn with_family<R>(
                &mut self,
                f: impl for<'a> FnOnce(&mut <Self::Family as NativeBackendFamily>::Backend<'a>) -> R,
            ) -> R;

            $(
                $(#[cfg($ncfg)])*
                $(#[doc = $ndoc])*
                fn $native(&mut self, native: $native_ty, env: &Environment, cx: Self::Ctx) -> Self::Output;
            )*

            $(
                $(#[cfg($pcfg)])*
                $(#[doc = $pdoc])*
                fn $parent<C: ChildView<Self>>(&mut self, native: $($parent_ty)::+<C>, env: &Environment, cx: Self::Ctx) -> Self::Output;
            )*

            $(
                $(#[cfg($mcfg)])*
                $(#[doc = $mdoc])*
                fn $meta(&mut self, metadata: Metadata<$meta_ty>, env: &Environment, cx: Self::Ctx) -> Self::Output;
            )*

            /// Realizes a native payload no method names: a component this
            /// backend provides itself, which it matches by type, or a payload
            /// it does not realize, which it reports through
            /// [`report_unprovided`] before returning an empty view's output.
            fn provided<T: NativeView + 'static>(&mut self, native: T, env: &Environment, cx: Self::Ctx) -> Self::Output;

            /// [`provided`](Self::provided) for a payload an [`AnyView`]
            /// handed back erased: the backend [`take`](ErasedPayload::take)s its
            /// own types out of `native`.
            fn provided_dyn(&mut self, native: ErasedPayload<'_>, env: &Environment, cx: Self::Ctx) -> Self::Output;

            /// Realizes a metadata wrapper no method names: a key this backend
            /// provides itself, or one it reports through
            /// [`report_unprovided`].
            fn provided_metadata<K: MetadataKey>(&mut self, metadata: Metadata<K>, env: &Environment, cx: Self::Ctx) -> Self::Output;

            /// [`provided_metadata`](Self::provided_metadata) for a wrapper an
            /// [`AnyView`] handed back erased.
            fn provided_metadata_dyn(&mut self, metadata: ErasedPayload<'_>, env: &Environment, cx: Self::Ctx) -> Self::Output;
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
            $(
                $(#[cfg($pcfg)])*
                let native = match cast::<T, $($parent_ty)::+<Child<B::Family>>>(native) {
                    Ok(native) => return backend.$parent(native, env, cx),
                    Err(native) => native,
                };
                $(#[cfg($pcfg)])*
                let native = match cast::<T, $($parent_ty)::+<AnyView>>(native) {
                    Ok(native) => return backend.$parent(native, env, cx),
                    Err(native) => native,
                };
            )*
            backend.provided(native, env, cx)
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
            backend.provided_metadata(metadata, env, cx)
        }

        fn route_native_dyn<B: NativeBackend>(
            backend: &mut B,
            mut native: ErasedPayload<'_>,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            $(
                $(#[cfg($ncfg)])*
                if let Some(native) = native.take::<$native_ty>() {
                    return backend.$native(native, env, cx);
                }
            )*
            $(
                $(#[cfg($pcfg)])*
                if let Some(native) = native.take::<$($parent_ty)::+<AnyView>>() {
                    return backend.$parent(native, env, cx);
                }
            )*
            backend.provided_dyn(native, env, cx)
        }

        fn route_metadata_dyn<B: NativeBackend>(
            backend: &mut B,
            mut metadata: ErasedPayload<'_>,
            env: &Environment,
            cx: B::Ctx,
        ) -> B::Output {
            $(
                $(#[cfg($mcfg)])*
                if let Some(metadata) = metadata.take::<Metadata<$meta_ty>>() {
                    return backend.$meta(metadata, env, cx);
                }
            )*
            backend.provided_metadata_dyn(metadata, env, cx)
        }

        #[cfg(test)]
        #[cfg(test)]
        impl NativeBackendFamily for tests::Recorder {
            type Output = ();
            type Ctx = ();
            type Backend<'a> = Self;
        }

        #[cfg(test)]
        impl NativeBackend for tests::Recorder {
            type Output = ();
            type Ctx = ();
            type Family = Self;

            fn with_family<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
                f(self)
            }

            $(
                $(#[cfg($ncfg)])*
                fn $native(&mut self, _native: $native_ty, _env: &Environment, (): ()) {
                    self.reached.push(stringify!($native));
                }
            )*

            $(
                $(#[cfg($pcfg)])*
                fn $parent<C: ChildView<Self>>(&mut self, native: $($parent_ty)::+<C>, env: &Environment, (): ()) {
                    self.reached.push(stringify!($parent));
                    tests::Recorder::realize_children(self, native, env);
                }
            )*

            $(
                $(#[cfg($mcfg)])*
                fn $meta(&mut self, metadata: Metadata<$meta_ty>, env: &Environment, (): ()) {
                    self.reached.push(stringify!($meta));
                    Realize::child(self, metadata.content, env, ());
                }
            )*

            fn provided<T: NativeView + 'static>(&mut self, _native: T, _env: &Environment, (): ()) {
                self.reached.push(core::any::type_name::<T>());
            }

            fn provided_dyn(&mut self, native: ErasedPayload<'_>, _env: &Environment, (): ()) {
                self.reached.push(native.type_name());
            }

            fn provided_metadata<K: MetadataKey>(&mut self, metadata: Metadata<K>, env: &Environment, (): ()) {
                self.reached.push(core::any::type_name::<K>());
                Realize::child(self, metadata.content, env, ());
            }

            fn provided_metadata_dyn(&mut self, metadata: ErasedPayload<'_>, _env: &Environment, (): ()) {
                self.reached.push(metadata.type_name());
            }
        }
    };
}

/// Reports that `payload` reached `backend`, which neither `NativeBackend`
/// nor the backend's own components realize: a contract violation.
pub fn report_unprovided(backend: &str, payload: &str) {
    contract_violation!(
        "{payload} reached {backend}, which realizes it neither through a `NativeBackend` method nor as a component it provides"
    );
}

/// How many erased views may expand into one another before any payload is
/// reached. Inherited from the recursion budget of Hydrolysis's former layout
/// normalization, the pass this check replaces; a finite composition never
/// comes near it, so only a body that erases itself again trips it.
const ERASED_HOP_BUDGET: usize = 64;

/// The core [`Backend`] a [`NativeBackend`] realizes a tree through.
#[derive(Debug)]
pub struct Realize<'a, B> {
    backend: &'a mut B,
    /// Erased views expanded since the last payload this realization reached.
    erased_hops: usize,
}

impl<'a, B: NativeBackend> Realize<'a, B> {
    /// Wraps `backend`.
    pub const fn new(backend: &'a mut B) -> Self {
        Self {
            backend,
            erased_hops: 0,
        }
    }

    /// Realizes `view` through `backend`.
    pub fn child<V: View>(backend: &mut B, view: V, env: &Environment, cx: B::Ctx) -> B::Output {
        view.dispatch(&mut Realize::new(backend), env, cx)
    }

    /// Realizes the native payload an [`AnyView`] handed back through a
    /// [`DynBackend`].
    pub fn native_dyn(
        &mut self,
        native: ErasedPayload<'_>,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        route_native_dyn(self.backend, native, env, cx)
    }

    /// Realizes the metadata wrapper an [`AnyView`] handed back through a
    /// [`DynBackend`].
    pub fn metadata_dyn(
        &mut self,
        metadata: ErasedPayload<'_>,
        env: &Environment,
        cx: B::Ctx,
    ) -> B::Output {
        route_metadata_dyn(self.backend, metadata, env, cx)
    }
}

impl<B: NativeBackend> Backend for Realize<'_, B> {
    type Output = B::Output;
    type Ctx = B::Ctx;
    type Slot = Child<B::Family>;

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
        if self.erased_hops == ERASED_HOP_BUDGET {
            contract_violation!(
                "{} expanded through {ERASED_HOP_BUDGET} erased views without reaching a payload; its body erases itself, so it realizes as an empty view",
                view.name()
            );
            return route_native(self.backend, (), env, cx);
        }
        let mut erased = RealizeDyn {
            backend: self.backend,
            erased_hops: self.erased_hops + 1,
        };
        let output = view.dispatch_dyn(&mut erased, env, Box::new(cx));
        *output
            .downcast::<B::Output>()
            .expect("the erased path returns the backend's own output")
    }
}

/// The object-safe face of a [`NativeBackend`], for the leaves of an
/// [`AnyView`].
struct RealizeDyn<'a, B> {
    backend: &'a mut B,
    erased_hops: usize,
}

impl<B: NativeBackend> RealizeDyn<'_, B> {
    fn cx(cx: Box<dyn Any>) -> B::Ctx {
        *cx.downcast::<B::Ctx>()
            .expect("the erased path carries the backend's own context")
    }
}

impl<B: NativeBackend> DynBackend for RealizeDyn<'_, B> {
    fn native_dyn(
        &mut self,
        native: ErasedPayload<'_>,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(route_native_dyn(self.backend, native, env, Self::cx(cx)))
    }

    fn metadata_dyn(
        &mut self,
        metadata: ErasedPayload<'_>,
        env: &Environment,
        cx: Box<dyn Any>,
    ) -> Box<dyn Any> {
        Box::new(route_metadata_dyn(
            self.backend,
            metadata,
            env,
            Self::cx(cx),
        ))
    }

    fn erased_dyn(&mut self, view: AnyView, env: &Environment, cx: Box<dyn Any>) -> Box<dyn Any> {
        let mut realize = Realize {
            backend: &mut *self.backend,
            erased_hops: self.erased_hops,
        };
        Box::new(realize.erased(view, env, Self::cx(cx)))
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
        /// A line separating content.
        divider: waterui::layout::Divider,
        /// A container with a fixed child list and a layout.
        fixed_container: waterui::layout::container::FixedContainer,
        /// A container whose children are produced lazily.
        lazy_container: waterui::layout::container::LazyContainer,
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
        /// A canvas.
        canvas: waterui::graphics::canvas::CanvasView,
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
    parents {
        /// A scroll view.
        scroll: waterui::layout::scroll::ScrollView,
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
    use waterui::layout::scroll::scroll;
    use waterui_core::layout::LayoutPriority;

    use super::*;

    /// Records the method each payload reaches.
    #[derive(Debug, Default)]
    pub(super) struct Recorder {
        pub(super) reached: Vec<&'static str>,
    }

    impl Recorder {
        /// Realizes each child view `native` holds.
        pub(super) fn realize_children<C: ChildView<Self>>(
            &mut self,
            native: waterui::layout::scroll::ScrollView<C>,
            env: &Environment,
        ) {
            self.reached.push(
                if core::any::TypeId::of::<C>() == core::any::TypeId::of::<AnyView>() {
                    "erased child"
                } else {
                    "typed child"
                },
            );
            let (_, content, _, _) = native.into_inner().into_parts();
            content.realize(self, env, ());
        }
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

    #[derive(Debug)]
    struct ErasesItself;

    impl View for ErasesItself {
        fn body(self, _env: &Environment) -> impl View {
            AnyView::new(self)
        }
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "erased views without reaching a payload")
    )]
    fn a_body_that_erases_itself_realizes_as_an_empty_view() {
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, ErasesItself, &Environment::new(), ());
        assert_eq!(recorder.reached, ["empty"]);
    }

    #[test]
    fn a_payload_child_realizes_through_the_same_backend() {
        let mut recorder = Recorder::default();
        let env = Environment::new();
        Realize::child(&mut recorder, scroll(tree()), &env, ());
        Realize::child(&mut recorder, AnyView::new(scroll(tree())), &env, ());
        Realize::child(&mut recorder, scroll(AnyView::new(tree())), &env, ());
        // A typed tree's child stays typed; only an erased scroll's payload
        // carries an erased child. Content erased by hand stays erased
        // inside the typed slot.
        assert_eq!(
            recorder.reached,
            [
                ["scroll", "typed child", "layout_priority", "spacer"],
                ["scroll", "erased child", "layout_priority", "spacer"],
                ["scroll", "typed child", "layout_priority", "spacer"],
            ]
            .concat()
        );
    }

    #[test]
    fn an_erased_tree_reaches_the_same_methods() {
        let mut recorder = Recorder::default();
        Realize::child(&mut recorder, AnyView::new(tree()), &Environment::new(), ());
        assert_eq!(recorder.reached, ["layout_priority", "spacer"]);
    }
}
