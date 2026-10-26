import AppIntents
import SwiftUI
import WidgetKit

// The entry point `water` generates for the WidgetKit extension: one
// `WaterUIWidget` per `#[widget]` declaration record, bound to the
// `waterui_widget_<crate>_<fn>` export the record pairs with.

{% for widget in widgets %}
@_extern(c, "{{ widget.symbol }}")
private func __wui_snapshot_{{ widget.index }}(_ ptr: UnsafePointer<UInt8>, _ len: Int, _ out: UnsafeMutableRawPointer)
{% endfor %}

@main
struct WaterUIWidgetExtension: WidgetBundle {
    var body: some Widget {
{% for widget in widgets %}
        WaterUIWidget(
            kind: {{ widget.kind }},
            displayName: {{ widget.display_name }},
            description: {{ widget.description }},
            families: [{{ widget.families }}],
            snapshot: __wui_snapshot_{{ widget.index }}
        )
{% endfor %}
    }
}
