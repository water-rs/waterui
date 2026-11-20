//! The Apple backend's [`NativeBackend`]: each component's realization,
//! reached through static dispatch.

use waterui_backend_core::{
    ChildView, Environment, NativeBackend, NativeBackendFamily, report_unprovided,
};
use waterui_core::backend::{ErasedPayload, MapSlots};
use waterui_core::metadata::MetadataKey;
use waterui_core::{Metadata, NativeView};

use crate::contract::{NativeLeaf, RenderContext};

/// Realizes views into `AppKit`/`UIKit` leaves.
#[derive(Debug, Clone)]
pub struct AppleBackend {
    mtm: cocoa_ui::MainThreadMarker,
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    pub(crate) filter_fusion: Option<crate::components::filtered::FilterFusion>,
}

impl AppleBackend {
    /// A backend on the main thread `mtm` proves.
    #[must_use]
    pub const fn new(mtm: cocoa_ui::MainThreadMarker) -> Self {
        Self {
            mtm,
            #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
            filter_fusion: None,
        }
    }

    /// Opens the [`RenderContext`] for a payload or metadata wrapper, ending
    /// any enclosing filter-fusion walk before its own children realize.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    pub(crate) fn entry<'a>(&mut self, env: &'a Environment) -> RenderContext<'a> {
        self.end_filter_fusion();
        RenderContext::entry(env, self.mtm)
    }

    /// Opens the [`RenderContext`] for a payload or metadata wrapper.
    #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
    pub(crate) fn entry<'a>(&self, env: &'a Environment) -> RenderContext<'a> {
        RenderContext::entry(env, self.mtm)
    }

    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    fn end_filter_fusion(&mut self) {
        if let Some(fusion) = self.filter_fusion.take() {
            fusion.disarm();
        }
    }
}

impl NativeBackendFamily for AppleBackend {
    type Output = NativeLeaf;
    type Ctx = ();
    type Backend<'a> = Self;

    fn reborrow(backend: &mut Self) -> Self {
        backend.clone()
    }
}

#[cfg(waterui_feature = "webview")]
#[doc(hidden)]
pub use crate::components::webview::SelectedWebViewController;
#[cfg(feature = "webview")]
#[doc(hidden)]
pub use crate::components::webview::{AppleWebViewController, WkWebViewHandle};

impl NativeBackend for AppleBackend {
    type Output = NativeLeaf;
    type Ctx = ();
    type Family = Self;
    #[cfg(waterui_feature = "webview")]
    type WebViewController = crate::components::webview::SelectedWebViewController;

