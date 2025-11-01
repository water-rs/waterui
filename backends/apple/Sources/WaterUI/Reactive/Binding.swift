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
final class WuiBinding<T> {
    private var inner: OpaquePointer
    private var watcher: WatcherGuard!

    private let readFn: (OpaquePointer?) -> T
    private let watchFn: (OpaquePointer?, @escaping (T, WuiWatcherMetadata) -> Void) -> WatcherGuard
    private let setFn: (OpaquePointer?, T) -> Void
    private let dropFn: (OpaquePointer?) -> Void
    private var cancelTracking: (() -> Void)?
    private var isSyncingFromRust = false

    var value: T {
        didSet {
            guard !isSyncingFromRust else { return }
            setFn(inner, value)
        }
    }
    
    init(
        inner: OpaquePointer,
        read: @escaping (OpaquePointer?) -> T,
        watch:
            @escaping (OpaquePointer?, @escaping (T, WuiWatcherMetadata) -> Void) -> WatcherGuard,
        set: @escaping (OpaquePointer?, T) -> Void,
        drop: @escaping (OpaquePointer?) -> Void
    ) {
        self.inner = inner
        self.readFn = read
        self.watchFn = watch
        self.setFn = set
        self.dropFn = drop
        self.isSyncingFromRust = true
        self.value = read(inner)
        self.isSyncingFromRust = false

        self.watcher = self.watch { [unowned self] value, metadata in
            self.withRustSync {
                useAnimation(metadata) {
                    self.value = value
                }
            }
        }

    }
    

    func compute() -> T {
        readFn(inner)
    }

    func watch(_ f: @escaping (T, WuiWatcherMetadata) -> Void) -> WatcherGuard {
        watchFn(inner, f)
    }

    func set(_ value: T) {
        self.value = value
    }

    @MainActor deinit {
        dropFn(inner)
    }
}

@MainActor
private extension WuiBinding {
    func withRustSync(_ update: () -> Void) {
        let wasSyncing = isSyncingFromRust
        isSyncingFromRust = true
        update()
        isSyncingFromRust = wasSyncing
    }
}

extension WuiBinding where T == WuiStr {
    convenience init(_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: { inner in WuiStr(waterui_read_binding_str(inner)) },
            watch: { inner, f in
                let g = waterui_watch_binding_str(inner, WuiWatcher_WuiStr(f))
                return WatcherGuard(g!)
            },
            set: { inner, value in
                waterui_set_binding_str(inner, value.intoInner())
            },
            drop: waterui_drop_binding_str
        )
    }
}

extension WuiBinding where T == Int32 {
    convenience init(_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: waterui_read_binding_i32,
            watch: { inner, f in
                let g = waterui_watch_binding_i32(inner, WuiWatcher_i32(f))
                return WatcherGuard(g!)
            },
            set: waterui_set_binding_i32,
            drop: waterui_drop_binding_i32
        )
    }
}

extension WuiBinding where T == Bool {
    convenience init(_ inner: OpaquePointer) {
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
    convenience init(_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: waterui_read_binding_f64,
            watch: { inner, f in
                let g = waterui_watch_binding_f64(inner, WuiWatcher_f64(f))
                return WatcherGuard(g!)
            },
            set: waterui_set_binding_f64,
            drop: waterui_drop_binding_f64
        )
    }
}
