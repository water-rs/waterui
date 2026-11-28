//! Embedding `AppKit` (`NSView`) and `UIKit` (`UIView`) views through
//! [`AppleContent`] and [`AppleView`].
//!
//! A crate that wraps a platform-coupled Apple control — `PKPaymentButton`, a
//! system picker, an advertising SDK's banner view — implements [`AppleContent`]
//! in Rust against `objc2` and places it in a view tree with [`AppleView`].
//! Both live outside `NativeBackend`'s list of first-party components:
//! [`AppleView`] reaches [`AppleBackend`](crate::backend::AppleBackend) through
//! `NativeBackend::provided` on the static path (monomorphized for `T` without
//! boxing) and `NativeBackend::provided_dyn` on the
//! [`Erased`](waterui_core::backend::Erased) path, answers
//! [`waterui_backend_core::ProvidedSemantics`] on semantic
//! test runtimes without mounting a platform view, and reports a contract
//! violation when reaching any other presentation backend.

use core::any::Any;
use core::fmt;

use alloc::rc::Rc;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "macos")]
pub use cocoa_ui::objc2_app_kit;
#[cfg(target_os = "ios")]
pub use cocoa_ui::objc2_ui_kit;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;
pub use cocoa_ui::{MainThreadMarker, PlatformView, Retained, objc2};
use nami::watcher::Context;
use nami::{Computed, Signal};
use waterui_accessibility::AccessibilityRole;
use waterui_backend_core::ProvidedSemantics;
use waterui_core::Str;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_core::{Environment, Native, NativeView, View};

use crate::contract::{KeepAlive, NativeLeaf};

/// Invalidates layout or captured rendering for a mounted [`AppleContent`] view
/// when a reactive input changes what the platform view measures or draws.
#[derive(Clone, Debug)]
pub struct AppleInvalidator {
    host: Retained<PlatformView>,
}

impl AppleInvalidator {
    /// Marks the leaf's memoized measurements stale, schedules native layout up
    /// the superview chain, and notifies any enclosing rendered-content capture.
    pub fn invalidate_layout(&self) {
        crate::invalidation::invalidate_layout_hierarchy(&self.host);
    }

    /// Notifies any enclosing rendered-content capture that the platform view
    /// redrew in place without changing its measured size.
    pub fn invalidate_display(&self) {
        crate::invalidation::invalidate_rendered_content(&self.host);
    }
}

/// The main-thread mount context handed to [`AppleContent::make_view`].
///
/// Supplies the active [`Environment`], the [`MainThreadMarker`] proving the
/// call runs on the platform main thread, an [`AppleInvalidator`] for reactive
/// size or display changes, and the mounted leaf's [`KeepAlive`] store so
/// signal subscription guards, target-action receivers, and delegates live
/// exactly as long as the mounted view.
#[derive(Debug)]
pub struct AppleMount<'a> {
    env: &'a Environment,
    mtm: MainThreadMarker,
    host: &'a PlatformView,
    keepalive: &'a mut KeepAlive,
}

impl AppleMount<'_> {
    /// The environment at this view's position in the tree.
    #[must_use]
    pub const fn env(&self) -> &Environment {
        self.env
    }

    /// Proof that [`AppleContent::make_view`] is executing on the platform main
    /// thread, for `objc2` initializers that require main-thread confinement.
    #[must_use]
    pub const fn mtm(&self) -> MainThreadMarker {
        self.mtm
    }

    /// A cloneable handle that invalidates layout or captured display when a
    /// signal watcher mutates the mounted platform view.
    #[must_use]
    pub fn invalidator(&self) -> AppleInvalidator {
        AppleInvalidator {
            host: cocoa_ui::view::retain_base(self.host),
        }
    }

    /// Retains `value` — a watcher guard, an Objective-C target or delegate, or
    /// auxiliary state — with the mounted view.
    ///
    /// Kept values drop in reverse insertion order before the platform view is
    /// released.
    pub fn keep(&mut self, value: impl Any) {
        self.keepalive.keep(value);
    }

    /// Subscribes `watcher` to `signal` and retains the guard with the mounted
    /// view.
    pub fn watch<S: Signal>(&mut self, signal: &S, watcher: impl Fn(Context<S::Output>) + 'static) {
        self.keepalive.watch(signal, watcher);
    }

    /// Applies `signal`'s current value immediately, then subscribes `apply` to
    /// every subsequent change and retains the guard with the mounted view.
    pub fn bind<S: Signal>(&mut self, signal: &S, apply: impl Fn(S::Output) + 'static) {
        self.keepalive.bind(signal, apply);
    }
}

