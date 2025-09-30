//
//  Binding.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Combine
import Foundation
import SwiftUI

@MainActor
final class BindingStr: ObservableObject {
    private var inner: OpaquePointer
    private var watcher: WatcherGuard!

    var value: Binding<String> {
        Binding(
            get: {
                self.compute()
            },
            set: { new in
                self.set(new)
            })
    }

    init(inner: OpaquePointer) {
        self.inner = inner

        watcher = self.watch { new, animation in
            useAnimation(animation: animation, publisher: self.objectWillChange)
        }
    }

    func compute() -> String {
        let wuiStr = WuiStr(waterui_read_binding_str(self.inner))
        return wuiStr.toString()
    }

    func watch(_ f: @escaping (String, Animation?) -> Void) -> WatcherGuard {
        let g = waterui_watch_binding_str(
            self.inner,
            WuiWatcher_WuiStr({ value, animation in
                f(value, animation)
            }))
        return WatcherGuard(g!)
    }

    func set(_ value: String) {
        let wuiStr = WuiStr(string:value)
        waterui_set_binding_str(self.inner, wuiStr.toCWuiStr())
    }

    deinit {

        weak var this = self
        Task { @MainActor in
            if let this = this {
                waterui_drop_binding_str(this.inner)
            }
        }

    }
}

@MainActor
class BindingInt: ObservableObject {
    private var inner: OpaquePointer
    private var watcher: WatcherGuard!

    var value: Binding<Int32> {
        Binding(
            get: {
                self.compute()
            },
            set: { new in
                self.set(new)
            })
    }

    init(inner: OpaquePointer) {
        self.inner = inner
        self.watcher = self.watch { new, animation in
            useAnimation(animation: animation, publisher: self.objectWillChange)
        }
    }

    func compute() -> Int32 {
        waterui_read_binding_int(self.inner)
    }

    func watch(_ f: @escaping (Int, Animation?) -> Void) -> WatcherGuard {
        let g = waterui_watch_binding_int(
            self.inner,
            WuiWatcher_i32({ value, animation in
                f(Int(value), animation)
            }))

        return WatcherGuard(g!)
    }

    func set(_ value: Int32) {
        waterui_set_binding_int(self.inner, value)

    }

    deinit {

        weak var this = self
        Task { @MainActor in
            if let this = this {
                waterui_drop_binding_int(this.inner)
            }
        }

    }
}

@MainActor
class BindingBool: ObservableObject {
    private var inner: OpaquePointer
    private var watcher: WatcherGuard!

    var value: Binding<Bool> {
        Binding(
            get: {
                self.compute()
            },
            set: { new in
                self.set(new)
            })
    }
    init(inner: OpaquePointer) {
        self.inner = inner

        self.watcher = self.watch { new, animation in
            useAnimation(animation: animation, publisher: self.objectWillChange)
        }
    }

    func compute() -> Bool {
        waterui_read_binding_bool(self.inner)
    }

    func watch(_ f: @escaping (Bool, Animation?) -> Void) -> WatcherGuard {
        let g = waterui_watch_binding_bool(
            self.inner,
            WuiWatcher_bool({ value, animation in
                f(value, animation)
            }))
        return WatcherGuard(g!)
    }

    func set(_ value: Bool) {
        waterui_set_binding_bool(self.inner, value)

    }

    deinit {

        weak var this = self
        Task { @MainActor in
            if let this = this {
                waterui_drop_binding_bool(this.inner)
            }
        }

    }
}

@MainActor
class BindingDouble: ObservableObject {
    private var inner: OpaquePointer
    private var watcher: WatcherGuard!

    var value: Binding<Double> {
        Binding(
            get: {
                self.compute()
            },
            set: { new in
                self.set(new)
            })
    }
    init(inner: OpaquePointer) {
        self.inner = inner

        self.watcher = self.watch { new, animation in
            useAnimation(animation: animation, publisher: self.objectWillChange)
        }
    }

    func compute() -> Double {
        waterui_read_binding_double(self.inner)
    }

    func watch(_ f: @escaping (Double, Animation?) -> Void) -> WatcherGuard {
        let g = waterui_watch_binding_double(
            self.inner,
            WuiWatcher_f64({ value, animation in
                f(value, animation)
            }))
        return WatcherGuard(g!)
    }

    func set(_ value: Double) {
        waterui_set_binding_double(self.inner, value)

    }

    deinit {

        weak var this = self
        Task { @MainActor in
            if let this = this {
                waterui_drop_binding_double(this.inner)
            }
        }

    }
}
