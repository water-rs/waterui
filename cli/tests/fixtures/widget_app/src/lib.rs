//! Fixture for `apple::widgets`: two `#[widget]` declarations exercising
//! the export + record emission — one with a `WidgetContext` parameter and
//! the default families, one context-free with an explicit family list.

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
        TimelineEntry::new(
            Date::unix_timestamp(1_700_000_060.0),
            text("second"),
        ),
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
