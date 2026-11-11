//! Messages to the unified log, the system log `Console.app` and `log stream`
//! read.
//!
//! The unified log is the one channel that reaches a developer from every
//! kind of launch: an application started by `open` has no reachable
//! standard output, and neither has one running in the simulator. Messages
//! are logged as public, so their text is readable outside the device too.

use std::borrow::Cow;
use std::fmt;

use oslog::{Level, OsLog};
use waterui_contract::contract_violation;

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
    /// A NUL character in either name breaks the contract: it is reported
    /// and the NULs are removed from the name.
    #[must_use]
    pub fn new(subsystem: &str, category: &str) -> Self {
        let subsystem = without_nul(subsystem, "subsystem");
        let category = without_nul(category, "category");
        Self {
            inner: OsLog::new(&subsystem, &category),
            subsystem: subsystem.into_owned(),
            category: category.into_owned(),
        }
    }

    /// Logs `message` at the debug level, which is kept only while a client
    /// is streaming it.
    ///
    /// A NUL character in `message` breaks the contract: it is reported and
    /// the NULs are removed from the logged text.
    pub fn debug(&self, message: &str) {
        self.log(Level::Debug, message);
    }

    /// Logs `message` at the info level, kept in memory until the buffer
    /// wraps.
    ///
    /// A NUL character in `message` breaks the contract: it is reported and
    /// the NULs are removed from the logged text.
    pub fn info(&self, message: &str) {
        self.log(Level::Info, message);
    }

    /// Logs `message` at the default level, the one Swift's `Logger.notice`
    /// uses: persisted to disk.
    ///
    /// A NUL character in `message` breaks the contract: it is reported and
    /// the NULs are removed from the logged text.
    pub fn notice(&self, message: &str) {
        self.log(Level::Default, message);
    }

    /// Logs `message` at the error level.
    ///
    /// A NUL character in `message` breaks the contract: it is reported and
    /// the NULs are removed from the logged text.
    pub fn error(&self, message: &str) {
        self.log(Level::Error, message);
    }

    /// Logs `message` at the fault level, for a bug in the process itself.
    ///
    /// A NUL character in `message` breaks the contract: it is reported and
    /// the NULs are removed from the logged text.
    pub fn fault(&self, message: &str) {
        self.log(Level::Fault, message);
    }

    fn log(&self, level: Level, message: &str) {
        self.inner
            .with_level(level, &without_nul(message, "log message"));
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
/// short, so a NUL is reported and removed rather than logged truncated.
fn without_nul<'a>(text: &'a str, what: &str) -> Cow<'a, str> {
    if text.contains('\0') {
        contract_violation!(
            "the {what} {text:?} contains a NUL character, which the unified log cannot carry; the NULs are removed"
        );
        return Cow::Owned(text.replace('\0', ""));
    }
    Cow::Borrowed(text)
}

#[cfg(test)]
mod tests {
    use super::without_nul;

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "contains a NUL character"))]
    fn nul_characters_are_removed_from_logged_text() {
        assert_eq!(without_nul("a\0b", "log message"), "ab");
    }

    #[test]
    fn text_without_nul_is_borrowed_unchanged() {
        assert_eq!(without_nul("ab", "log message"), "ab");
    }
}
