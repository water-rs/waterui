//! Widget declarations and a typed action exercised through native Swift.

use waterui::widgets::prelude::*;

#[waterui::widget(name = "Steps", description = "Today's step count")]
fn steps(context: &WidgetContext) -> Timeline {
    // The FFI probe asserts both the context and shared store travel intact —
    // echo the family, proposed size, and stored step count into the produced
    // content, and exercise the full aligned widget vocabulary.
    let size = context.display_size();
    let count: u32 = context.shared_store().get("steps").unwrap_or(0);
    let steps = format!(
        "{:?} {:.0}x{:.0} steps={count}",
        context.family(),
        size.width,
        size.height
    );
    Timeline::new([
        TimelineEntry::new(
            Date::unix_timestamp(1_700_000_000.0),
            vstack((
                text("Steps").headline(),
                spacer(),
                text(Date::unix_timestamp(1_700_000_060.0)).relative(),
                image_system("figure.walk").a11y_label("Walking"),
                text(steps).foreground(Color::ACCENT),
                Shape::capsule()
                    .fill(Color::srgb(0.2, 0.6, 0.9))
                    .size(48.0, 8.0),
            ))
            .padding()
            .background(Color::ACCENT),
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
    hstack((text("now"), image_system("clock.fill")))
}

/// A serializable widget action — `#[widget_intent]` on the type declares
/// it; the extension decodes the payload and calls `perform` with the shared
/// widget store.
#[waterui::widget_intent(
    name = "Write record",
    description = "Writes a value to the shared store and record file"
)]
#[derive(Debug)]
pub struct WriteRecord {
    /// The owned record file the action writes.
    pub output_path: String,
    /// The value the action writes.
    pub value: String,
    /// Step count to persist into the shared widget store.
    pub next_steps: u32,
}

impl WidgetIntent for WriteRecord {
    fn perform(self, store: &WidgetStore) {
        store
            .set("steps", &self.next_steps)
            .expect("the shared widget store writes");
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
            next_steps: 4242,
        },
        text("Tap to write"),
    )
}
