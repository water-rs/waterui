//
//  Closure.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI

extension waterui_bridge_closure {
    init(_ f: @escaping () -> Void) {
        class ClosureWrapper {
            var inner: () -> Void
            init(_ inner: @escaping () -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(ClosureWrapper(f)).toOpaque())

        self.init(data: data, call: { value in
            let f = Unmanaged<ClosureWrapper>.fromOpaque(value!).takeUnretainedValue().inner
            f()

        })
    }
}
