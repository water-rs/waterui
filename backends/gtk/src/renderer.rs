//! View renderer that dispatches `WaterUI` views to GTK widgets.

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_backend_core::ViewDispatcher;
use waterui_core::dynamic::Dynamic;
use waterui_core::{IgnorableMetadata, Metadata, Retain, Str};
use waterui_core::metadata::MetadataKey;
use waterui_core::{AnyView, Environment, Native, View};
use waterui::component::list::ListConfig;
use waterui::component::progress::ProgressConfig;
use waterui::prelude::Divider;
use waterui_controls::button::ButtonConfig;
use waterui_controls::slider::SliderConfig;
use waterui_controls::text_field::TextFieldConfig;
use waterui_controls::stepper::StepperConfig;
use waterui_controls::toggle::ToggleConfig;
use waterui_form::picker::PickerConfig;
use waterui_form::secure::SecureFieldConfig;
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::padding::Padding;
use waterui_layout::scroll::ScrollView;
use waterui_layout::spacer::Spacer;
use waterui_media::photo::PhotoConfig;
use waterui_navigation::tab::Tabs;
use waterui_navigation::{NavigationStack, NavigationView};
use waterui_text::TextConfig;
use waterui_color::Color;
#[cfg(feature = "gpu")]
use waterui_graphics::gpu_surface::GpuSurface;

use crate::component::GtkComponent;

/// Context passed to component renderers.
#[derive(Debug, Clone)]
pub struct RenderContext {
    /// Reference to the renderer for recursive rendering.
    /// This is a raw pointer because we can't have self-referential borrows.
    renderer_ptr: *mut GtkRenderer,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            renderer_ptr: std::ptr::null_mut(),
        }
    }
}

impl RenderContext {
    /// Creates a context with a renderer reference.
    fn with_renderer(renderer: &mut GtkRenderer) -> Self {
        Self {
            renderer_ptr: renderer as *mut GtkRenderer,
        }
    }

    /// Gets a mutable reference to the renderer.
    ///
    /// # Safety
    ///
    /// The caller must ensure the renderer pointer is valid.
    pub(crate) unsafe fn renderer(&self) -> Option<&mut GtkRenderer> {
        if self.renderer_ptr.is_null() {
            None
        } else {
            // SAFETY: Caller ensures the pointer is valid for the duration of the borrow.
            Some(unsafe { &mut *self.renderer_ptr })
        }
    }
}

/// GTK renderer that converts `WaterUI` views to GTK widgets.
#[derive(Debug)]
pub struct GtkRenderer {
    dispatcher: ViewDispatcher<(), RenderContext, Widget>,
}

impl GtkRenderer {
    /// Creates a new GTK renderer with all component handlers registered.
    #[must_use]
    pub fn new() -> Self {
        let mut dispatcher = ViewDispatcher::new();

        // Register component handlers
        Self::register_components(&mut dispatcher);

        Self { dispatcher }
    }

    /// Renders a view to a GTK widget.
    pub fn render<V: View>(&mut self, view: V, env: &Environment) -> Widget {
        let ctx = RenderContext::with_renderer(self);
        self.dispatcher.dispatch(view, env, ctx)
    }

    /// Renders an `AnyView` to a GTK widget.
    pub fn render_any(&mut self, view: AnyView, env: &Environment) -> Widget {
        let ctx = RenderContext::with_renderer(self);
        self.dispatcher.dispatch_any(view, env, ctx)
    }

    fn register_components(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        // Register Native<T> wrapped components
        Self::register_native::<TextConfig>(dispatcher);
        Self::register_native::<Spacer>(dispatcher);
        Self::register_native::<FixedContainer>(dispatcher);
        Self::register_native::<LazyContainer>(dispatcher);
        Self::register_native::<ButtonConfig>(dispatcher);
        Self::register_native::<ToggleConfig>(dispatcher);
        Self::register_native::<SliderConfig>(dispatcher);
        Self::register_native::<TextFieldConfig>(dispatcher);
        Self::register_native::<ProgressConfig>(dispatcher);
        Self::register_native::<StepperConfig>(dispatcher);
        Self::register_native::<ScrollView>(dispatcher);
        Self::register_native::<Tabs>(dispatcher);
        Self::register_native::<Color>(dispatcher);
        Self::register_native::<ListConfig>(dispatcher);
        Self::register_native::<PhotoConfig>(dispatcher);
        Self::register_native::<SecureFieldConfig>(dispatcher);
        Self::register_native::<PickerConfig>(dispatcher);

        // Register Dynamic for reactive content
        Self::register::<Native<Dynamic>>(dispatcher);

        // Register GPU surface (requires "gpu" feature)
        #[cfg(feature = "gpu")]
        Self::register_native::<GpuSurface>(dispatcher);

        // Register views that implement View directly
        Self::register::<Divider>(dispatcher);
        Self::register::<NavigationView>(dispatcher);
        Self::register::<NavigationStack<(), ()>>(dispatcher);

        // Register metadata handlers
        Self::register_metadata_handlers(dispatcher);

        // Register Padding to apply margins (before it becomes FixedContainer)
        Self::register_padding_handler(dispatcher);

        // Register Str directly (before it wraps into Native<Str>)
        Self::register_str_handler(dispatcher);

        // Register unit type () as empty widget
        Self::register_unit_handler(dispatcher);
    }