/// Supplies an `AppKit` (`NSView` on macOS) or `UIKit` (`UIView` on iOS) view
/// for [`AppleView`].
pub trait AppleContent: 'static {
    /// The `NSView` (macOS) or `UIView` (iOS) class this content creates.
    type View: AsRef<PlatformView> + 'static;

    /// Creates the platform view on the main thread and subscribes any reactive
    /// inputs through `mount`, keeping their guards with the mounted view.
    fn make_view(&mut self, mount: &mut AppleMount<'_>) -> Retained<Self::View>;

    /// Measures the mounted `view` against `proposal`.
    ///
    /// Defaults to `view`'s fitting size (`NSView::fittingSize` on macOS,
    /// `UIView::systemLayoutSizeFittingSize(CGSize::ZERO)` on iOS), expanding
    /// along axes [`stretch_axis`](Self::stretch_axis) stretches when `proposal`
    /// offers more space.
    #[must_use]
    fn measure(&self, view: &Self::View, proposal: ProposalSize) -> ViewDimensions {
        default_measure(
            cocoa_ui::view::fitting_size(view.as_ref()),
            self.stretch_axis(),
            proposal,
        )
    }

    /// Resolves `view`'s response geometry inside `bounds` under the layout
    /// specification's leaf rule (`docs/layout-spec.md` §6), defaulting to
    /// [`ResolvedExtent::centred`].
    ///
    /// # Errors
    ///
    /// Returns a [`LayoutError`] when `bounds` or the measured answer is not
    /// valid geometry.
    fn resolve_extent(
        &self,
        view: &Self::View,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(view, proposal).size)
    }

    /// Which axis (or axes) this content stretches to fill available space.
    #[must_use]
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    /// The semantic role this content declares for backends that build their
    /// own accessibility tree without mounting a platform view.
    #[must_use]
    fn accessibility_role(&self) -> Option<AccessibilityRole> {
        None
    }

    /// The spoken label this content declares for backends that build their own
    /// accessibility tree without mounting a platform view.
    #[must_use]
    fn accessibility_label(&self) -> Option<Computed<Str>> {
        None
    }

    /// The semantic activation handler this content exposes for semantic test
    /// runtimes without mounting a platform view.
    #[must_use]
    fn accessibility_activate(&self) -> Option<Rc<dyn Fn()>> {
        None
    }
}

/// Default leaf measurement from a platform view's `fitting` size and declared
/// `stretch` axis: non-stretching axes report `fitting`, while stretching axes
/// accept a larger proposal (including `+INFINITY`) and never compress below
/// `fitting`, satisfying `min <= ideal <= max`.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract uses f32 points; kit fitting sizes are f64 points"
)]
pub(crate) fn default_measure(
    fitting: cocoa_ui::geometry::Size,
    stretch: StretchAxis,
    proposal: ProposalSize,
) -> ViewDimensions {
    let fit_width = (fitting.width as f32).max(0.0);
    let fit_height = (fitting.height as f32).max(0.0);
    let width = if stretch.stretches_horizontal() {
        proposal
            .width
            .map_or(fit_width, |offered| offered.max(fit_width))
    } else {
        fit_width
    };
    let height = if stretch.stretches_vertical() {
        proposal
            .height
            .map_or(fit_height, |offered| offered.max(fit_height))
    } else {
        fit_height
    };
    ViewDimensions::new(Size::new(width, height))
}