    fn with_family<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        f(self)
    }

    #[cfg(waterui_feature = "webview")]
    fn web_view_controller(&self) -> Self::WebViewController {
        crate::components::webview::selected_controller()
    }

    fn empty(&mut self, native: (), env: &Environment, (): ()) -> NativeLeaf {
        crate::components::empty::realize_empty(native, &self.entry(env))
    }

    fn text(&mut self, native: waterui::text::TextConfig, env: &Environment, (): ()) -> NativeLeaf {
        crate::components::text::realize_text_config(native, &self.entry(env))
    }

    fn label(&mut self, native: waterui_core::Str, env: &Environment, (): ()) -> NativeLeaf {
        #[cfg(feature = "plain")]
        let realize = crate::components::plain::realize_str;
        #[cfg(not(feature = "plain"))]
        let realize = crate::components::text::realize_str;
        realize(native, &self.entry(env))
    }

    fn color(&mut self, native: waterui::Color, env: &Environment, (): ()) -> NativeLeaf {
        crate::components::resolved_color::realize_color(native, &self.entry(env))
    }

    fn spacer(&mut self, native: waterui::layout::Spacer, env: &Environment, (): ()) -> NativeLeaf {
        crate::components::spacer::realize_spacer(native, &self.entry(env))
    }

    fn divider(
        &mut self,
        native: waterui::layout::Divider,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
        self.end_filter_fusion();
        crate::dispatch::render(native.composed(env), env, self.mtm)
    }

    fn fixed_container<C: ChildView<Self>>(
        &mut self,
        native: waterui::layout::container::FixedContainer<Vec<C>>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let ctx = self.entry(env);
        crate::components::fixed_container::realize_fixed_container(
            native,
            |child| child.realize(self, env, ()),
            &ctx,
        )
    }

    fn lazy_container<C: ChildView<Self>>(
        &mut self,
        native: waterui::layout::container::LazyContainer<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::container::realize_lazy_container(native, &self.entry(env))
    }

    fn scroll<C: ChildView<Self>>(
        &mut self,
        native: waterui::layout::scroll::ScrollView<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let ctx = self.entry(env);
        crate::components::scroll::realize_scroll_view(
            native,
            |content| content.realize(self, env, ()),
            &ctx,
        )
    }

    fn dynamic<C: ChildView<Self>>(
        &mut self,
        native: waterui_core::Dynamic<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::dynamic::realize_dynamic(native, &self.entry(env))
    }

    fn window<C: ChildView<Self>>(
        &mut self,
        native: waterui::window::Window<waterui::window::WindowSlot<C>, Option<C>>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        #[cfg(target_os = "macos")]
        crate::windows::show_from_tree(native, env, self.mtm);
        #[cfg(target_os = "ios")]
        {
            let _ = native;
            crate::windows::show_unsupported_on_ios();
        }
        crate::components::empty::realize_empty((), &RenderContext::entry(env, self.mtm))
    }

    fn shape(
        &mut self,
        native: waterui::shape::ResolvedShape,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::resolved_shape::realize_resolved_shape(native, &self.entry(env))
    }

    fn icon(&mut self, native: waterui::icon::SystemIcon, env: &Environment, (): ()) -> NativeLeaf {
        crate::components::image::realize_system_icon(native, &self.entry(env))
    }

    fn gradient(
        &mut self,
        native: waterui::graphics::Gradient,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::resolved_gradient::realize_gradient(native, &self.entry(env))
    }

    fn button<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::button::ButtonConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::button::realize_button_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    fn badge<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::badge::BadgeConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::badge::realize_badge_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    fn progress<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::progress::ProgressConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::progress::realize_progress_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "toggle")]
    fn toggle<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::toggle::ToggleConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::toggle::realize_toggle_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "slider")]
    fn slider<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::slider::SliderConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::slider::realize_slider_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "stepper")]
    fn stepper<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::stepper::StepperConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::stepper::realize_stepper_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "text-field")]
    fn text_field<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::text_field::ResolvedTextFieldConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::text_field::realize_resolved_text_field_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "secure-field")]
    fn secure_field<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::secure::SecureFieldConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::secure_field::realize_secure_field_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "picker")]
    fn picker<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::picker::PickerConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::picker::realize_picker_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "color-picker")]
    fn color_picker<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::picker::color::ColorPickerConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::color_picker::realize_color_picker_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "date-picker")]
    fn date_picker<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::picker::date::DatePickerConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::date_picker::realize_date_picker_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "multi-date-picker")]
    fn multi_date_picker<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::picker::multi_date::MultiDatePickerConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::multi_date_picker::realize_multi_date_picker_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "calendar")]
    fn calendar<C: ChildView<Self>>(
        &mut self,
        native: waterui::form::calendar::CalendarConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::calendar::realize_calendar_config(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "menu")]
    fn menu<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::menu::ResolvedMenu<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::menu::realize_resolved_menu(
            &native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "list")]
    fn list<C: ChildView<Self>>(
        &mut self,
        native: waterui::component::list::ListConfig<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::list::realize_list_config(native, &self.entry(env))
    }

    #[cfg(waterui_feature = "table")]
    fn table(
        &mut self,
        native: waterui::component::table::TableConfig,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::table::realize_table_config(native, &self.entry(env))
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_view<C: ChildView<Self>>(
        &mut self,
        native: waterui::navigation::NavigationView<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::navigation::nav_view::realize_navigation_view(
            native.map_content(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_stack<C: ChildView<Self>>(
        &mut self,
        native: waterui::navigation::NavigationStackLayout<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::navigation::stack::realize_navigation_stack(
            native.map_slot(C::into_child),
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_split(
        &mut self,
        native: waterui::navigation::split::NavigationSplitLayout,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::navigation::split::realize_navigation_split_layout(
            native,
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "navigation")]
    fn tabs<C: ChildView<Self>>(
        &mut self,
        native: waterui::navigation::tab::TabsLayout<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::navigation::tabs::realize_tabs_layout(
            native.map_slots(C::into_child),
            &self.entry(env),
        )
    }

    // Every application can build a canvas, so the method exists whether or
    // not this build carries the `canvas` realization.
    fn canvas(
        &mut self,
        native: waterui::graphics::canvas::CanvasView,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        #[cfg(feature = "canvas")]
        return crate::components::canvas::realize_canvas_view(native, &self.entry(env));
        #[cfg(not(feature = "canvas"))]
        {
            drop(native);
            report_unprovided(
                "waterui-apple without its `canvas` feature",
                "waterui_graphics::canvas::CanvasView",
            );
            self.empty((), env, ())
        }
    }

    #[cfg(waterui_feature = "effects")]
    fn filtered<C: ChildView<Self>>(
        &mut self,
        native: waterui::graphics::backend::FilteredView<C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
        return crate::components::filtered::realize_filtered_view(self, native, env);
        #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
        {
            drop(native);
            report_unprovided(
                "waterui-apple without its `applied_filter` or `view_effect` feature",
                "waterui_graphics::FilteredView",
            );
            self.empty((), env, ())
        }
    }

    #[cfg(waterui_feature = "gpu")]
    fn gpu(&mut self, native: waterui::GpuView, env: &Environment, (): ()) -> NativeLeaf {
        crate::components::gpu_surface::realize_gpu_view(native, &self.entry(env))
    }

    #[cfg(waterui_feature = "gpu")]
    fn external_frame(
        &mut self,
        native: waterui::graphics::gpu::external::ExternalFrameView,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::gpu_surface::realize_external_frame_view(native, &self.entry(env))
    }

    #[cfg(waterui_feature = "gpu")]
    fn scene(
        &mut self,
        native: waterui::graphics::backend::SceneView,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::gpu_surface::realize_scene_view(native, &self.entry(env))
    }

    #[cfg(waterui_feature = "gpu")]
    fn picture(
        &mut self,
        native: waterui::graphics::backend::Picture,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::picture::realize_picture(native, &self.entry(env))
    }

    #[cfg(waterui_feature = "video")]
    fn video(
        &mut self,
        native: waterui::video::video::NativeVideoConfig,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::video::realize_native_video_config(native, &self.entry(env))
    }

    #[cfg(waterui_feature = "video")]
    fn video_player(
        &mut self,
        native: waterui::video::video::NativeVideoPlayerConfig,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::video::realize_native_video_player_config(native, &self.entry(env))
    }

    #[cfg(waterui_feature = "webview")]
    fn web_view(
        &mut self,
        native: waterui::webview::WebViewOpen,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        crate::components::webview::realize_web_view(native, &self.entry(env))
    }

    #[cfg(waterui_feature = "map")]
    fn map(&mut self, native: waterui::map::MapConfig, env: &Environment, (): ()) -> NativeLeaf {
        crate::components::map::realize_map_config(native, &self.entry(env))
    }

    fn environment<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<Environment, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::with_env::realize_environment_metadata(metadata, &self.entry(env))
    }

    fn retain<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::metadata::Retain, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::retain::realize_retain_metadata(metadata, &self.entry(env))
    }

    fn layout_priority<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::layout::LayoutPriority, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::layout_priority::realize_layout_priority_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn lifecycle_hook<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::event::LifeCycleHook, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::lifecycle_hook::realize_life_cycle_hook_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn on_event<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::event::OnEvent, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::on_event::realize_on_event_metadata(metadata, &self.entry(env))
    }

    fn on_key_press<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::key::OnKeyPress, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::on_key_press::realize_on_key_press_metadata(metadata, &self.entry(env))
    }

    fn gesture_observer<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::gesture::GestureObserver, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::gesture::realize_gesture_observer_metadata(metadata, &self.entry(env))
    }

    fn selected<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::interaction::Selected, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_metadata::realize_selected_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn accessibility_children<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::accessibility::AccessibilityChildren, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_metadata::realize_accessibility_children_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn accessibility_geometry<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::accessibility::AccessibilityGeometry, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_metadata::realize_accessibility_geometry_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn accessibility_hidden<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::accessibility::AccessibilityHidden, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_metadata::realize_accessibility_hidden_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn accessibility_identifier<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::accessibility::AccessibilityIdentifier, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_identifier::realize_accessibility_identifier_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn accessibility_label<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::accessibility::AccessibilityLabel, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_metadata::realize_accessibility_label_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn accessibility_role<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::accessibility::AccessibilityRole, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_metadata::realize_accessibility_role_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn accessibility_state<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::accessibility::AccessibilityState, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_metadata::realize_accessibility_state_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn accessibility_state_signal<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::accessibility::AccessibilityStateSignal, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_metadata::realize_accessibility_state_signal_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn accessibility_value<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_core::accessibility::AccessibilityValue, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::accessibility_metadata::realize_accessibility_value_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn transition<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui_structural_transition::TransitionSpec, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::transition::realize_transition_spec_metadata(metadata, &self.entry(env))
    }

    fn opacity<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::filter::Opacity, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::opacity::realize_opacity_metadata(metadata, &self.entry(env))
    }

    fn scale<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::style::Scale, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::scale::realize_scale_metadata(metadata, &self.entry(env))
    }

    fn rotation<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::style::Rotation, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::rotation::realize_rotation_metadata(metadata, &self.entry(env))
    }

    fn offset<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::style::Offset, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::offset::realize_offset_metadata(metadata, &self.entry(env))
    }

    fn shadow<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::style::Shadow, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::shadow::realize_shadow_metadata(metadata, &self.entry(env))
    }

    fn border<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::border::Border, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::border::realize_border_metadata(metadata, &self.entry(env))
    }

    fn clip_shape<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::shape::ClipShape, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::clip_shape::realize_clip_shape_metadata(metadata, &self.entry(env))
    }

    fn material_background<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::background::MaterialBackground, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::material_background::realize_material_background_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn glass_background<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::background::GlassBackground, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::glass_background::realize_glass_background_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn material_group<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::background::MaterialGroup, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::material_background::realize_material_group_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn ignore_safe_area<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::layout::safe_area::IgnoreSafeArea, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::ignore_safe_area::realize_ignore_safe_area_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn cursor<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::cursor::Cursor, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::cursor::realize_cursor_metadata(metadata, &self.entry(env))
    }

    fn draggable<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::drag_drop::Draggable, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::draggable::realize_draggable_metadata(metadata, &self.entry(env))
    }

    fn drop_destination<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::drag_drop::DropDestination, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::drop_destination::realize_drop_destination_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn hittable<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::interaction::Hittable, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::hittable::realize_hittable_metadata(metadata, &self.entry(env))
    }

    fn focused<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::component::focus::Focused, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::focused::realize_focused_metadata(metadata, &self.entry(env))
    }

    fn secure<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::secure::Secure, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::secure::realize_secure_metadata(metadata, &self.entry(env))
    }

    fn standard_dynamic_range<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::secure::StandardDynamicRange, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::dynamic_range::realize_standard_dynamic_range_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn high_dynamic_range<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::secure::HighDynamicRange, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::dynamic_range::realize_high_dynamic_range_metadata(
            metadata,
            &self.entry(env),
        )
    }

    fn anchored_overlay<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::anchored_overlay::AnchoredOverlay, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::anchored_overlay::realize_anchored_overlay_metadata(
            metadata,
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "menu")]
    fn resolved_context_menu<S: ChildView<Self>, C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::metadata::context_menu::ResolvedContextMenu<S>, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let Metadata { content, value } = metadata;
        let metadata = Metadata::new(content.into_child(), value.map_slots(S::into_child));
        crate::components::context_menu::realize_resolved_context_menu_metadata(
            metadata,
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "dialog")]
    fn dialog<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::dialog::Dialog, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::dialog::realize_dialog_metadata(metadata, &self.entry(env))
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_link_hint<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::navigation::NavigationLinkHint, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::navigation::metadata::realize_navigation_link_hint_metadata(
            metadata,
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_transition_source<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::navigation::NavigationTransitionSource, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::navigation::metadata::realize_navigation_transition_source_metadata(
            metadata,
            &self.entry(env),
        )
    }

    #[cfg(waterui_feature = "navigation")]
    fn navigation_transition_destination<C: ChildView<Self>>(
        &mut self,
        metadata: Metadata<waterui::navigation::NavigationTransitionDestination, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        let metadata = metadata.map_content(C::into_child);
        crate::components::navigation::metadata::realize_navigation_transition_destination_metadata(
            metadata,
            &self.entry(env),
        )
    }

    // waterui-apple provides no components of its own yet: every payload
    // outside `NativeBackend` is a contract violation.

    fn provided<T: NativeView + 'static>(
        &mut self,
        _native: T,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        report_unprovided("waterui-apple", core::any::type_name::<T>());
        self.empty((), env, ())
    }

    fn provided_dyn(&mut self, native: ErasedPayload<'_>, env: &Environment, (): ()) -> NativeLeaf {
        report_unprovided("waterui-apple", native.type_name());
        self.empty((), env, ())
    }

    fn provided_metadata<K: MetadataKey, C: ChildView<Self>>(
        &mut self,
        _metadata: Metadata<K, C>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        report_unprovided("waterui-apple", core::any::type_name::<K>());
        self.empty((), env, ())
    }

    fn provided_metadata_dyn(
        &mut self,
        metadata: ErasedPayload<'_>,
        env: &Environment,
        (): (),
    ) -> NativeLeaf {
        report_unprovided("waterui-apple", metadata.type_name());
        self.empty((), env, ())
    }
}
