# waterui-widgets

The type-restricted authoring surface, shared state store, and timeline reload
bridge for home-screen widgets. A `#[waterui::widget]` function produces an
immutable serialized snapshot per dated `TimelineEntry`, reads shared state
through `WidgetContext::shared_store()`, and interactive `#[waterui::widget_intent]`
actions mutate the same [`WidgetStore`] before the next timeline renders.

Only the closed schema can cross the extension boundary — text, image,
stack, shape, color, link and declared intents. A `WaterUI` `View` in a
widget position is a compile-time error: `WidgetKit` renders in another
process, where representable embedding cannot reach.

The `water` CLI discovers declarations through the crate's
`waterui_meta_widget_*` records, provisions the shared App Group container on
both the host app and `.appex` extension on Apple (and stages the Kotlin widget
host on Android), compiles the shipped `swift/` sources into the packaged
`.appex`, and lets the app refresh visible widgets with `waterui::widgets::reload`.