/// Private receiver [`AppleBackend`](crate::backend::AppleBackend) passes into
/// [`NativeView::realize_provided`] to realize [`AppleView`] monomorphized on
/// the static path and without extra boxing across [`ErasedPayload`](waterui_core::backend::ErasedPayload).
pub(crate) struct AppleProvidedSlot {
    pub(crate) mtm: MainThreadMarker,
    pub(crate) leaf: Option<NativeLeaf>,
}

/// A component the Apple backend provides to embed [`AppleContent`] (`NSView`
/// on macOS, `UIView` on iOS) inside a `WaterUI` view tree.
///
/// Generic over `T` so static dispatch through
/// [`AppleBackend`](crate::backend::AppleBackend) is monomorphized without
/// boxing `T`; `T` is boxed only when the caller erases the subtree with
/// [`AnyView`](waterui_core::AnyView). Reaching any other presentation backend
/// reports a contract violation.
#[must_use]
pub struct AppleView<T: AppleContent> {
    content: T,
}

impl<T: AppleContent> fmt::Debug for AppleView<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppleView")
            .field("content", &core::any::type_name::<T>())
            .finish()
    }
}

impl<T: AppleContent> AppleView<T> {
    /// Wraps `content` in an Apple-backend leaf view.
    pub const fn new(content: T) -> Self {
        Self { content }
    }

    /// Consumes the view and returns the inner [`AppleContent`].
    #[must_use]
    pub fn into_inner(self) -> T {
        self.content
    }

    /// Borrows the inner [`AppleContent`].
    #[must_use]
    pub const fn as_inner(&self) -> &T {
        &self.content
    }

    /// Mutably borrows the inner [`AppleContent`].
    #[must_use]
    pub const fn as_inner_mut(&mut self) -> &mut T {
        &mut self.content
    }
}

impl<T: AppleContent> NativeView for AppleView<T> {
    fn stretch_axis(&self) -> StretchAxis {
        self.content.stretch_axis()
    }

    fn realize_provided(self, context: &mut dyn Any, env: &Environment) -> Result<(), Self> {
        if let Some(slot) = context.downcast_mut::<AppleProvidedSlot>() {
            slot.leaf = Some(realize_apple_view(self.content, env, slot.mtm));
            return Ok(());
        }
        if let Some(semantics) = context.downcast_mut::<ProvidedSemantics>() {
            semantics.set_role(self.content.accessibility_role());
            semantics.set_label(self.content.accessibility_label());
            semantics.set_on_activate(self.content.accessibility_activate());
            return Ok(());
        }
        Err(self)
    }
}

impl<T: AppleContent> View for AppleView<T> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }
}

/// Monomorphized [`SubView`] face for a mounted [`AppleContent`] instance.
struct AppleSubView<T: AppleContent> {
    content: T,
    view: Retained<T::View>,
}

impl<T: AppleContent> fmt::Debug for AppleSubView<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppleSubView")
            .field("content", &core::any::type_name::<T>())
            .finish()
    }
}

