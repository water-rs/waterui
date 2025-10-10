use crate::error::WebError;

use std::sync::OnceLock;

use wasm_bindgen::{JsCast, JsValue};

use web_sys::{Document, Element, HtmlElement, Window};

#[derive(Debug, Clone)]
pub struct DomRoot {
    document: Document,
    element: Element,
}

impl DomRoot {
    /// Creates a [`DomRoot`] pointing at the provided element id.
    pub fn new(root_id: Option<&str>, inject_styles: bool) -> Result<Self, WebError> {
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
            host.set_id("waterui-root");
            body.append_child(&host)?;
            host
        };

        Ok(Self { document, element })
    }

    /// Returns the DOM element representing the mounting point.
    #[must_use]
    pub const fn element(&self) -> &Element {
        &self.element
    }

    /// Returns the owning document.
    #[must_use]
    pub const fn document(&self) -> &Document {
        &self.document
    }

    /// Clears the mounting element.
    pub fn clear(&self) -> Result<(), WebError> {
        while let Some(child) = self.element.first_child() {
            self.element.remove_child(&child)?;
        }
        Ok(())
    }

    /// Sets the CSS class name for the root element.
    pub fn set_class_name(&self, class_name: &str) {
        self.element.set_class_name(class_name);
    }

    /// Converts the element into an [`HtmlElement`].
    pub fn as_html_element(&self) -> Result<HtmlElement, WebError> {
        self.element
            .clone()
            .dyn_into::<HtmlElement>()
            .map_err(|e| WebError::from(JsValue::from(e)))
    }
}

fn inject_stylesheet(document: &Document) -> Result<(), WebError> {
    static STYLE_CACHE: OnceLock<Result<(), WebError>> = OnceLock::new();

    if document.get_element_by_id("waterui-web-styles").is_some() {
        return Ok(());
    }

    STYLE_CACHE
        .get_or_init(|| {
            let style = match document.create_element("style") {
                Ok(s) => s,
                Err(e) => return Err(e.into()),
            };
            style.set_id("waterui-web-styles");
            if let Err(e) = style.set_attribute("data-waterui", "true") {
                return Err(e.into());
            }
            style.set_inner_html(include_str!("../styles/default.css"));

            if let Some(head) = document.head() {
                if let Err(e) = head.append_child(&style) {
                    return Err(e.into());
                }
            } else if let Some(body) = document.body() {
                if let Err(e) = body.prepend_with_node_1(&style) {
                    return Err(e.into());
                }
            } else {
                return Err(WebError::DomUnavailable);
            }

            Ok(())
        })
        .clone()
}
