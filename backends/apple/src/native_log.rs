//! `tracing` records into the platform unified log through cocoa-ui's
//! `Log`, the single `os_log` provider in this process — the `oslog` and
//! `tracing-oslog` crates each export a C `wrapped_os_log_*` shim, so only
//! one can ever link.
//!
//! A `MakeWriter` factory is the natural fit: `fmt` already renders the
//! event — fields, span context, the whole formatted line — and hands the
//! record to `make_writer_for` together with its `Metadata`, which selects
//! the unified-log level. Each finished record owns its buffer and a fresh
//! `Log` handle, so nothing borrowed crosses threads and `Log`'s Send +
//! Sync contract is never stretched.

use std::io;

use cocoa_ui::log::Log;
use tracing::{Level, Metadata};
use tracing_subscriber::fmt::MakeWriter;

/// Writer factory emitting formatted `tracing` records under the
/// `dev.waterui` subsystem and the `default` category — public records,
/// the same destination `Console.app` and `log stream` filter on.
#[derive(Clone, Copy, Debug)]
pub struct NativeLog;

impl<'a> MakeWriter<'a> for NativeLog {
    type Writer = RecordWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RecordWriter::new(Level::INFO)
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        RecordWriter::new(*meta.level())
    }
}

/// Buffers one formatted record and emits it once at the mapped level.
/// `fmt` writes the whole event through `Write`, so emission happens in
/// `Drop` — exactly once per record regardless of chunking.
pub struct RecordWriter {
    log: Log,
    level: Level,
    buffer: Vec<u8>,
}

impl RecordWriter {
    fn new(level: Level) -> Self {
        Self {
            log: Log::new("dev.waterui", "default"),
            level,
            buffer: Vec::new(),
        }
    }
}

impl io::Write for RecordWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RecordWriter {
    fn drop(&mut self) {
        let message = String::from_utf8_lossy(&self.buffer);
        let message = message.trim_end();
        if message.is_empty() {
            return;
        }
        // `Log` panics on NUL; formatted output never produces one, but a
        // recorded field value can.
        let message = message.replace('\0', "\\0");
        match self.level {
            Level::ERROR => self.log.error(&message),
            Level::WARN => self.log.notice(&message),
            Level::INFO => self.log.info(&message),
            Level::DEBUG | Level::TRACE => self.log.debug(&message),
        }
    }
}
