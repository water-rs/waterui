import CWaterUI
import SwiftUI

extension WuiStr {
    init(_ string: String) {
        // Using waterui_str_from_bytes to properly handle strings with null bytes
        let utf8 = Array(string.utf8)
        self = utf8.withUnsafeBufferPointer { buffer in
            // Cast UInt8 to Int8 (char) for C interface
            let charPtr = buffer.baseAddress?.withMemoryRebound(to: Int8.self, capacity: buffer.count) { $0 }
            return waterui_str_from_bytes(charPtr, UInt32(buffer.count))
        }
    }
    
    func toString() -> String {
        // IMPORTANT: WuiStr behaves like Str - it can be either:
        // - A static reference (len >= 0): ptr points to static UTF-8 data  
        // - A reference-counted string (len < 0): ptr points to heap-allocated Shared struct
        //
        // We MUST use FFI functions to safely access the data, never directly read memory!
        
        let length = abs(Int(self.len))
        
        guard length > 0 else {
            return ""
        }
        
        // Get the actual length using FFI function
        var copy = self
        let actualLength = withUnsafePointer(to: &copy) { ptr in
            waterui_str_len(OpaquePointer(ptr))
        }
        
        // Get pointer to the UTF-8 bytes using the new FFI function
        let bytesPtr = withUnsafePointer(to: &copy) { ptr in
            waterui_str_as_ptr(OpaquePointer(ptr))
        }
        
        guard let bytes = bytesPtr else {
            return ""
        }
        
        // Create a buffer from the bytes and convert to String
        let buffer = UnsafeBufferPointer(start: bytes, count: Int(actualLength))
        return String(decoding: buffer, as: UTF8.self)
    }
    
    func drop() {
        waterui_str_drop(self)
    }
}

extension WuiTypeId: Equatable {
    public static func == (lhs: WuiTypeId, rhs: WuiTypeId) -> Bool {
        return lhs.inner.0 == rhs.inner.0 && lhs.inner.1 == rhs.inner.1
    }
}

@MainActor
protocol Component:View{
    static var id: WuiTypeId {get}
    init(anyview: OpaquePointer,env:WaterUI.Environment)
}






@MainActor
public func mainWidget() -> some View{
    let env=Environment(waterui_env_new())
    return NavigationStack{
        WaterUI.AnyView(anyview: waterui_widget_main(), env: env)
    }
    
}
