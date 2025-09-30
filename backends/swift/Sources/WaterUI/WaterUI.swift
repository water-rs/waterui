import CWaterUI
import SwiftUI


@MainActor
protocol WuiComponent: View {
    static var id: WuiTypeId { get }
    init(anyview: OpaquePointer, env: WuiEnvironment)
}


@MainActor
public struct App: View {
    var env: WuiEnvironment
    public init() {
        self.env = WuiEnvironment(waterui_init())
    }

    public var body: some View {
        VStack {
            WuiAnyView(anyview: waterui_main(), env: env)
        }
    }
}
