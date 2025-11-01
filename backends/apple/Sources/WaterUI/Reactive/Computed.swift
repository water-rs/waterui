//
//  Computed.swift
//
//
//  Created by Lexo Liu on 5/13/24.
//

import CWaterUI
import Foundation
import Combine
import SwiftUI


@MainActor
@Observable
final class WuiComputed<T>: ObservableObject {
    private var inner: OpaquePointer
    private var watcher: WatcherGuard!

    private let readFn: (OpaquePointer?) -> T
    private let watchFn: (OpaquePointer?, @escaping (T, WuiWatcherMetadata) -> Void) -> WatcherGuard
    private let dropFn: (OpaquePointer?) -> Void

    var value: T

    init(
        inner: OpaquePointer,
        read: @escaping (OpaquePointer?) -> T,
        watch:
            @escaping (OpaquePointer?, @escaping (T, WuiWatcherMetadata) -> Void) -> WatcherGuard,
        drop: @escaping (OpaquePointer?) -> Void
    ) {
        self.inner = inner
        self.readFn = read
        self.watchFn = watch
        self.dropFn = drop
        self.value = read(inner)
        
        self.watcher = self.watch { [unowned self] value, metadata in
            useAnimation(metadata) {
                self.value = value
            }
        }
    }

    func compute() -> T {
        readFn(inner)
    }

    func watch(_ f: @escaping (T, WuiWatcherMetadata) -> Void) -> WatcherGuard {
        watchFn(inner, f)
    }


    @MainActor deinit {
        dropFn(inner)
    }
}





extension WuiComputed where T == WuiStr {
    convenience init(_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: { inner in WuiStr(waterui_read_binding_str(inner)) },
            watch: { inner, f in
                let g = waterui_watch_binding_str(inner, WuiWatcher_WuiStr(f))
                return WatcherGuard(g!)
            },
            drop: waterui_drop_binding_str
        )
    }
}

extension WuiComputed where T == Int32 {
    convenience init(_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: waterui_read_binding_i32,
            watch: { inner, f in
                let g = waterui_watch_binding_i32(inner, WuiWatcher_i32(f))
                return WatcherGuard(g!)
            },
            drop: waterui_drop_binding_i32
        )
    }
}

extension WuiComputed where T == Bool {
    convenience init(_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: waterui_read_binding_bool,
            watch: { inner, f in
                let g = waterui_watch_binding_bool(inner, WuiWatcher_bool(f))
                return WatcherGuard(g!)
            },
            drop: waterui_drop_binding_bool
        )
    }
}

extension WuiComputed where T == Double {
    convenience init(_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: waterui_read_binding_f64,
            watch: { inner, f in
                let g = waterui_watch_binding_f64(inner, WuiWatcher_f64(f))
                return WatcherGuard(g!)
            },
            drop: waterui_drop_binding_f64
        )
    }
}

extension WuiComputed where T == WuiResolvedFont {
    convenience init(_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: { inner in
                return waterui_read_computed_resolved_font(inner)
            },
            watch: { inner, f in
                let g = waterui_watch_computed_resolved_font(inner, WuiWatcher_WuiResolvedFont(f))
                return WatcherGuard(g!)
            },
            drop: waterui_drop_computed_resolved_font
        )
    }
}

extension WuiComputed where T == WuiResolvedColor {
    convenience init(_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: { inner in
                return waterui_read_computed_resolved_color(inner)
            },
            watch: { inner, f in
                let g = waterui_watch_computed_resolved_color(inner, WuiWatcher_WuiResolvedColor(f))
                return WatcherGuard(g!)
            },
            drop: waterui_drop_computed_resolved_color
        )
    }
}

extension WuiComputed where T == WuiStyledStr {
    convenience init (_ inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: { inner in
                return WuiStyledStr(waterui_read_computed_styled_str(inner))
            },
            watch: { inner, f in
                let g = waterui_watch_computed_styled_str(inner, WuiWatcher_WuiStyledStr(f))
                return WatcherGuard(g!)
            },
            drop: waterui_drop_computed_styled_str
        )
    }
}