    /// Registers a handler for `Padding` that applies GTK margins.
    fn register_padding_handler(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<Padding>(|_state, ctx, padding, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let (edges, content) = padding.into_inner();

            // Render the content
            let widget = renderer.render_any(content, env);

            // Apply margins from EdgeInsets
            #[allow(clippy::cast_possible_truncation)]
            {
                widget.set_margin_top(edges.top() as i32);
                widget.set_margin_bottom(edges.bottom() as i32);
                widget.set_margin_start(edges.leading() as i32);
                widget.set_margin_end(edges.trailing() as i32);
            }

            widget
        });
    }

    /// Registers a handler for `Str` that renders it as a GTK Label.
    fn register_str_handler(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<Str>(|_state, _ctx, s, _env| {
            let label = gtk4::Label::new(Some(s.as_str()));
            // Let text maintain natural width - layout system handles sizing
            label.upcast()
        });
    }

    /// Registers a handler for unit type `()` as an empty widget.
    fn register_unit_handler(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<Native<()>>(|_state, _ctx, _unit, _env| {
            // Return an empty widget (invisible box with no children)
            let empty = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            empty.set_visible(false);
            empty.upcast()
        });
    }

    /// Registers handlers for metadata wrapper views.
    fn register_metadata_handlers(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        use waterui::background::{Background, ForegroundColor};
        use waterui::component::focus::Focused;
        use waterui::gesture::GestureObserver;
        use waterui::metadata::secure::Secure;
        use waterui::style::Shadow;
        use waterui_core::event::OnEvent;
        use waterui_layout::safe_area::IgnoreSafeArea;
        use waterui::accessibility::{AccessibilityLabel, AccessibilityRole};

        // Metadata<Environment> - merge environment and render content
        dispatcher.register::<Metadata<Environment>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            // Merge the metadata environment into the current environment
            let mut merged_env = env.clone();
            // Copy values from the metadata environment
            // For now, we just use the content's environment as-is
            // A full implementation would merge specific values
            renderer.render_any(metadata.content, &merged_env)
        });

        // Metadata<Retain> - just render content (the value stays alive in the struct)
        dispatcher.register::<Metadata<Retain>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });

        // Metadata<Background> - apply background color/style and render content
        dispatcher.register::<Metadata<Background>>(|_state, ctx, metadata, env| {
            use nami::Signal;
            use waterui::background::Background;
            use waterui_core::resolve::Resolvable;

            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);

            // Apply background color if it's a color background
            if let Background::Color(color_signal) = metadata.value {
                let color = color_signal.get();
                // Resolve the color to get RGB values
                let resolved = color.resolve(env).get();
                let srgb = resolved.to_srgb();

                // Create inline CSS for background color
                let css = format!(
                    "* {{ background-color: rgba({}, {}, {}, {}); }}",
                    (srgb.red * 255.0) as u8,
                    (srgb.green * 255.0) as u8,
                    (srgb.blue * 255.0) as u8,
                    resolved.opacity
                );

                let provider = gtk4::CssProvider::new();
                provider.load_from_data(&css);

                widget
                    .style_context()
                    .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
            }

            widget
        });

        // Metadata<ForegroundColor> - apply foreground color and render content
        dispatcher.register::<Metadata<ForegroundColor>>(|_state, ctx, metadata, env| {
            use nami::Signal;
            use waterui_core::resolve::Resolvable;

            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);

            // Apply foreground color using CSS
            let color = metadata.value.color.get();
            let resolved = color.resolve(env).get();
            let srgb = resolved.to_srgb();

            let css = format!(
                "* {{ color: rgba({}, {}, {}, {}); }}",
                (srgb.red * 255.0) as u8,
                (srgb.green * 255.0) as u8,
                (srgb.blue * 255.0) as u8,
                resolved.opacity
            );

            let provider = gtk4::CssProvider::new();
            provider.load_from_data(&css);

            widget
                .style_context()
                .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);

            widget
        });

        // Metadata<Shadow> - apply shadow and render content
        dispatcher.register::<Metadata<Shadow>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);
            // TODO: Apply shadow styling (GTK CSS or custom drawing)
            widget
        });

        // Metadata<Focused> - handle focus state
        dispatcher.register::<Metadata<Focused>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);
            // TODO: Set up focus handling
            widget
        });

        // Metadata<Secure> - mark content as secure (e.g., password fields)
        dispatcher.register::<Metadata<Secure>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });

        // Metadata<OnEvent> - handle events
        dispatcher.register::<Metadata<OnEvent>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);
            // TODO: Connect event handlers
            widget
        });

        // Metadata<GestureObserver> - handle gestures
        dispatcher.register::<Metadata<GestureObserver>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);
            // TODO: Connect gesture recognizers
            widget
        });

        // Metadata<IgnoreSafeArea> - ignore safe area insets
        dispatcher.register::<Metadata<IgnoreSafeArea>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });

        // IgnorableMetadata handlers - these can safely just render the content
        Self::register_ignorable_metadata::<AccessibilityLabel>(dispatcher);
        Self::register_ignorable_metadata::<AccessibilityRole>(dispatcher);
    }

    /// Registers a handler for `IgnorableMetadata<T>` that just renders the content.
    fn register_ignorable_metadata<T: MetadataKey>(
        dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>,
    ) {
        dispatcher.register::<IgnorableMetadata<T>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });
    }

    /// Registers a `Native<T>` wrapped component with the dispatcher.
    fn register_native<T: waterui_core::NativeView + 'static>(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>)
    where
        Native<T>: GtkComponent,
    {
        Self::register::<Native<T>>(dispatcher);
    }

    /// Registers a `GtkComponent` view type with the dispatcher.
    fn register<V: GtkComponent>(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<V>(|_state, ctx, view, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            view.render(env, renderer)
        });
    }
}

impl Default for GtkRenderer {
    fn default() -> Self {
        Self::new()
    }
}
