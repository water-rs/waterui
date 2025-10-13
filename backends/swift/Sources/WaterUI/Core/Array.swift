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

   
    
    init(_ inner:CWaterUI.WuiArray){
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
        if let inner = inner{
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
    
    init(array:[T]){
        self.inner = .init(array: array)
    }

    func intoInner() -> CWaterUI.WuiArray {
        self.inner.intoInner()
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

extension WuiArray<CWaterUI.WuiTableColumn> {
    init(_ inner: CWaterUI.WuiArray_WuiTableColumn) {
        let raw = unsafeBitCast(inner, to: CWaterUI.WuiArray.self)
        self.init(c: raw)
    }
}

@MainActor
final class WuiAnyViewCollection {
    private let handleAddress: UInt?
    private let environment: WuiEnvironment

    init(_ handle: UnsafeMutableRawPointer?, env: WuiEnvironment) {
        self.handleAddress = handle.map { UInt(bitPattern: $0) }
        self.environment = env
    }

    private func resolveHandle(_ function: StaticString = #function) -> UnsafeMutableRawPointer {
        guard let address = handleAddress,
              let handle = UnsafeMutableRawPointer(bitPattern: address) else {
            preconditionFailure("\(function): WuiAnyViewCollection handle is no longer valid")
        }
        return handle
    }

    deinit {
        guard let address = handleAddress,
              let handle = UnsafeMutableRawPointer(bitPattern: address) else { return }
        waterui_drop_any_views_opaque(handle)
    }

    var count: Int {
        Int(waterui_any_views_len_opaque(resolveHandle()))
    }

    func view(at index: Int) -> WuiAnyView {
        let handle = resolveHandle()
        let ptr = waterui_any_views_get_view_opaque(handle, UInt(index))
        guard let ptr else {
            preconditionFailure("WuiAnyViewCollection.view(at:): null view pointer for index \(index)")
        }
        return WuiAnyView(anyview: ptr, env: environment)
    }

    func toArray() -> [WuiAnyView] {
        (0..<count).map { view(at: $0) }
    }
}

final class WuiSharedAnyViewCollection {
    private let handleAddress: UInt?
    private let environment: WuiEnvironment

    init(_ handle: UnsafeMutableRawPointer?, env: WuiEnvironment) {
        self.handleAddress = handle.map { UInt(bitPattern: $0) }
        self.environment = env
    }

    private func resolveHandle(_ function: StaticString = #function) -> UnsafeMutableRawPointer {
        guard let address = handleAddress,
              let handle = UnsafeMutableRawPointer(bitPattern: address) else {
            preconditionFailure("\(function): WuiSharedAnyViewCollection handle is no longer valid")
        }
        return handle
    }

    deinit {
        guard let address = handleAddress,
              let handle = UnsafeMutableRawPointer(bitPattern: address) else { return }
        waterui_drop_shared_any_views(handle.assumingMemoryBound(to: WuiSharedAnyViews.self))
    }

    var count: Int {
        Int(waterui_shared_any_views_len_opaque(resolveHandle()))
    }

    func view(at index: Int) -> WuiAnyView {
        let handle = resolveHandle()
        let ptr = waterui_shared_any_views_get_view_opaque(handle, UInt(index))
        guard let ptr else {
            preconditionFailure("WuiSharedAnyViewCollection.view(at:): null view pointer for index \(index)")
        }
        return WuiAnyView(anyview: ptr, env: environment)
    }

    func toArray() -> [WuiAnyView] {
        (0..<count).map { view(at: $0) }
    }
}

struct WuiTableColumnSnapshot {
    let id: Int
    let rows: [WuiAnyView]
}

final class WuiTableColumnCollection {
    private let handleAddress: UInt?
    private let environment: WuiEnvironment

    init(_ handle: UnsafeMutableRawPointer?, env: WuiEnvironment) {
        self.handleAddress = handle.map { UInt(bitPattern: $0) }
        self.environment = env
    }

    private func resolveHandle(_ function: StaticString = #function) -> UnsafeMutablePointer<WuiTableColumns> {
        guard let address = handleAddress,
              let rawHandle = UnsafeMutableRawPointer(bitPattern: address) else {
            preconditionFailure("\(function): WuiTableColumnCollection handle is no longer valid")
        }
        return rawHandle.assumingMemoryBound(to: WuiTableColumns.self)
    }

    deinit {
        guard let address = handleAddress,
              let handle = UnsafeMutableRawPointer(bitPattern: address) else { return }
        waterui_drop_table_columns(handle.assumingMemoryBound(to: WuiTableColumns.self))
    }

    var count: Int {
        Int(waterui_table_columns_len(resolveHandle()))
    }

    private func columnId(at index: Int, handle: UnsafeMutablePointer<WuiTableColumns>) -> Int {
        let rawId = waterui_table_columns_get_id(handle, UInt(index))
        return Int(rawId.inner)
    }

    func snapshot(at index: Int) -> WuiTableColumnSnapshot {
        let handle = resolveHandle()
        let column = waterui_table_columns_get_column(handle, UInt(index))
        guard let rowsHandle = column.rows else {
            preconditionFailure("WuiTableColumnCollection.snapshot(at:): missing row collection for index \(index)")
        }
        let rows = WuiSharedAnyViewCollection(UnsafeMutableRawPointer(rowsHandle), env: environment)
        return WuiTableColumnSnapshot(id: columnId(at: index, handle: handle), rows: rows.toArray())
    }

    func toArray() -> [WuiTableColumnSnapshot] {
        (0..<count).map { snapshot(at: $0) }
    }
}

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

    func intoInner() -> CWaterUI.WuiStr {
        unsafeBitCast(self.inner.intoInner(), to: CWaterUI.WuiStr.self)
    }

}
