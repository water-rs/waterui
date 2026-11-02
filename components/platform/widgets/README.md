# waterui-widgets

The type-restricted authoring surface for home-screen widgets. A
`#[waterui::widget]` function produces an immutable serialized snapshot per
dated `TimelineEntry`; the generated Apple `WidgetKit` extension renders
that snapshot with native `SwiftUI`.

Only the closed schema can cross the extension boundary — text, image,
stack, shape, color, link and declared intents. A `WaterUI` `View` in a
widget position is a compile-time error: `WidgetKit` renders in another
process, where representable embedding cannot reach.

The `water` CLI discovers declarations through the crate's
`waterui_meta_widget_*` records and compiles the shipped `swift/` sources
into the packaged `.appex`.
