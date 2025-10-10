use crate::{dom::DomRoot, error::WebError};
use waterui_core::{AnyView, Environment};

#[cfg(target_arch = "wasm32")]
use waterui_core::Str;

#[cfg(target_arch = "wasm32")]
use waterui_render_utils::ViewDispatcher;

#[cfg(target_arch = "wasm32")]
use web_sys::{Document, Element};

/// Internal state machine for the web renderer.
#[derive(Debug, Default, Clone, Copy)]
pub enum WebRendererState {
    /// The renderer has been created but not mounted to the DOM yet.
    #[default]
    Initialising,
    /// The renderer has mounted its root container into the DOM.
    Mounted,
}

/// Responsible for routing WaterUI views through the dispatcher and producing DOM updates.
#[derive(Debug)]
pub struct WebRenderer {
    root: DomRoot,
    state: WebRendererState,
    #[cfg(target_arch = "wasm32")]
    dispatcher: ViewDispatcher<DispatcherState, RenderContext, Result<(), WebError>>,
}

impl WebRenderer {
    /// Creates a new renderer instance bound to the provided DOM root.
    #[must_use]
    pub fn new(root: DomRoot) -> Self {
        Self {
            root,
            state: WebRendererState::Initialising,
            #[cfg(target_arch = "wasm32")]
            dispatcher: initialise_dispatcher(),
        }
    }

    /// Returns the current renderer state.
    #[must_use]
    pub const fn state(&self) -> WebRendererState {
        self.state
    }

    /// Mounts the renderer, preparing the DOM for updates.
    pub fn mount(&mut self) -> Result<(), WebError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = self;
            return Err(WebError::Unsupported);
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.root.set_class_name("waterui-web-root")?;
            self.root.clear()?;
            self.state = WebRendererState::Mounted;
            Ok(())
        }
    }

    /// Renders the provided [`AnyView`]. The current implementation paints a placeholder UI
    /// until the dispatcher learns how to translate core views into DOM nodes.
    pub fn render(&mut self, _environment: &Environment, view: AnyView) -> Result<(), WebError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = _environment;
            let _ = view;
            return Err(WebError::Unsupported);
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.root.set_class_name("waterui-web-root")?;
            self.root.clear()?;

            let document = self.root.document();

            let surface = document.create_element("section")?;
            surface.set_class_name("waterui-surface");
            self.root.element().append_child(&surface)?;

            let context = RenderContext::new(document.clone(), surface);
            self.dispatcher.dispatch_any(view, _environment, context)
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Default)]
struct DispatcherState;

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone)]
struct RenderContext {
    document: Document,
    parent: Element,
}

#[cfg(target_arch = "wasm32")]
impl RenderContext {
    fn new(document: Document, parent: Element) -> Self {
        Self { document, parent }
    }

    #[allow(dead_code)]
    fn document(&self) -> &Document {
        &self.document
    }

    #[allow(dead_code)]
    fn parent(&self) -> &Element {
        &self.parent
    }
}

#[cfg(target_arch = "wasm32")]
fn initialise_dispatcher() -> ViewDispatcher<DispatcherState, RenderContext, Result<(), WebError>> {
    let mut dispatcher = ViewDispatcher::default();
    dispatcher.register(|state, context, view: Str, _env| {
        let _ = state;
        let _ = context;
        let _ = view;
        todo!("Render text views into DOM nodes");
    });
    dispatcher.register(|state, context, _view: (), _env| {
        let _ = state;
        let _ = context;
        todo!("Render unit views into DOM nodes");
    });
    dispatcher
}
