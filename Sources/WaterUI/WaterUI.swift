import CWaterUI
import Semaphore
import SwiftUI

extension waterui_str {
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
