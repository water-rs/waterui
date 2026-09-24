//! A `tracing` layer that writes records to logcat.
//!
//! The event wire format is WaterUI's own contract — the event's message
//! first, then each field as `key=value`, space-separated — so a record with
//! fields never concatenates into one token the way
//! `lostreason=Unknown"…"` did when a third-party formatter wrote every
//! field back to back and the `message` field as a bare quoted value.

use alloc::string::String;
use core::fmt;

use tracing::Event;
use tracing::field::{Field, Visit};

/// Renders `event` as one logcat line: the target, then the event's message,
/// then each recorded field as `key=value`, all space-separated.
fn format_event(event: &Event<'_>) -> String {
    let mut fields = EventFields::default();
    event.record(&mut fields);
    let mut line = event.metadata().target().to_owned();
    line.push_str(": ");
    line.push_str(&fields.message);
    if !fields.named.is_empty() {
        if !fields.message.is_empty() {
            line.push(' ');
        }
        line.push_str(&fields.named);
    }
    line
}

/// Collects an event's fields into a message and a `key=value` list.
///
/// The first field named `message` — the literal every `tracing` macro
/// records for its format string — is the event's message and is written
/// bare. Every later field, a second `message` included, keeps its
/// `key=value` form.
#[derive(Default)]
struct EventFields {
    message: String,
    named: String,
    message_seen: bool,
}

impl Visit for EventFields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        use core::fmt::Write;
        if field.name() == "message" && !self.message_seen {
            let _ = write!(self.message, "{value:?}");
            self.message_seen = true;
        } else {
            if !self.named.is_empty() {
                self.named.push(' ');
            }
            let _ = write!(self.named, "{}={value:?}", field.name());
        }
    }
}

/// The largest payload one `__android_log_write` call carries; logd truncates
/// anything longer, so a record is emitted in chunks of this many bytes.
#[cfg(target_os = "android")]
const LOGCAT_WRITE_LIMIT: usize = 4000;

/// A `tracing_subscriber` layer that writes each event to logcat through
/// `__android_log_write`.
#[cfg(target_os = "android")]
pub struct AndroidLogLayer {
    tag: std::ffi::CString,
}

#[cfg(target_os = "android")]
impl AndroidLogLayer {
    /// A layer tagging every record with `tag`, truncated to logd's 23-byte
    /// cap rather than rejected.
    pub fn new(tag: &str) -> Self {
        let end = tag.floor_char_boundary(tag.len().min(23));
        Self {
            tag: std::ffi::CString::new(&tag[..end])
                .expect("an Android log tag must not contain NUL"),
        }
    }
}

#[cfg(target_os = "android")]
const fn log_priority(level: tracing::Level) -> ndk_sys::android_LogPriority {
    use ndk_sys::android_LogPriority as Priority;
    match level {
        tracing::Level::ERROR => Priority::ANDROID_LOG_ERROR,
        tracing::Level::WARN => Priority::ANDROID_LOG_WARN,
        tracing::Level::INFO => Priority::ANDROID_LOG_INFO,
        tracing::Level::DEBUG => Priority::ANDROID_LOG_DEBUG,
        tracing::Level::TRACE => Priority::ANDROID_LOG_VERBOSE,
    }
}

#[cfg(target_os = "android")]
impl<S> tracing_subscriber::Layer<S> for AndroidLogLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let line = format_event(event);
        let priority = log_priority(*event.metadata().level());
        let mut rest = line.as_str();
        while !rest.is_empty() {
            let end = rest.floor_char_boundary(rest.len().min(LOGCAT_WRITE_LIMIT));
            write_logcat(priority, &self.tag, &rest[..end]);
            rest = &rest[end..];
        }
    }
}

/// Emits one chunk of a record. `__android_log_write` stops at the first NUL,
/// so a payload that somehow carries one goes out with it replaced rather
/// than truncated mid-line.
#[cfg(target_os = "android")]
fn write_logcat(priority: ndk_sys::android_LogPriority, tag: &std::ffi::CStr, text: &str) {
    let text = std::ffi::CString::new(text).unwrap_or_else(|_| {
        std::ffi::CString::new(text.replace('\0', " ")).expect("NUL-stripped text is NUL-free")
    });
    // SAFETY: `tag` and `text` are valid NUL-terminated C strings that outlive
    // the call, and `__android_log_write` does not retain them.
    unsafe {
        ndk_sys::__android_log_write(priority.0.cast_signed(), tag.as_ptr(), text.as_ptr());
    }
}

#[cfg(test)]
mod tests {
    use super::format_event;
    use alloc::string::String;
    use alloc::sync::Arc;
    use alloc::vec::Vec;
    use std::sync::Mutex;
    use tracing::Event;
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

    /// Captures the lines a record produces, the way the logcat writer would
    /// emit them.
    struct Capture {
        lines: Arc<Mutex<Vec<String>>>,
    }

    impl<S: tracing::Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            self.lines
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(format_event(event));
        }
    }

    /// A record carrying two fields — an enum-shaped `reason` and a second
    /// field literally named `message` — must read as the event's message
    /// followed by separated `key=value` fields, never the concatenated
    /// `lostreason=Unknown"…"` the old formatter produced.
    #[test]
    fn an_event_with_fields_separates_message_and_fields() {
        #[derive(Debug)]
        enum Reason {
            Unknown,
        }

        let lines = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(Capture {
            lines: Arc::clone(&lines),
        });
        let reason = Reason::Unknown;
        let message = "Unexpected error variant (driver implementation is at fault)";
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(?reason, message, "WaterUI GPU device was lost");
        });

        let lines = lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(
            lines.as_slice(),
            [alloc::format!(
                "{}: WaterUI GPU device was lost reason=Unknown \
                 message=\"Unexpected error variant (driver implementation is at fault)\"",
                module_path!()
            )]
        );
    }
}
