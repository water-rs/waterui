//! Widget declarations and a typed action exercised through native Swift.

use waterui::widgets::*;

#[waterui::widget(name = "Steps", description = "Today's step count")]
fn steps(context: &WidgetContext) -> Timeline {
    // The FFI probe asserts the context travels intact — echo the family
    // and proposed size into the produced content.
    let steps = format!(
        "{:?} {:.0}x{:.0}",
        context.family, context.display_size.width, context.display_size.height
    );
    Timeline::new([
        TimelineEntry::new(
            Date::unix_timestamp(1_700_000_000.0),
            vstack((
                text("Steps").font(Font::style(FontStyle::Title)),
                text(steps).color(Color::ACCENT),
                Shape::capsule().fill(Color::srgb(0.2, 0.6, 0.9)),
            )),
        ),
        TimelineEntry::new(Date::unix_timestamp(1_700_000_060.0), text("second")),
    ])
    .policy(ReloadPolicy::After {
        date: Date::unix_timestamp(1_700_000_120.0),
    })
}

#[waterui::widget(
    name = "Now",
    description = "The current time",
    kind = "now-kind",
    families = [systemSmall]
)]
fn now() -> impl WidgetContent {
    hstack((text("now"), Image::system("clock.fill")))
}

/// A serializable widget action — `#[widget_intent]` on the type declares
/// it; the extension decodes the payload and calls `perform`.
#[waterui::widget_intent(
    name = "Write record",
    description = "Writes a value to the record file"
)]
#[derive(Debug)]
pub struct WriteRecord {
    /// The owned record file the action writes.
    pub output_path: String,
    /// The value the action writes.
    pub value: String,
}

impl WidgetIntent for WriteRecord {
    fn perform(self) {
        std::fs::write(&self.output_path, &self.value).expect("the record file writes");
    }
}

#[waterui::widget(
    name = "Record",
    description = "Writes a record",
    kind = "record-kind",
    families = [systemSmall]
)]
fn record() -> impl WidgetContent {
    intent(
        &WriteRecord {
            // The probe runs in its own scratch directory.
            output_path: "record.txt".into(),
            value: "record \"value\"\\line\n\u{65e5}\u{672c}\u{8a9e}".into(),
        },
        text("Tap to write"),
    )
}
