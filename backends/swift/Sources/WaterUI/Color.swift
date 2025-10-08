import CWaterUI
//
//  Color.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//
import SwiftUI

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
