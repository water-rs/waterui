use core::fmt;

/// Error type produced by the web backend.
#[derive(Debug, Clone)]
pub enum WebError {
    /// The DOM APIs are not accessible (e.g., when executed outside of a browser).
    DomUnavailable,
    /// The requested mounting node cannot be located.
    RootNotFound(String),
    /// The feature is currently unsupported on the active target.
    Unsupported,
    /// Wrapper around JavaScript exceptions.
    Js(String),
}

impl fmt::Display for WebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DomUnavailable => write!(f, "DOM is not available"),
            Self::RootNotFound(id) => write!(f, "Failed to find DOM element with id `{id}`"),
            Self::Unsupported => write!(f, "WaterUI web backend requires the wasm32 target"),
            Self::Js(msg) => write!(f, "JavaScript error: {msg}"),
        }
    }
}

impl std::error::Error for WebError {}

impl From<wasm_bindgen::JsValue> for WebError {
    fn from(value: wasm_bindgen::JsValue) -> Self {
        value
            .as_string()
            .map_or_else(|| Self::Js(format!("{value:?}")), Self::Js)
    }
}

impl From<WebError> for wasm_bindgen::JsValue {
    fn from(value: WebError) -> Self {
        match value {
            WebError::Js(msg) => Self::from(msg),
            WebError::DomUnavailable => Self::from("DOM is not available"),
            WebError::RootNotFound(id) => {
                Self::from(format!("Failed to find DOM element with id `{id}`"))
            }
            WebError::Unsupported => Self::from("WaterUI web backend requires the wasm32 target"),
        }
    }
}
