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

@MainActor
final class WuiRawArray {
    var inner: CWaterUI.WuiArray

   
    
    init(_ inner:CWaterUI.WuiArray){
        self.inner = inner
    }
    
    init<T>(array:[T]){
        let contiguousArray = ContiguousArray(array)
        
        // Create vtable functions that don't capture generic parameters
        let dropFunction: @convention(c) (UnsafeMutableRawPointer?) -> Void = { ptr in
            guard let ptr = ptr else { return }
            Unmanaged<AnyObject>.fromOpaque(ptr).release()
        }
        
        // For slice, we need to store the element size and stride information
        let elementSize = MemoryLayout<T>.size
        let elementStride = MemoryLayout<T>.stride
        
        let sliceFunction: @convention(c) (UnsafeRawPointer?) -> WuiArraySlice = { ptr in
            guard let ptr = ptr else {
                return WuiArraySlice(head: nil, len: 0)
            }
            
            // Get the stored array information
            let box = Unmanaged<AnyObject>.fromOpaque(ptr).takeUnretainedValue()
            
            // We stored both the array and metadata
            if let arrayInfo = box as? ArrayInfo {
                return WuiArraySlice(
                    head: arrayInfo.baseAddress,
                    len: UInt(arrayInfo.count)
                )
            }
            
            return WuiArraySlice(head: nil, len: 0)
        }
        
        let vtable = WuiArrayVTable(drop: dropFunction, slice: sliceFunction)
        
        // Create array info that stores the buffer pointer and count
        let innerArray = contiguousArray.withUnsafeBufferPointer { buffer in
            let arrayInfo = ArrayInfo(
                baseAddress: UnsafeMutableRawPointer(mutating: buffer.baseAddress),
                count: buffer.count,
                elementSize: elementSize,
                retainedArray: contiguousArray
            )
            let ptr = Unmanaged.passRetained(arrayInfo as AnyObject).toOpaque()
            return CWaterUI.WuiArray(data: ptr, vtable: vtable)
        }
        
        self.inner = innerArray
    }
    
    
    subscript<T>(index:Int) -> T {
        get {
            let slice = (inner.vtable.slice)(inner.data)
            let head = slice.head!
            let len = Int(slice.len)
            precondition(index >= 0 && index < len, "Index out of bounds")
            let elementPtr = head.advanced(by: index)
            return elementPtr.withMemoryRebound(to: T.self, capacity: 1) {
                $0.pointee
            }
        }
        
        set{
            let slice = (inner.vtable.slice)(inner.data)
            let head = slice.head!
            let len = Int(slice.len)
            precondition(index >= 0 && index < len, "Index out of bounds")
            let elementPtr = head.advanced(by: index)
            elementPtr.withMemoryRebound(to: T.self, capacity: 1) {
                $0.pointee = newValue
                
            }
        }
    }
    
    func toArray<T>() -> [T]{
        let slice = (inner.vtable.slice)(inner.data)
        let head = slice.head!.assumingMemoryBound(to: T.self)
        let len = Int(slice.len)
        // Copy to Swift array
        let buffer = UnsafeBufferPointer<T>(start: head, count: len)
        
        return Array(buffer)
            
    }
    

    deinit {// Some type may not thread-safe...So let's deinit it on main thread!
        let this = self
        Task { @MainActor in
            (this.inner.vtable.drop)(this.inner.data)

        }
    }
}

@MainActor
struct WuiArray<T> {
    var inner: WuiRawArray
    
    init(raw: WuiRawArray) {
        self.inner = raw
    }
    
    init(c: CWaterUI.WuiArray) {
        self.inner = .init(c)
    }
    
    init(array:[T]){
        self.inner = .init(array: array)
    }
    
    subscript(index:Int) -> T {
        get {
            self.inner[index]
        }
        
        set{
            self.inner[index] = newValue
        }
    }
    
    func toArray() -> [T]{
        self.inner.toArray()
    }
}

extension WuiArray<UInt8>{
    init(_ inner:CWaterUI.WuiArray_u8){
        let raw = unsafeBitCast(inner,to:CWaterUI.WuiArray.self)
        self.init(c: raw)
    }
}

@MainActor
struct WuiAnyViews{
    var inner: WuiArray<OpaquePointer>
    var env: WuiEnvironment
    // WARN: NEED WRAP later
    init(_ inner:CWaterUI.WuiArray_____WuiAnyView,env:WuiEnvironment){
        let raw = unsafeBitCast(inner,to:CWaterUI.WuiArray.self)
        self.inner = WuiArray(c: raw)
        self.env = env
    }

    
    mutating func take(index:Int) -> WuiAnyView{
        let ptr = self.inner[index]
        self.inner[index] = waterui_empty_anyview()
        return WuiAnyView(anyview: ptr, env: env)
    }
    
    mutating func toArray() -> [WuiAnyView]{
        // Need set original buffer to all empty anyview, preventing double free
        
        var result: [WuiAnyView] = []
        for i in 0..<self.inner.toArray().count{
            result.append(take(index: i))
        }
        
        return result
        
    }
    
}


@MainActor
struct WuiStr{
    var inner: WuiArray<UInt8>
    
    init(_ inner: CWaterUI.WuiStr) {
        self.inner = WuiArray<UInt8>(inner._0)
    }
    
    init(string:String){
        let bytes = [UInt8](string.utf8)
        self.inner = WuiArray<UInt8>(array:bytes)
    }
    
    func toString() -> String{
        let bytes = inner.toArray()
        return String(bytes: bytes, encoding: .utf8)!
    }
    
    func toCWuiStr() -> CWaterUI.WuiStr{
        .init(_0: unsafeBitCast(self.inner.inner, to: CWaterUI.WuiArray_u8.self))
    }
    
}
