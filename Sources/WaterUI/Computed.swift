//
//  Computed.swift
//
//
//  Created by Lexo Liu on 5/13/24.
//

import CWaterUI
import Foundation
import Combine



@MainActor
class ComputedStr:ObservableObject{
    private var inner: OpaquePointer
    private var rustWatcher=Set<OpaquePointer>()
    @Published var value:String = ""
    
    init(inner: OpaquePointer) {
        self.inner = inner
        self.value=self.compute()
        self.watch{new in
            self.value = new
        }
    }
    
    func compute()  -> String{
        waterui_read_computed_str(self.inner).toString()
    }
    
    
    func watch(_ f:@escaping (String)->()) {
        let g=waterui_watch_computed_str(self.inner, waterui_fn_waterui_str({value in
            f(value)
        }))
        rustWatcher.insert(g!)
    }

    deinit {
        waterui_drop_computed_str(self.inner)
        for watcher in self.rustWatcher{
            waterui_drop_watcher_guard(watcher)
        }
    }
}

@MainActor
class ComputedInt:ObservableObject{
    private var inner: OpaquePointer
    private var rustWatcher=Set<OpaquePointer>()
    @Published var value = 0
    init(inner: OpaquePointer) {
        self.inner = inner
        self.watch{new in
            self.value = new
        }
    }
    
    func compute() -> Int{
        Int(waterui_read_computed_int(self.inner))
    }
    
    func watch(_ f:@escaping (Int)->()) {
        let g = waterui_watch_computed_int(self.inner, waterui_fn_i32({value in
            f(Int(value))
        }))
        rustWatcher.insert(g!)
    }

    deinit {
        waterui_drop_computed_int(self.inner)
        for watcher in self.rustWatcher{
                waterui_drop_watcher_guard(watcher)

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

extension waterui_fn_i32 {
    init(_ f: @escaping (Int32) -> Void) {
        class Wrapper {
            var inner: (Int32) -> Void
            init(_ inner: @escaping (Int32) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value)

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}


extension waterui_fn_bool {
    init(_ f: @escaping (Bool) -> Void) {
        class Wrapper {
            var inner: (Bool) -> Void
            init(_ inner: @escaping (Bool) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value)

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}
