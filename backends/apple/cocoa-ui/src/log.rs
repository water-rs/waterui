//! Messages to the unified log, the system log `Console.app` and `log stream`
//! read.
//!
//! The unified log is the one channel that reaches a developer from every
//! kind of launch: an application started by `open` has no reachable
//! standard output, and neither has one running in the simulator. Messages
//! are logged as public, so their text is readable outside the device too.

use std::fmt;

use oslog::{Level, OsLog};

/// A log handle for one subsystem and category.
///
/// The subsystem is conventionally a reverse-DNS identifier and the category
/// names the area within it; `log stream --predicate 'subsystem == "…"'`
/// filters on both.
pub struct Log {
    subsystem: String,
    category: String,
    inner: OsLog,
}

impl Log {
    /// A handle logging under `subsystem` and `category`.
    ///
    /// # Panics
    ///
    /// If either name contains a NUL character.
    #[must_use]
    pub fn new(subsystem: &str, category: &str) -> Self {
        assert_no_nul(subsystem, "subsystem");
        assert_no_nul(category, "category");
        Self {
            subsystem: subsystem.to_owned(),
            category: category.to_owned(),
            inner: OsLog::new(subsystem, category),
        }
    }

    /// Logs `message` at the debug level, which is kept only while a client
    /// is streaming it.
    ///
    /// # Panics
    ///
    /// If `message` contains a NUL character.
    pub fn debug(&self, message: &str) {
        self.log(Level::Debug, message);
    }

    /// Logs `message` at the info level, kept in memory until the buffer
    /// wraps.
    ///
    /// # Panics
    ///
    /// If `message` contains a NUL character.
    pub fn info(&self, message: &str) {
        self.log(Level::Info, message);
    }

    /// Logs `message` at the default level, the one Swift's `Logger.notice`
    /// uses: persisted to disk.
    ///
    /// # Panics
    ///
    /// If `message` contains a NUL character.
    pub fn notice(&self, message: &str) {
        self.log(Level::Default, message);
    }

    /// Logs `message` at the error level.
    ///
    /// # Panics
    ///
    /// If `message` contains a NUL character.
    pub fn error(&self, message: &str) {
        self.log(Level::Error, message);
    }

    /// Logs `message` at the fault level, for a bug in the process itself.
    ///
    /// # Panics
    ///
    /// If `message` contains a NUL character.
    pub fn fault(&self, message: &str) {
        self.log(Level::Fault, message);
    }

    fn log(&self, level: Level, message: &str) {
        assert_no_nul(message, "log message");
        self.inner.with_level(level, message);
    }
}

impl fmt::Debug for Log {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Log")
            .field("subsystem", &self.subsystem)
            .field("category", &self.category)
            .finish_non_exhaustive()
    }
}

/// The C API takes NUL-terminated strings; an interior NUL would cut the text
/// short, so it is refused rather than logged truncated or rewritten.
fn assert_no_nul(text: &str, what: &str) {
    assert!(
        !text.contains('\0'),
        "the {what} {text:?} contains a NUL character, which the unified log cannot carry"
    );
}