impl<T: AppleContent> SubView for AppleSubView<T> {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        self.content.resolve_extent(&self.view, bounds, proposal)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.content.measure(&self.view, proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.content.stretch_axis()
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Mounts `content` inside a `HostView` leaf: adds the created `NSView`/`UIView`
/// as a subview, forwards primary-content queries, frames the child at the
/// leaf's committed response rectangle on every layout pass, and retains signal
/// subscription guards for the leaf's lifetime.
pub(crate) fn realize_apple_view<T: AppleContent>(
    mut content: T,
    env: &Environment,
    mtm: MainThreadMarker,
) -> NativeLeaf {
    let host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
    let mut keepalive = KeepAlive::default();
    let view = {
        let mut mount = AppleMount {
            env,
            mtm,
            host: &host,
            keepalive: &mut keepalive,
        };
        content.make_view(&mut mount)
    };
    let base = cocoa_ui::view::retain_base(view.as_ref());
    cocoa_ui::view::add_subview(&host, &base);
    crate::primary_content::forward(&host, &base);
    host.set_layout_handler(move |host| {
        cocoa_ui::view::set_frame(&base, crate::proposal::response(host));
    });
    let mut leaf = NativeLeaf::new(&*host, AppleSubView { content, view });
    leaf.keep(keepalive);
    leaf
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_backend_core::{
        ChildView, NativeBackend, NativeBackendFamily, Realize, report_unprovided,
    };
    use waterui_controls::anchored_overlay::AnchoredOverlay;
    use waterui_controls::sheet::Sheet;
    use waterui_core::backend::{ErasedPayload, peel_native};
    use waterui_core::metadata::MetadataKey;
    use waterui_core::{AnyView, Metadata};

    struct DummyContent {
        stretch: StretchAxis,
        label: Computed<Str>,
    }

    impl DummyContent {
        fn new(stretch: StretchAxis, label: Computed<Str>) -> Self {
            Self { stretch, label }
        }
    }

    impl AppleContent for DummyContent {
        type View = PlatformView;

        fn make_view(&mut self, _mount: &mut AppleMount<'_>) -> Retained<Self::View> {
            unreachable!("headless semantic and dispatch unit tests never mount platform views")
        }

        fn stretch_axis(&self) -> StretchAxis {
            self.stretch
        }

        fn accessibility_role(&self) -> Option<AccessibilityRole> {
            Some(AccessibilityRole::Button)
        }

        fn accessibility_label(&self) -> Option<Computed<Str>> {
            Some(self.label.clone())
        }
    }

    #[test]
    fn default_measure_obeys_fitting_size_and_stretch_axis() {
        let fitting = cocoa_ui::geometry::Size::new(140.0, 32.0);

        for proposal in [
            ProposalSize::UNSPECIFIED,
            ProposalSize::new(Some(0.0), Some(0.0)),
            ProposalSize::new(Some(300.0), Some(80.0)),
            ProposalSize::new(Some(f32::INFINITY), Some(f32::INFINITY)),
        ] {
            assert_eq!(
                default_measure(fitting, StretchAxis::None, proposal).size,
                Size::new(140.0, 32.0)
            );
        }

        assert_eq!(
            default_measure(fitting, StretchAxis::Horizontal, ProposalSize::UNSPECIFIED).size,
            Size::new(140.0, 32.0)
        );
        assert_eq!(
            default_measure(
                fitting,
                StretchAxis::Horizontal,
                ProposalSize::new(Some(0.0), Some(0.0))
            )
            .size,
            Size::new(140.0, 32.0)
        );
        assert_eq!(
            default_measure(
                fitting,
                StretchAxis::Horizontal,
                ProposalSize::new(Some(320.0), Some(100.0))
            )
            .size,
            Size::new(320.0, 32.0)
        );
        assert_eq!(
            default_measure(
                fitting,
                StretchAxis::Horizontal,
                ProposalSize::new(Some(f32::INFINITY), Some(f32::INFINITY))
            )
            .size,
            Size::new(f32::INFINITY, 32.0)
        );
    }

    #[test]
    fn apple_view_preserves_concrete_type_and_declared_semantics() {
        let env = Environment::new();
        let view = AppleView::new(DummyContent::new(
            StretchAxis::Horizontal,
            Computed::constant(Str::from_static("Pay with Apple Pay")),
        ));
        assert_eq!(NativeView::stretch_axis(&view), StretchAxis::Horizontal);

        let peeled = peel_native::<AppleView<DummyContent>>(view, &env)
            .expect("AppleView dispatches its unboxed concrete type");
        let mut semantics = ProvidedSemantics::default();
        assert!(peeled.realize_provided(&mut semantics, &env).is_ok());
        assert_eq!(semantics.role(), Some(&AccessibilityRole::Button));
        assert_eq!(
            semantics
                .label()
                .expect("label populated")
                .snapshot()
                .as_str(),
            "Pay with Apple Pay"
        );
    }

    /// A minimal non-Apple `NativeBackend` whose `provided` / `provided_dyn`
    /// report contract violations, verifying that `AppleView` reaching another
    /// backend triggers `report_unprovided` with the real `AppleView<T>` name.
    #[derive(Clone, Copy)]
    struct OtherBackend;

    impl NativeBackendFamily for OtherBackend {
        type Output = ();
        type Ctx = ();
        type Backend<'a> = Self;

        fn reborrow(_backend: &mut Self) -> Self {
            Self
        }
    }

    macro_rules! stub_natives {
        ($( $(#[cfg($cfg:meta)])* $name:ident : $ty:ty ),* $(,)?) => {
            $(
                $(#[cfg($cfg)])*
                fn $name(&mut self, _: $ty, _: &Environment, (): ()) {}
            )*
        };
    }

    macro_rules! stub_parents {
        ($( $(#[cfg($cfg:meta)])* $name:ident < $c:ident > : $ty:ty ),* $(,)?) => {
            $(
                $(#[cfg($cfg)])*
                fn $name<$c: ChildView<Self>>(&mut self, _: $ty, _: &Environment, (): ()) {}
            )*
        };
    }

    macro_rules! stub_metadata {
        ($( $(#[cfg($cfg:meta)])* $name:ident : $ty:ty ),* $(,)?) => {
            $(
                $(#[cfg($cfg)])*
                fn $name<C: ChildView<Self>>(&mut self, _: Metadata<$ty, C>, _: &Environment, (): ()) {}
            )*
        };
    }

    #[cfg(waterui_feature = "capture")]
    struct OtherOffscreen;

    #[cfg(waterui_feature = "capture")]
    impl waterui_capture::ViewRenderer for OtherOffscreen {
        fn render_to_rgba<V: View>(
            &self,
            _view: V,
            _env: &Environment,
            _request: waterui_capture::RenderRequest,
        ) -> impl core::future::Future<
            Output = Result<waterui_capture::RenderResult, waterui_capture::RenderError>,
        > {
            core::future::ready(Ok(waterui_capture::RenderResult::new(Vec::new(), 1, 1)))
        }
    }

    impl NativeBackend for OtherBackend {
        type Output = ();
        type Ctx = ();
        type Family = Self;
        #[cfg(waterui_feature = "webview")]
        type WebViewController = waterui_backend_core::webview::UnavailableWebViewController;
        #[cfg(waterui_feature = "capture")]
        type Offscreen = OtherOffscreen;

        fn with_family<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
            f(self)
        }

        #[cfg(waterui_feature = "webview")]
        fn web_view_controller(&self) -> Self::WebViewController {
            waterui_backend_core::webview::UnavailableWebViewController
        }

        #[cfg(waterui_feature = "capture")]
        fn offscreen(&self, _env: &Environment) -> Self::Offscreen {
            OtherOffscreen
        }

        stub_natives! {
            empty: (),
            text: waterui_text::TextConfig,
            #[cfg(waterui_feature = "highlight")]
            code: waterui_text::code::CodeConfig,
            label: waterui_core::Str,
            color: waterui_graphics::Color,
            spacer: waterui_layout::Spacer,
            divider: waterui_controls::Divider,
            shape: waterui_shape::ResolvedShape,
            gradient: waterui_graphics::Gradient,
            #[cfg(any(waterui_feature = "gpu", waterui_feature = "cpu"))]
            shader_paint: waterui_graphics::ShaderPaintView,
            #[cfg(waterui_feature = "canvas")]
            canvas: waterui_graphics::canvas::CanvasView,
            #[cfg(waterui_feature = "gpu")]
            gpu: waterui_graphics::backend::GpuContentLeaf,
            #[cfg(waterui_feature = "gpu")]
            external_frame: waterui_graphics::gpu::external::ExternalFrameView,
            #[cfg(any(
                waterui_feature = "gpu",
                waterui_feature = "cpu",
                waterui_feature = "canvas"
            ))]
            scene: waterui_graphics::backend::SceneView,
            #[cfg(any(
                waterui_feature = "gpu",
                waterui_feature = "cpu",
                waterui_feature = "canvas"
            ))]
            picture: waterui_graphics::backend::Picture,
            #[cfg(waterui_feature = "video")]
            video: waterui_backend_core::video::video::NativeVideoConfig,
            #[cfg(waterui_feature = "video")]
            video_player: waterui_backend_core::video::video::NativeVideoPlayerConfig,
            #[cfg(waterui_feature = "webview")]
            web_view: waterui_backend_core::webview::WebViewOpen,
            #[cfg(waterui_feature = "map")]
            map: waterui_backend_core::map::MapConfig,
        }

        stub_parents! {
            #[cfg(waterui_feature = "avatar")]
            avatar<C>: waterui_controls::avatar::AvatarConfig<C>,
            button<C>: waterui_controls::button::ButtonConfig<C>,
            #[cfg(waterui_feature = "link")]
            link<C>: waterui_controls::link::LinkConfig<C>,
            badge<C>: waterui_controls::badge::BadgeConfig<C>,
            card<C>: waterui_controls::card::CardConfig<C>,
            disclosure<C>: waterui_controls::disclosure::DisclosureConfig<C>,
            progress<C>: waterui_controls::progress::ProgressConfig<C>,
            #[cfg(waterui_feature = "toggle")]
            toggle<C>: waterui_controls::toggle::ToggleConfig<C>,
            #[cfg(waterui_feature = "slider")]
            slider<C>: waterui_controls::slider::SliderConfig<C>,
            #[cfg(waterui_feature = "stepper")]
            stepper<C>: waterui_controls::stepper::StepperConfig<C>,
            #[cfg(waterui_feature = "text-field")]
            text_field<C>: waterui_controls::text_field::TextFieldConfig<C>,
            #[cfg(waterui_feature = "secure-field")]
            secure_field<C>: waterui_backend_core::form::secure::SecureFieldConfig<C>,
            #[cfg(waterui_feature = "picker")]
            picker<C>: waterui_backend_core::form::picker::PickerConfig<C>,
            #[cfg(waterui_feature = "color-picker")]
            color_picker<C>: waterui_backend_core::form::picker::color::ColorPickerConfig<C>,
            #[cfg(waterui_feature = "date-picker")]
            date_picker<C>: waterui_backend_core::form::picker::date::DatePickerConfig<C>,
            #[cfg(waterui_feature = "multi-date-picker")]
            multi_date_picker<C>: waterui_backend_core::form::picker::multi_date::MultiDatePickerConfig<C>,
            #[cfg(waterui_feature = "calendar")]
            calendar<C>: waterui_backend_core::form::calendar::CalendarConfig<C>,
            #[cfg(waterui_feature = "menu")]
            menu<C>: waterui_controls::menu::ResolvedMenu<C>,
            scroll<C>: waterui_layout::scroll::ScrollView<C>,
            fixed_container<C>: waterui_layout::container::FixedContainer<alloc::vec::Vec<C>>,
            dynamic<C>: waterui_core::Dynamic<C>,
            lazy_container<C>: waterui_layout::container::LazyContainer<C>,
            #[cfg(waterui_feature = "effects")]
            filtered<C>: waterui_graphics::backend::FilteredView<C>,
            #[cfg(waterui_feature = "list")]
            list<C>: waterui_controls::list::ListConfig<C>,
            #[cfg(waterui_feature = "list")]
            list_row<C>: waterui_controls::list::ListRowConfig<C>,
            #[cfg(waterui_feature = "table")]
            table<C>: waterui_controls::table::TableConfig<C>,
            #[cfg(waterui_feature = "navigation")]
            navigation_view<C>: waterui_backend_core::navigation::ResolvedNavigationView<C>,
            #[cfg(waterui_feature = "navigation")]
            navigation_split<C>: waterui_backend_core::navigation::split::NavigationSplitLayout<C>,
            #[cfg(waterui_feature = "navigation")]
            navigation_stack<C>: waterui_backend_core::navigation::NavigationStackLayout<C>,
            #[cfg(waterui_feature = "navigation")]
            tabs<C>: waterui_backend_core::navigation::tab::TabsLayout<C>,
        }

        stub_metadata! {
            environment: Environment,
            retain: waterui_core::metadata::Retain,
            layout_priority: waterui_core::layout::LayoutPriority,
            report_size: waterui_layout::report_size::ReportSize,
            lifecycle_hook: waterui_interaction::event::LifeCycleHook,
            task_hook: waterui_interaction::event::TaskHook,
            on_event: waterui_interaction::event::OnEvent,
            on_key_press: waterui_interaction::key::OnKeyPress,
            gesture_observer: waterui_interaction::gesture::GestureObserver,
            selected: waterui_interaction::interaction::Selected,
            accessibility_children: waterui_accessibility::AccessibilityChildren,
            accessibility_geometry: waterui_accessibility::AccessibilityGeometry,
            accessibility_hidden: waterui_accessibility::AccessibilityHidden,
            accessibility_identifier: waterui_accessibility::AccessibilityIdentifier,
            accessibility_label: waterui_accessibility::AccessibilityLabel,
            accessibility_role: waterui_accessibility::AccessibilityRole,
            accessibility_state: waterui_accessibility::AccessibilityState,
            accessibility_state_signal: waterui_accessibility::AccessibilityStateSignal,
            accessibility_value: waterui_accessibility::AccessibilityValue,
            transition: waterui_structural_transition::TransitionSpec,
            opacity: waterui_shape::filter::Opacity,
            scale: waterui_shape::style::Scale,
            rotation: waterui_shape::style::Rotation,
            offset: waterui_shape::style::Offset,
            shadow: waterui_shape::style::Shadow,
            border: waterui_shape::border::Border,
            clip_shape: waterui_shape::ClipShape,
            material_background: waterui_shape::background::MaterialBackground,
            glass_background: waterui_shape::background::GlassBackground,
            material_group: waterui_shape::background::MaterialGroup,
            floating: waterui_shape::floating::Floating,
            ignore_safe_area: waterui_layout::safe_area::IgnoreSafeArea,
            cursor: waterui_interaction::cursor::Cursor,
            draggable: waterui_interaction::drag_drop::Draggable,
            drop_destination: waterui_interaction::drag_drop::DropDestination,
            hittable: waterui_interaction::interaction::Hittable,
            focused: waterui_interaction::focus::Focused,
            secure: waterui_controls::secure::Secure,
            dynamic_range: waterui_controls::secure::DynamicRangeMetadata,
            frame_rate: waterui_core::animation::FrameRateVote,
            #[cfg(waterui_feature = "dialog")]
            dialog: waterui_controls::dialog::Dialog,
            #[cfg(waterui_feature = "navigation")]
            navigation_link_hint: waterui_backend_core::navigation::NavigationLinkHint,
            #[cfg(waterui_feature = "navigation")]
            navigation_transition_source: waterui_backend_core::navigation::NavigationTransitionSource,
            #[cfg(waterui_feature = "navigation")]
            navigation_transition_destination: waterui_backend_core::navigation::NavigationTransitionDestination,
        }

        fn mask<S: ChildView<Self>, C: ChildView<Self>>(
            &mut self,
            _: Metadata<waterui_shape::Mask<S>, C>,
            _: &Environment,
            (): (),
        ) {
        }

        fn anchored_overlay<S: ChildView<Self>, C: ChildView<Self>>(
            &mut self,
            _: Metadata<AnchoredOverlay<S>, C>,
            _: &Environment,
            (): (),
        ) {
        }

        fn sheet<S: ChildView<Self>, C: ChildView<Self>>(
            &mut self,
            _: Metadata<Sheet<S>, C>,
            _: &Environment,
            (): (),
        ) {
        }

        #[cfg(waterui_feature = "menu")]
        fn resolved_context_menu<S: ChildView<Self>, C: ChildView<Self>>(
            &mut self,
            _: Metadata<waterui_controls::context_menu::ResolvedContextMenu<S>, C>,
            _: &Environment,
            (): (),
        ) {
        }

        #[cfg(waterui_feature = "snackbar")]
        fn snackbar_host<S: ChildView<Self>, C: ChildView<Self>>(
            &mut self,
            _: Metadata<waterui_controls::snackbar::SnackbarHostConfig<S>, C>,
            _: &Environment,
            (): (),
        ) {
        }

        fn provided<T: NativeView + 'static>(&mut self, _: T, _: &Environment, (): ()) {
            report_unprovided("OtherBackend", core::any::type_name::<T>());
        }

        fn provided_dyn(&mut self, native: ErasedPayload<'_>, _: &Environment, (): ()) {
            report_unprovided("OtherBackend", native.type_name());
        }

        fn provided_metadata<K: MetadataKey, C: ChildView<Self>>(
            &mut self,
            _: Metadata<K, C>,
            _: &Environment,
            (): (),
        ) {
            report_unprovided("OtherBackend", core::any::type_name::<K>());
        }

        fn provided_metadata_dyn(&mut self, metadata: ErasedPayload<'_>, _: &Environment, (): ()) {
            report_unprovided("OtherBackend", metadata.type_name());
        }
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(
            expected = "AppleView<waterui_apple::apple_content::tests::DummyContent> reached OtherBackend"
        )
    )]
    fn static_dispatch_to_another_backend_is_a_contract_violation() {
        let view = AppleView::new(DummyContent::new(
            StretchAxis::None,
            Computed::constant(Str::from_static("Buy")),
        ));
        Realize::child(&mut OtherBackend, view, &Environment::new(), ());
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(
            expected = "AppleView<waterui_apple::apple_content::tests::DummyContent> reached OtherBackend"
        )
    )]
    fn erased_dispatch_to_another_backend_is_a_contract_violation() {
        let view = AnyView::new(AppleView::new(DummyContent::new(
            StretchAxis::None,
            Computed::constant(Str::from_static("Buy")),
        )));
        Realize::child(&mut OtherBackend, view, &Environment::new(), ());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn declared_semantics_are_reachable_through_waterui_testing() {
        use nami::{binding, signal::IntoComputed as _};
        use waterui::ViewExt as _;
        use waterui_testing::{Role, ui};

        let label = binding(Str::from_static("Buy with Apple Pay"));

        // Static dispatch on `SemanticRuntime` (`waterui-testing`):
        let label_for_static = label.clone();
        let mut app = ui().mount(move || {
            AppleView::new(DummyContent::new(
                StretchAxis::Horizontal,
                label_for_static.clone().into_computed(),
            ))
        });
        app.query()
            .role(Role::BUTTON)
            .label("Buy with Apple Pay")
            .assert_exists();

        // Reactive label update propagates without rebuilding the tree:
        label.set(Str::from_static("Subscribe with Apple Pay"));
        app.query()
            .role(Role::BUTTON)
            .label("Subscribe with Apple Pay")
            .assert_exists();

        // Erased (`AnyView`) dispatch + outer `.a11y_label` override:
        let mut erased_app = ui().mount(|| {
            AnyView::new(
                AppleView::new(DummyContent::new(
                    StretchAxis::None,
                    Computed::constant(Str::from_static("Default")),
                ))
                .a11y_label("Custom Checkout"),
            )
        });
        erased_app
            .query()
            .role(Role::BUTTON)
            .label("Custom Checkout")
            .assert_exists();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(
            expected = "AppleView<waterui_apple::apple_content::tests::DummyContent> reached Hydrolysis"
        )
    )]
    fn rendered_hydrolysis_rejects_apple_view_as_a_contract_violation() {
        use waterui_testing::{TestStyle, ui};

        let _app = ui().theme(TestStyle::new()).mount_offscreen(|| {
            AppleView::new(DummyContent::new(
                StretchAxis::None,
                Computed::constant(Str::from_static("Buy")),
            ))
        });
    }
}
