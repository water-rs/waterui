//! `tracing` records, bridged to the `logs` channel.
//!
//! The inspector shows the same records `water run --logs debug` prints, so a
//! developer does not have to choose between a terminal and a UI.

use std::fmt;
use std::sync::Arc;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use waterui_inspector_protocol::{Channel, InspectorEvent, LogLevel, LogRecord};

use super::hub::EventHub;

/// A `tracing` layer that forwards records to a connected inspector.
#[derive(Debug)]
pub struct InspectorLayer {
    hub: Arc<EventHub>,
}

impl InspectorLayer {
    /// Creates a layer publishing to `hub`.
    pub(super) const fn new(hub: Arc<EventHub>) -> Self {
        Self { hub }
    }
}

impl<S: Subscriber> Layer<S> for InspectorLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if !self.hub.wants(Channel::Logs) {
            return;
        }

        let metadata = event.metadata();
        let mut visitor = RecordVisitor::default();
        event.record(&mut visitor);

        self.hub.publish(InspectorEvent::Log(LogRecord {
            level: level_of(*metadata.level()),
            target: metadata.target().to_string(),
            message: visitor.message.unwrap_or_default(),
            file: metadata.file().map(ToString::to_string),
            line: metadata.line(),
            fields: visitor.fields,
        }));
    }
}

/// Splits a record's `message` field from its structured fields.
#[derive(Default)]
struct RecordVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl RecordVisitor {
    fn record(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push((field.name().to_string(), value));
        }
    }
}

impl Visit for RecordVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record(field, value.to_string());
    }
}

const fn level_of(level: Level) -> LogLevel {
    match level {
        Level::TRACE => LogLevel::Trace,
        Level::DEBUG => LogLevel::Debug,
        Level::INFO => LogLevel::Info,
        Level::WARN => LogLevel::Warn,
        Level::ERROR => LogLevel::Error,
    }
}
