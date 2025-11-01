//
//  Color.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//
import SwiftUI
import CWaterUI


@MainActor
class WuiColor {
    var inner: OpaquePointer
    init(_ inner: OpaquePointer) {
        self.inner = inner
    }

    func resolve(in env: WuiEnvironment) -> WuiComputed<WuiResolvedColor> {
        let computed = waterui_resolve_color(inner, env.inner)
        return WuiComputed(computed!)
    }

    @MainActor deinit {
        waterui_drop_color(inner)
    }
}

struct WuiColorView: WuiComponent, View{
    var color: WuiComputed<WuiResolvedColor>
    static let id: String = Self.decodeId(waterui_color_id())
    
    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.color = WuiColor(waterui_force_as_color(anyview)).resolve(in: env)
    }
    
    var body: some View {
        color.value.toSwiftUI()
    }
}


extension WuiResolvedColor {
    func toSwiftUI() -> SwiftUI.Color {
        Color(
            red: Double(self.red), green: Double(self.green), blue: Double(self.blue),
            opacity: Double(self.opacity))
    }

    init(_ color: SwiftUI.Color) {
        let resolved = color.resolveHDR(in: .init())
        self.init(
            red: Float(resolved.red), green: Float(resolved.green), blue: Float(resolved.blue),
            opacity: Float(resolved.opacity))
    }
}
