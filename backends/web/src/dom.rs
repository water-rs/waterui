use crate::error::WebError;

#[cfg(target_arch = "wasm32")]
use std::sync::OnceLock;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{JsCast, JsValue};

#[cfg(target_arch = "wasm32")]
use web_sys::{Document, Element, HtmlElement, Window};

#[cfg(not(target_arch = "wasm32"))]
/// Placeholder DOM root for non-wasm targets.
#[derive(Debug, Clone)]
pub struct DomRoot;

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone)]
pub struct DomRoot {
    document: Document,
    element: Element,
}

impl DomRoot {
    /// Creates a [`DomRoot`] pointing at the provided element id.
    pub fn new(root_id: Option<&str>, inject_styles: bool) -> Result<Self, WebError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = root_id;
            let _ = inject_styles;
            return Err(WebError::Unsupported);
        }

        #[cfg(target_arch = "wasm32")]
        {
            let window: Window = web_sys::window().ok_or(WebError::DomUnavailable)?;
            let document: Document = window.document().ok_or(WebError::DomUnavailable)?;

            if inject_styles {
                inject_stylesheet(&document)?;
            }

            let element = if let Some(id) = root_id {
                document
                    .get_element_by_id(id)
                    .ok_or_else(|| WebError::RootNotFound(id.to_string()))?
            } else {
                let body = document.body().ok_or(WebError::DomUnavailable)?;
                let host = document.create_element("div")?;
                host.set_id("waterui-web-root");
                body.append_child(&host)?;
                host
            };

            Ok(Self { document, element })
        }
    }

    /// Returns the DOM element representing the mounting point.
    #[must_use]
    pub fn element(&self) -> &Element {
        #[cfg(not(target_arch = "wasm32"))]
        {
            panic!("DomRoot::element is only available on wasm32 targets");
        }

        #[cfg(target_arch = "wasm32")]
        {
            &self.element
        }
    }

    /// Returns the owning document.
    #[must_use]
    pub fn document(&self) -> &Document {
        #[cfg(not(target_arch = "wasm32"))]
        {
            panic!("DomRoot::document is only available on wasm32 targets");
        }

        #[cfg(target_arch = "wasm32")]
        {
            &self.document
        }
    }

    /// Clears the mounting element.
    pub fn clear(&self) -> Result<(), WebError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            return Err(WebError::Unsupported);
        }

        #[cfg(target_arch = "wasm32")]
        {
            while let Some(child) = self.element.first_child() {
                self.element.remove_child(&child)?;
            }
            Ok(())
        }
    }

    /// Sets the CSS class name for the root element.
    pub fn set_class_name(&self, class_name: &str) -> Result<(), WebError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = class_name;
            return Err(WebError::Unsupported);
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.element.set_class_name(class_name);
            Ok(())
        }
    }

    /// Converts the element into an [`HtmlElement`].
    #[cfg(target_arch = "wasm32")]
    pub fn as_html_element(&self) -> Result<HtmlElement, WebError> {
        self.element
            .clone()
            .dyn_into::<HtmlElement>()
            .map_err(WebError::from)
    }
}

#[cfg(target_arch = "wasm32")]
fn inject_stylesheet(document: &Document) -> Result<(), WebError> {
    static STYLE_CACHE: OnceLock<()> = OnceLock::new();

    if document.get_element_by_id("waterui-web-styles").is_some() {
        return Ok(());
    }

    STYLE_CACHE
        .get_or_try_init(|| {
            let style = document.create_element("style")?;
            style.set_id("waterui-web-styles");
            style.set_attribute("data-waterui", "true")?;
            style.set_inner_html(include_str!("../styles/default.css"));

            if let Some(head) = document.head() {
                head.append_child(&style)?;
            } else if let Some(body) = document.body() {
                body.prepend_with_node_1(&style)?;
            } else {
                return Err(WebError::DomUnavailable);
            }

            Ok(())
        })
        .map(|_| ())
}
