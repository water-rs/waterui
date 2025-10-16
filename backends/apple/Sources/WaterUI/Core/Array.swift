//
//  Array.swift
//  waterui-swift
//
//  Created by Lexo Liu on 9/30/25.
//

import CWaterUI

// Helper class to store array information without generic parameters
private final class ArrayInfo {
    let baseAddress: UnsafeMutableRawPointer?
    let count: Int
    let elementSize: Int
    let retainedArray: Any  // Keeps the original array alive

    init(baseAddress: UnsafeMutableRawPointer?, count: Int, elementSize: Int, retainedArray: Any) {
        self.baseAddress = baseAddress
        self.count = count
        self.elementSize = elementSize
        self.retainedArray = retainedArray
    }
}

final class WuiRawArray {
    private var inner: CWaterUI.WuiArray?

    init(_ inner: CWaterUI.WuiArray) {
        self.inner = inner
    }

    func intoInner() -> CWaterUI.WuiArray {
        let v = inner!
        inner = nil
        return v
    }

    init<T>(array: [T]) {
        let contiguousArray = ContiguousArray(array)

        // Simplified drop function
        let dropFunction: @convention(c) (UnsafeMutableRawPointer?) -> Void = { ptr in
            guard let ptr = ptr else { return }
            // This releases the ArrayInfo object
            _ = Unmanaged<AnyObject>.fromOpaque(ptr).takeRetainedValue()
        }

        let sliceFunction: @convention(c) (UnsafeRawPointer?) -> WuiArraySlice = { ptr in
            guard let ptr = ptr else {
                return WuiArraySlice(head: nil, len: 0)
            }

            let box = Unmanaged<AnyObject>.fromOpaque(ptr).takeUnretainedValue()
            if let arrayInfo = box as? ArrayInfo {
                return WuiArraySlice(
                    head: arrayInfo.baseAddress,
                    len: UInt(arrayInfo.count)
                )
            }

            return WuiArraySlice(head: nil, len: 0)
        }

        let vtable = WuiArrayVTable(drop: dropFunction, slice: sliceFunction)

        let innerArray = contiguousArray.withUnsafeBufferPointer { buffer in
            let arrayInfo = ArrayInfo(
                baseAddress: UnsafeMutableRawPointer(mutating: buffer.baseAddress),
                count: buffer.count,
                elementSize: MemoryLayout<T>.size,
                retainedArray: contiguousArray
            )
            let ptr = Unmanaged.passRetained(arrayInfo as AnyObject).toOpaque()
            return CWaterUI.WuiArray(data: ptr, vtable: vtable)
        }

        self.inner = innerArray
    }

    subscript<T>(index: Int) -> T {
        get {
            let slice = (inner!.vtable.slice)(inner!.data)
            let head = slice.head!
            let len = Int(slice.len)
            precondition(index >= 0 && index < len, "Index out of bounds")

            let typedPtr = head.assumingMemoryBound(to: T.self)
            return typedPtr[index]
        }

        set {
            let slice = (inner!.vtable.slice)(inner!.data)
            let head = slice.head!
            let len = Int(slice.len)
            precondition(index >= 0 && index < len, "Index out of bounds")

            let typedPtr = head.assumingMemoryBound(to: T.self)
            typedPtr[index] = newValue
        }
    }

    func toArray<T>() -> [T] {
        let slice = (inner!.vtable.slice)(inner!.data)
        let len = Int(slice.len)
        guard len > 0, let head = slice.head else {
            return []
        }

        let typedHead = head.assumingMemoryBound(to: T.self)
        let buffer = UnsafeBufferPointer<T>(start: typedHead, count: len)
        return Array(buffer)
    }

    @MainActor deinit {
        if let inner = inner {
            inner.vtable.drop(inner.data)
        }

    }
}

struct WuiArray<T> {
    var inner: WuiRawArray

    init(raw: WuiRawArray) {
        self.inner = raw
    }

    init(c: CWaterUI.WuiArray) {
        self.inner = .init(c)
    }

    init(array: [T]) {
        self.inner = .init(array: array)
    }

    func intoInner() -> CWaterUI.WuiArray {
        self.inner.intoInner()
    }

    subscript(index: Int) -> T {
        get {
            self.inner[index]
        }

        set {
            self.inner[index] = newValue
        }
    }

    func toArray() -> [T] {
        self.inner.toArray()
    }
}

extension WuiArray<UInt8> {
    init(_ inner: CWaterUI.WuiArray_u8) {
        let raw = unsafeBitCast(inner, to: CWaterUI.WuiArray.self)
        self.init(c: raw)
    }
}

extension WuiArray<OpaquePointer> {
    init(_ inner: CWaterUI.WuiArray_____WuiAnyView) {
        let raw = unsafeBitCast(inner, to: CWaterUI.WuiArray.self)
        self.init(c: raw)
    }

}

extension WuiArray<CWaterUI.WuiStyledChunk> {
    init(_ inner: CWaterUI.WuiArray_WuiStyledChunk) {
        let raw = unsafeBitCast(inner, to: CWaterUI.WuiArray.self)
        self.init(c: raw)
    }
}

struct WuiStr {
    var inner: WuiArray<UInt8>

    init(_ inner: CWaterUI.WuiStr) {
        self.inner = WuiArray<UInt8>(inner._0)
    }

    init(string: String) {
        let bytes = [UInt8](string.utf8)
        self.inner = WuiArray<UInt8>(array: bytes)
    }

    func toString() -> String {
        let bytes = inner.toArray()
        return String(bytes: bytes, encoding: .utf8)!
    }

    func intoInner() -> CWaterUI.WuiStr {
        unsafeBitCast(self.inner.intoInner(), to: CWaterUI.WuiStr.self)
    }

}
