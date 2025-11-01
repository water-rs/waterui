import CWaterUI
import SwiftUI


@MainActor
protocol WuiComponent: View {
    static var id: String { get }
    init(anyview: OpaquePointer, env: WuiEnvironment)
}

extension WuiComponent {
    static func decodeId(_ raw: CWaterUI.WuiStr) -> String {
        decodeViewIdentifier(raw)
    }
}

@inline(__always)
func decodeViewIdentifier(_ raw: CWaterUI.WuiStr) -> String {
    WuiStr(raw).toString()
}


@MainActor
final class WuiRootContext: ObservableObject {
    let env: WuiEnvironment
    let rootView: WuiAnyView

    init() {
        let environment = WuiEnvironment(waterui_init())
        self.env = environment
        guard let mainView = waterui_main() else {
            fatalError("waterui_main() returned nil")
        }
        self.rootView = WuiAnyView(anyview: mainView, env: environment)
    }

    deinit {
        // `WuiAnyView` owns the underlying pointer and will drop it on deinit.
    }
}

public struct App: View {
    @StateObject private var context = WuiRootContext()

    public init() {}

    public var body: some View {
        GeometryReader { proxy in
            context.rootView
                .frame(
                    width: proxy.size.width,
                    height: proxy.size.height,
                    alignment: .topLeading
                )
        }
    }
}
