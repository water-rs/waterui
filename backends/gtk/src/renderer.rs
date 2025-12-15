//! View renderer that dispatches `WaterUI` views to GTK widgets.

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_backend_core::ViewDispatcher;
use waterui_core::{AnyView, Environment, Native, View};
use waterui::component::progress::ProgressConfig;
use waterui::prelude::Divider;
use waterui_controls::button::ButtonConfig;
use waterui_controls::slider::SliderConfig;
use waterui_controls::text_field::TextFieldConfig;
use waterui_controls::toggle::ToggleConfig;
use waterui_layout::container::FixedContainer;
use waterui_layout::scroll::ScrollView;
use waterui_layout::spacer::Spacer;
use waterui_navigation::tab::Tabs;
use waterui_navigation::{NavigationStack, NavigationView};
use waterui_text::TextConfig;

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
            Some(&mut *self.renderer_ptr)
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
        Self::register_native::<ButtonConfig>(dispatcher);
        Self::register_native::<ToggleConfig>(dispatcher);
        Self::register_native::<SliderConfig>(dispatcher);
        Self::register_native::<TextFieldConfig>(dispatcher);
        Self::register_native::<ProgressConfig>(dispatcher);
        Self::register_native::<ScrollView>(dispatcher);
        Self::register_native::<Tabs>(dispatcher);

        // Register views that implement View directly
        Self::register::<Divider>(dispatcher);
        Self::register::<NavigationView>(dispatcher);
        Self::register::<NavigationStack<(), ()>>(dispatcher);
    }

    /// Registers a `Native<T>` wrapped component with the dispatcher.
    fn register_native<T: 'static>(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>)
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
