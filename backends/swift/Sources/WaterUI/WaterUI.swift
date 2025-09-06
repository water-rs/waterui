import CWaterUI
import SwiftUI

class WuiStr {
    var inner: OpaquePointer
    var isLeak: Bool = false
    init(_ inner: OpaquePointer) {
        self.inner = inner
    }

    init(string: String) {
        self.inner = string.utf8.withContiguousStorageIfAvailable { head in
            return waterui_str_from_bytes(head.baseAddress, UInt32(string.count))
        }!
    }

    func toString() -> String {
        // By ptr and len, copying the data
        let start = waterui_str_as_ptr(self.inner)
        let len = Int(waterui_str_len(self.inner))
        let buf = UnsafeBufferPointer<UInt8>(start: start, count: len)  // do not copy

        return String(decoding: buf, as: UTF8.self)  // copy here

    }

    func leak() {
        self.isLeak = true
    }

    deinit {
        if !isLeak {
            waterui_str_drop(self.inner)
        }
    }
}

extension WuiTypeId: @retroactive Equatable {
    public static func == (lhs: WuiTypeId, rhs: WuiTypeId) -> Bool {
        return lhs.inner.0 == rhs.inner.0 && lhs.inner.1 == rhs.inner.1
    }
}

@MainActor
protocol Component: View {
    static var id: WuiTypeId { get }
    init(anyview: OpaquePointer, env: WaterUI.Environment)
}

@MainActor
public struct App:View{
    var env: WaterUI.Environment
    public init() {
        self.env = WaterUI.Environment(waterui_init())
    }

    public var body: some View {
        WaterUI.AnyView(anyview: waterui_main(), env: env)
    }
}
