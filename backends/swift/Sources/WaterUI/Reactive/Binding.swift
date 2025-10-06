//
//  WuiBinding.swift
//
//
//  Created by Gemini on 10/6/25.
//

import CWaterUI
import Combine
import Foundation
import SwiftUI

@MainActor
@Observable
final class WuiBinding<T>: ObservableObject {
    private var inner: OpaquePointer
    private var watcher: WatcherGuard!

    private let readFn: (OpaquePointer?) -> T
    private let watchFn: (OpaquePointer?, @escaping (T, Animation?) -> Void) -> WatcherGuard
    private let setFn: (OpaquePointer?, T) -> Void
    private let dropFn: (OpaquePointer?) -> Void

    var value: T
    
    init(
        inner: OpaquePointer,
        read: @escaping (OpaquePointer?) -> T,
        watch: @escaping (OpaquePointer?, @escaping (T, Animation?) -> Void) -> WatcherGuard,
        set: @escaping (OpaquePointer?, T) -> Void,
        drop: @escaping (OpaquePointer?) -> Void
    ) {
        self.inner = inner
        self.readFn = read
        self.watchFn = watch
        self.setFn = set
        self.dropFn = drop
        self.value = read(inner)

        self.watcher = self.watch { [weak self] _, animation in
            guard let self = self else { return }
            useAnimation(animation: animation, publisher: self.objectWillChange)
        }
    }
    

    func compute() -> T {
        readFn(inner)
    }

    func watch(_ f: @escaping (T, Animation?) -> Void) -> WatcherGuard {
        watchFn(inner, f)
    }

    func set(_ value: T) {
        setFn(inner, value)
    }

    @MainActor deinit {
        dropFn(inner)
    }
}


typealias WuiBindingStr = WuiBinding<String>
typealias WuiBindingInt = WuiBinding<Int32>
typealias WuiBindingBool = WuiBinding<Bool>
typealias WuiBindingDouble = WuiBinding<Double>

extension WuiBinding where T == String {
    convenience init(inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: { inner in WuiStr(waterui_read_binding_str(inner)).toString() },
            watch: { inner, f in
                let g = waterui_watch_binding_str(inner, WuiWatcher_WuiStr(f))
                return WatcherGuard(g!)
            },
            set: { inner, value in
                let wuiStr = WuiStr(string: value)
                waterui_set_binding_str(inner, wuiStr.toCWuiStr())
            },
            drop: waterui_drop_binding_str
        )
    }
}

extension WuiBinding where T == Int32 {
    convenience init(inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: waterui_read_binding_int,
            watch: { inner, f in
                let g = waterui_watch_binding_int(inner, WuiWatcher_i32(f))
                return WatcherGuard(g!)
            },
            set: waterui_set_binding_int,
            drop: waterui_drop_binding_int
        )
    }
}

extension WuiBinding where T == Bool {
    convenience init(inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: waterui_read_binding_bool,
            watch: { inner, f in
                let g = waterui_watch_binding_bool(inner, WuiWatcher_bool(f))
                return WatcherGuard(g!)
            },
            set: waterui_set_binding_bool,
            drop: waterui_drop_binding_bool
        )
    }
}

extension WuiBinding where T == Double {
    convenience init(inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: waterui_read_binding_double,
            watch: { inner, f in
                let g = waterui_watch_binding_double(inner, WuiWatcher_f64(f))
                return WatcherGuard(g!)
            },
            set: waterui_set_binding_double,
            drop: waterui_drop_binding_double
        )
    }
}
