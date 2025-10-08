//
//  Watcher.swift
//  
//
//  Created by Gemini on 10/6/25.
//

import CWaterUI
import SwiftUI

/// The protocol that all C-level watcher structs should conform to via an extension.
@MainActor
protocol Watcher {
    associatedtype Output
    
    init(_ f: @escaping (Output, WuiWatcherMetadata) -> Void)
}

@MainActor
class WatcherGuard {
    var inner: OpaquePointer
    init(_ inner: OpaquePointer) {
        self.inner = inner
    }
    @MainActor deinit {
        waterui_drop_box_watcher_guard(inner)
    }
}

@MainActor
class WuiWatcherMetadata {
    var inner: OpaquePointer
    init(_ inner: OpaquePointer) {
        self.inner = inner
    }

    func getAnimation() -> Animation? {
        let animation = waterui_get_animation(inner)
        return .init(animation)
    }

    @MainActor deinit {
        waterui_drop_watcher_metadata(inner)
    }
}

// MARK: - Watcher Implementations
//
// Pattern for implementing Watcher protocol for C-level watcher types:
//
// For value types (Int32, Bool, Double, etc.):
//   1. Create a Wrapper class to hold the Swift closure
//   2. Create C-style call function with matching parameter type
//   3. Create C-style drop function
//   4. Pass data, call, and drop to the C struct initializer
//
// For reference types (OpaquePointer-based):
//   1. Same as value types, but:
//   2. Use (UnsafeRawPointer?, OpaquePointer?, OpaquePointer?) for call signature
//   3. Convert OpaquePointer to Swift type in the call function

class Wrapper<T> {
    let inner: (T, WuiWatcherMetadata) -> Void
    init(_ inner: @escaping (T, WuiWatcherMetadata) -> Void) { self.inner = inner }

}

@MainActor
func callWrapper<T>(
    _ data: UnsafeRawPointer?, _ value: T, _ metadata: OpaquePointer?
) {
    let wrapper = Unmanaged<Wrapper<T>>.fromOpaque(data!).takeUnretainedValue()
    wrapper.inner(value, WuiWatcherMetadata(metadata!))
}

func dropWrapper<T>(_ data: UnsafeMutableRawPointer?, _: T.Type) {
    _ = Unmanaged<Wrapper<T>>.fromOpaque(data!).takeRetainedValue()
}

func wrap<T>(_ f: @escaping (T, WuiWatcherMetadata) -> Void) -> UnsafeMutableRawPointer {
    let wrapper = Wrapper(f)
    return UnsafeMutableRawPointer(Unmanaged.passRetained(wrapper).toOpaque())
}

/// Watcher implementation for Int32 values
extension WuiWatcher_i32: Watcher {
    typealias Output = Int32

    init(_ f: @escaping (Self.Output, WuiWatcherMetadata) -> Void) {
        let data = wrap(f)

        let call: @convention(c) (UnsafeRawPointer?, Int32, OpaquePointer?) -> Void = {
            data, value, metadata in
            callWrapper(data, value, metadata)
        }

        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = {
            dropWrapper($0, Self.Output.self)
        }

        self.init(data: data, call: call, drop: drop)
    }
}

/// Watcher implementation for Bool values
extension WuiWatcher_bool: Watcher {
    typealias Output = Bool
    init(_ f: @escaping (Self.Output, WuiWatcherMetadata) -> Void) {
        let data = wrap(f)

        let call: @convention(c) (UnsafeRawPointer?, Bool, OpaquePointer?) -> Void = {
            data, value, metadata in
            callWrapper(data, value, metadata)
        }

        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = {
            dropWrapper($0, Self.Output.self)
        }

        self.init(data: data, call: call, drop: drop)
    }
}

/// Watcher implementation for Double values
extension WuiWatcher_f64: Watcher {
    typealias Output = Double
    init(_ f: @escaping (Self.Output, WuiWatcherMetadata) -> Void) {
        let data = wrap(f)
        let call: @convention(c) (UnsafeRawPointer?, Double, OpaquePointer?) -> Void = {
            data, value, metadata in
            callWrapper(data, value, metadata)
        }
        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = {
            dropWrapper($0, Self.Output.self)
        }
        self.init(data: data, call: call, drop: drop)
    }
}

/// Watcher implementation for WuiStr values
extension WuiWatcher_WuiStr: Watcher {
    typealias Output = WuiStr
    init(_ f: @escaping (Self.Output, WuiWatcherMetadata) -> Void) {
        let data = wrap(f)
        let call: @convention(c) (UnsafeRawPointer?, CWaterUI.WuiStr, OpaquePointer?) -> Void = {
            data, value, metadata in
            let str = WuiStr(value)
            callWrapper(data, str, metadata)
        }
        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = {
            dropWrapper($0, Self.Output.self)
        }
        self.init(data: data, call: call, drop: drop)
    }
}

extension WuiWatcher_WuiStyledStr: Watcher{
    typealias Output = WuiStyledStr
    init(_ f: @escaping (Self.Output, WuiWatcherMetadata) -> Void) {
        let data = wrap(f)
        let call: @convention(c) (UnsafeRawPointer?, CWaterUI.WuiStyledStr, OpaquePointer?) -> Void = {
            data, value, metadata in
            let str = WuiStyledStr(value)
            callWrapper(data, str, metadata)
        }
        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = {
            dropWrapper($0, Self.Output.self)
        }
        self.init(data: data, call: call, drop: drop)
    }
}

extension WuiWatcher_WuiResolvedFont: Watcher {
    typealias Output = WuiResolvedFont
    init(_ f: @escaping (Self.Output, WuiWatcherMetadata) -> Void) {
        let data = wrap(f)
        let call: @convention(c) (UnsafeRawPointer?, WuiResolvedFont, OpaquePointer?) -> Void = {
            data, value, metadata in
            callWrapper(data, value, metadata)
        }
        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = {
            dropWrapper($0, Self.Output.self)
        }
        self.init(data: data, call: call, drop: drop)
    }
}

extension WuiWatcher_WuiResolvedColor: Watcher {
    typealias Output = WuiResolvedColor
    init(_ f: @escaping (Self.Output, WuiWatcherMetadata) -> Void) {
        let data = wrap(f)
        let call: @convention(c) (UnsafeRawPointer?, WuiResolvedColor, OpaquePointer?) -> Void = {
            data, value, metadata in
            callWrapper(data, value, metadata)
        }
        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = {
            dropWrapper($0, Self.Output.self)
        }
        self.init(data: data, call: call, drop: drop)
    }
}
