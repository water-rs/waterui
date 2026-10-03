//! `tracing` → `__android_log_write`, the platform's log channel — the
//! counterpart of the Apple backend's `os_log` writer. One `write()` call
//! per record, on record end: `__android_log_write` takes a finished line,
//! not a stream.

use std::ffi::CString;
use std::io::{Result, Write};
use std::os::raw::c_char;

use tracing_subscriber::fmt::MakeWriter;

/// The log tag every record carries — the app's own channel in logcat.
const TAG: &std::ffi::CStr = c"WaterUI";

#[link(name = "log")]
unsafe extern "C" {
    /// Writes `text` to logcat at `priority` under `tag`.
    fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char);
}

/// The `tracing` writer factory `FmtSubscriber` asks for a writer per event.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AndroidLog;

impl<'a> MakeWriter<'a> for AndroidLog {
    type Writer = RecordWriter;

    fn make_writer(&self) -> Self::Writer {
        RecordWriter::new()
    }
}

/// Buffers one record's bytes and emits them on drop: tracing may write a
/// record in several calls, but logcat must see exactly one line per record.
pub(crate) struct RecordWriter {
    buffer: Vec<u8>,
}

impl RecordWriter {
    const fn new() -> Self {
        Self { buffer: Vec::new() }
    }
}

impl Write for RecordWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Drop for RecordWriter {
    fn drop(&mut self) {
        // `__android_log_write` wants a C string; an interior NUL would end
        // the line early, so the record is written only if it is clean —
        // and a malformed one is never worth a second syscall to split.
        if let Ok(text) = CString::new(self.buffer.as_slice()) {
            // SAFETY: `TAG` and `text` are NUL-terminated C strings that
            // outlive the call; INFO is a valid logcat priority.
            unsafe {
                __android_log_write(4, TAG.as_ptr(), text.as_ptr());
            }
        }
    }
}
