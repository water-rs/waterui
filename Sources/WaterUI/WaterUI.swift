import CWaterUI
import SwiftUI

extension waterui_str {
    init(_ string: String) {
        let len = string.utf8.count
        let raw = UnsafeMutableBufferPointer<UInt8>.allocate(capacity: len)
        _ = raw.initialize(fromContentsOf: string.utf8)
        self.init(head: raw.baseAddress, len: UInt(len))
    }

    func toString() -> String {
        let buf = UnsafeMutableBufferPointer(start: head, count: Int(len))
        let data = Data(buf)
        return String(decoding: data, as: UTF8.self)
    }
}

extension waterui_type_id: Equatable {
    public static func == (lhs: waterui_type_id, rhs: waterui_type_id) -> Bool {
        return lhs.inner == rhs.inner
    }
}

extension waterui_fnonce {
    init(_ f: @escaping () -> Void) {
        class Wrapper {
            var inner: () -> Void
            init(_ inner: @escaping () -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { value in
            let f = Unmanaged<Wrapper>.fromOpaque(value!).takeRetainedValue().inner
            f()
        })
    }
}



@MainActor
public func mainWidget() -> WaterUI.AnyView{
    let env=Environment(waterui_init())
    return WaterUI.AnyView(view: waterui_widget_main(), env: env)
}
