use core::fmt;

use std::io;

/// Errors that can occur while building or running a [`TuiApp`](crate::TuiApp).
#[derive(Debug, Clone)]
pub enum TuiError {
    /// Returned when the terminal handle was not configured on the builder.
    TerminalUnavailable,
    /// Returned when the renderer is missing and no placeholder feature was enabled.
    RendererUnavailable,
    /// Low level terminal I/O failure.
    Io(String),
    /// Rendering pipeline error.
    Render(String),
}

impl fmt::Display for TuiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TerminalUnavailable => {
                write!(f, "terminal backend has not been configured")
            }
            Self::RendererUnavailable => {
                write!(f, "renderer backend has not been configured")
            }
            Self::Io(message) => write!(f, "terminal I/O error: {message}"),
            Self::Render(message) => write!(f, "rendering error: {message}"),
        }
    }
}

impl std::error::Error for TuiError {}

impl From<io::Error> for TuiError {
    fn from(value: io::Error) -> Self {
        Self::Io(value.to_string())
    }
}
