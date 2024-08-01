//
//  Computed.swift
//
//
//  Created by Lexo Liu on 5/13/24.
//

import CWaterUI
import Foundation

class ComputedStr {
    var inner: OpaquePointer
    var app: App
    init(inner: OpaquePointer, app: App) {
        self.inner = inner
        self.app = app
    }

    public func compute() async -> String {
        await app.task {
            waterui_read_computed_str(self.inner).toString()
        }
    }

    public func watch(_ f: @escaping (String) -> Void) {
        app.spawn {
            waterui_watch_computed_str(self.inner, waterui_fn_waterui_str(f))
        }
    }

    deinit {
        app.spawn {
            waterui_drop_computed_str(self.inner)
        }
    }
}

extension waterui_fn_waterui_str {
    init(_ f: @escaping (String) -> Void) {
        class Wrapper {
            var inner: (String) -> Void
            init(_ inner: @escaping (String) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value.toString())

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}

class ComputedInt {
    var inner: OpaquePointer
    init(_ inner: OpaquePointer) {
        self.inner = inner
    }

    func compute() -> Int32 {
        return waterui_read_computed_int(inner)
    }

    deinit {
        waterui_drop_computed_int(inner)
    }
}
