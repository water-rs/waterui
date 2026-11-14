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
#[non_exhaustive]
pub struct JsError {
    /// The exception class name (`"Error"`, `"TypeError"`, …).
    name: String,
    /// The exception message.
    message: String,
    /// The JavaScript stack trace (`error.stack`), when the engine gives one.
    stack: Option<String>,
}

impl JsError {
    /// The exception class name (`"Error"`, `"TypeError"`, …).
    #[must_use]
    pub const fn name(&self) -> &str {
        self.name.as_str()
    }

    /// The exception message.
    #[must_use]
    pub const fn message(&self) -> &str {
        self.message.as_str()
    }

    /// Sets the `message`.
    pub fn set_message(&mut self, message: String) {
        self.message = message;
    }

    /// The JavaScript stack trace (`error.stack`), when the engine gives one.
    #[must_use]
    pub fn stack(&self) -> Option<&str> {
        self.stack.as_deref()
    }

    /// Sets the `stack`.
    pub fn set_stack(&mut self, stack: Option<String>) {
        self.stack = stack;
    }
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
        // Neither engine prefixes its stack with a `name: message` line —
        // render it whole so the throw-site frame is never lost.
        if let Some(stack) = &self.stack {
            write!(f, "\n{stack}")?;
        }
        Ok(())
    }
}

impl std::error::Error for JsError {}
