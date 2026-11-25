import AppIntents
import Foundation
import SwiftUI

// The extension's declared intents — one concrete `AppIntent` per
// `waterui_meta_intent_*` record, plus the `WUIDeclaredIntentButton` view
// `WUINodeView`'s intent case dispatches through.

{% for intent in intents %}
@_extern(c, "{{ intent.symbol }}")
private func __wui_intent_{{ intent.index }}(
    _ payloadPtr: UnsafePointer<UInt8>,
    _ payloadLen: Int,
    _ storePtr: UnsafePointer<UInt8>,
    _ storeLen: Int
)

struct {{ intent.swift_type }}: AppIntent {
    static let title: LocalizedStringResource = {{ intent.display_name }}
    static let description: IntentDescription? = IntentDescription({{ intent.description }})
    static var isDiscoverable: Bool { false }

    @Parameter(title: "Payload")
    var payload: String

    var storePath: String? = nil

    init() {}

    init(payload: String, storePath: String? = nil) {
        self.payload = payload
        self.storePath = storePath
    }

    func perform() async throws -> some IntentResult {
        performWidgetIntent(
            __wui_intent_{{ intent.index }},
            payload: payload,
            storePath: storePath ?? WUIWidgetStore.sharedContainerPath()
        )
        return .result()
    }
}

{% endfor %}
/// The view `WUINodeView`'s intent case renders — dispatches the declared
/// kind to its concrete `AppIntent`. An undeclared kind is a schema
/// violation, not a fallback case.
struct WUIDeclaredIntentButton: View {
    let intent: WUIIntent

    init(_ intent: WUIIntent) {
        self.intent = intent
    }

    @ViewBuilder
    var body: some View {
        switch intent.kind {
{% for intent in intents %}
        case {{ intent.kind }}:
            Button(intent: {{ intent.swift_type }}(payload: intent.payload)) {
                WUINodeView(intent.label)
            }
            .buttonStyle(.plain)
{% endfor %}
        default:
            fatalError("undeclared widget intent kind: \(intent.kind)")
        }
    }
}
