import CWaterUI
import SwiftUI

enum WuiEventKind {
    case appear
    case disappear

    init(_ value: WuiEvent) {
        switch value {
        case WuiEvent_Appear:
            self = .appear
        case WuiEvent_Disappear:
            self = .disappear
        default:
            self = .appear
        }
    }
}

@MainActor
final class WuiOnEventHandle {
    private var inner: OpaquePointer?
    let kind: WuiEventKind

    init(_ inner: OpaquePointer) {
        self.inner = inner
        self.kind = WuiEventKind(waterui_on_event_kind(inner))
    }

    @MainActor deinit {
        if let pointer = inner {
            waterui_drop_on_event(pointer)
        }
    }

    func triggerIfNeeded(env: WuiEnvironment) {
        guard let pointer = inner else { return }
        waterui_on_event_trigger(pointer, env.inner)
        inner = nil
    }
}

struct WuiEventMetadataView: View, WuiComponent {
    static let id: String = decodeViewIdentifier(waterui_metadata_on_event_id())

    private let content: WuiAnyView
    private let event: WuiOnEventHandle
    private let env: WuiEnvironment
    private let kind: WuiEventKind

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        let metadata = waterui_metadata_force_as_on_event(anyview)
        self.content = WuiAnyView(anyview: metadata.content, env: env)
        let handle = WuiOnEventHandle(metadata.value)
        self.event = handle
        self.kind = handle.kind
        self.env = env
    }

    var body: some View {
        switch kind {
        case .appear:
            content.onAppear {
                event.triggerIfNeeded(env: env)
            }
        case .disappear:
            content.onDisappear {
                event.triggerIfNeeded(env: env)
            }
        }
    }
}
