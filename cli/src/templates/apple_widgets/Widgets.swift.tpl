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

{% for widget in widgets %}
private enum __WUIDeclaration_{{ widget.index }}: WUIWidgetDeclaration {
    static let kind = {{ widget.kind }}
    static let displayName = {{ widget.display_name }}
    static let widgetDescription = {{ widget.description }}
    static let families: [WidgetFamily] = [{{ widget.families }}]
    static let snapshotProvider: WUISnapshotProvider = __wui_snapshot_{{ widget.index }}
}

{% endfor %}
@main
struct WaterUIWidgetExtension: WidgetBundle {
    var body: some Widget {
{% for widget in widgets %}
        WaterUIWidget<__WUIDeclaration_{{ widget.index }}>()
{% endfor %}
    }
}
