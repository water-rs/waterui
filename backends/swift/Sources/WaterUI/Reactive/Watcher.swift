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
    
    init(_ f: @escaping (Output, Animation?) -> Void)
    
}

/// A factory for creating C-compatible watcher structs from Swift closures.
@MainActor
struct FfiWatcherFactory<SwiftType, CType> {
    private class Wrapper {
        let inner: (SwiftType, Animation?) -> Void
        init(_ inner: @escaping (SwiftType, Animation?) -> Void) { self.inner = inner }
    }

    let data: UnsafeMutableRawPointer
    let call: (@convention(c) (UnsafeRawPointer?, CType, UnsafePointer<CWaterUI.Metadata>?) -> Void)
    let drop: (@convention(c) (UnsafeMutableRawPointer?) -> Void)

    init(_ f: @escaping (SwiftType, Animation?) -> Void, transform: @escaping (CType) -> SwiftType) {
        self.data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())
        
        let call: @convention(c) (UnsafeRawPointer?, CType, UnsafePointer<CWaterUI.Metadata>?) -> Void = { data, value, metadataPtr in
            guard let data else { return }
            let mutableData = UnsafeMutableRawPointer(mutating: data)
            let wrapper = Unmanaged<Wrapper>.fromOpaque(mutableData).takeUnretainedValue()
            let swiftValue = transform(value)

            let animation: Animation?
            if let metadataPtr {
                animation = WuiWatcherMetadata(OpaquePointer(metadataPtr)).getAnimation()
            } else {
                animation = nil
            }

            wrapper.inner(swiftValue, animation)
        }
        self.call = call

        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = { data in
            guard let data else { return }
            _ = Unmanaged<Wrapper>.fromOpaque(data).takeRetainedValue()
        }
        self.drop = drop
    }
    
    init(_ f: @escaping (SwiftType, Animation?) -> Void) where SwiftType == CType {
        self.init(f, transform: { $0 })
    }
}
