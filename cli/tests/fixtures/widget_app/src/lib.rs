//! Fixture for `apple::widgets`: two `#[widget]` declarations exercising
//! the export + record emission — one with a `WidgetContext` parameter and
//! the default families, one context-free with an explicit family list.

use waterui::widgets::*;

#[waterui::widget(name = "Steps", description = "Today's step count")]
fn steps(context: &WidgetContext) -> impl WidgetContent {
    let _ = context;
    vstack((
        text("Steps").font(Font::style(FontStyle::Title)),
        text("6,241").color(Color::ACCENT),
        Shape::capsule().fill(Color::srgb(0.2, 0.6, 0.9)),
    ))
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
