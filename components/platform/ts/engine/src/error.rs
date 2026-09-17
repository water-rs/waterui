//! What a JavaScript exception — or a failure to cross the seam — becomes.

use std::fmt;

/// A JavaScript exception, or an error raised while converting a value.
///
/// A thrown `Error` arrives with its class [`name`](Self::name),
/// [`message`](Self::message) and, when the engine provides one, its
/// JavaScript [`stack`](Self::stack). Conversion failures carry the
/// synthetic name `"TypeError"`.
///
/// The type is `Send + Sync` and implements `std::error::Error`, so
/// `waterui_core::Error::from` accepts it and renders the stack as part of
/// the message.
#[derive(Debug, Clone)]
pub struct JsError {
    /// The exception class name (`"Error"`, `"TypeError"`, …).
    pub name: String,
    /// The exception message.
    pub message: String,
    /// The JavaScript stack trace (`error.stack`), when the engine gives one.
    pub stack: Option<String>,
}

impl JsError {
    /// An error with no stack attached.
    pub fn new(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            message: message.into(),
            stack: None,
        }
    }

    /// Attaches a JavaScript stack trace.
    #[must_use]
    pub fn with_stack(mut self, stack: impl Into<String>) -> Self {
        self.stack = Some(stack.into());
        self
    }

    /// A value could not cross the seam — a `symbol`, a `bigint` beyond
    /// `u64`, or a `JsValue` an engine cannot materialize.
    pub fn conversion(message: impl Into<String>) -> Self {
        Self::new("TypeError", message)
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.message)?;
        // A `stack` string's first line already reads `name: message`; append
        // only the frames so the rendered error is not duplicated.
        if let Some(stack) = &self.stack
            && let Some((_, frames)) = stack.split_once('\n')
            && !frames.is_empty()
        {
            write!(f, "\n{frames}")?;
        }
        Ok(())
    }
}

impl std::error::Error for JsError {}
