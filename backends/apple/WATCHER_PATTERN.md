# Watcher Implementation Pattern

This document explains the pattern used to implement the `Watcher` protocol for C-level watcher structs.

## Problem

The WaterUI Apple backend needs to bridge C-level watcher structs (from `CWaterUI`) to Swift. Each watcher type has the same structure but different parameter types, leading to repetitive boilerplate code.

## Solution

While Swift macros were considered for code generation, the FFI (Foreign Function Interface) layer requires specific C-compatible types that don't work well with generic abstractions. The final approach uses a well-documented pattern with clear examples.

## Implementation Pattern

### For Value Types (Int32, Bool, Double, etc.)

```swift
extension WuiWatcher_TypeName: Watcher {
    typealias Output = SwiftType
    
    init(_ f: @escaping (SwiftType, WuiWatcherMetadata) -> Void) {
        // 1. Create a wrapper class to hold the Swift closure
        class Wrapper {
            let inner: (SwiftType, WuiWatcherMetadata) -> Void
            init(_ inner: @escaping (SwiftType, WuiWatcherMetadata) -> Void) { self.inner = inner }
        }

        // 2. Retain the wrapper as opaque pointer
        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        // 3. Create C-style call function with matching type
        let call: @convention(c) (UnsafeRawPointer?, SwiftType, OpaquePointer?) -> Void = {
            data, value, metadata in
            let mutableData = UnsafeMutableRawPointer(mutating: data!)
            let wrapper = Unmanaged<Wrapper>.fromOpaque(mutableData).takeUnretainedValue()
            wrapper.inner(value, WuiWatcherMetadata(metadata!))
        }

        // 4. Create C-style drop function for cleanup
        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = { data in
            guard let data else { return }
            _ = Unmanaged<Wrapper>.fromOpaque(data).takeRetainedValue()
        }

        // 5. Initialize the C struct
        self.init(data: data, call: call, drop: drop)
    }
}
```

### For Reference Types (OpaquePointer-based)

```swift
extension WuiWatcher_TypeName: Watcher {
    typealias Output = SwiftType
    
    init(_ f: @escaping (SwiftType, WuiWatcherMetadata) -> Void) {
        class Wrapper {
            let inner: (SwiftType, WuiWatcherMetadata) -> Void
            init(_ inner: @escaping (SwiftType, WuiWatcherMetadata) -> Void) { self.inner = inner }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        // Note: Use OpaquePointer? for the value parameter
        let call: @convention(c) (UnsafeRawPointer?, OpaquePointer?, OpaquePointer?) -> Void = {
            data, value, metadata in
            let wrapper = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue()
            // Convert OpaquePointer to Swift type
            wrapper.inner(SwiftType(value!), WuiWatcherMetadata(metadata!))
        }

        let drop: @convention(c) (UnsafeMutableRawPointer?) -> Void = { data in
            guard let data else { return }
            _ = Unmanaged<Wrapper>.fromOpaque(data).takeRetainedValue()
        }

        self.init(data: data, call: call, drop: drop)
    }
}
```

## Key Differences

1. **Value types**: Use the Swift type directly in the `call` signature (e.g., `Int32`, `Bool`, `Double`)
2. **Reference types**: Use `OpaquePointer?` in the `call` signature and convert to Swift type inside the closure

## Why Not Use Macros?

While Swift macros can generate code, they have limitations with FFI:
- C function types with `@convention(c)` cannot be genericized
- Each type needs its exact signature at compile time
- The pattern is simple enough that clear documentation is more valuable than macro magic

## Adding New Watcher Types

To add a new watcher type:
1. Identify if it's a value or reference type
2. Copy the appropriate template above  
3. Replace `TypeName` with the C struct name
4. Replace `SwiftType` with the Swift type
5. For reference types, ensure the conversion function is correct

## Examples

See `Watcher.swift` for complete implementations of:
- `WuiWatcher_i32` (value type)
- `WuiWatcher_bool` (value type)
- `WuiWatcher_f64` (value type)
- `WuiWatcher_____WuiFont` (reference type)
