import CWaterUI
import SwiftUI

extension waterui_str {
    init(_ string: String) {
        self.init()
        withUnsafeBytes(of: string.utf8){ptr in
            self = waterui_new_str(ptr.baseAddress, UInt(ptr.count))
        }
    }
    
    func toString() -> String {
        let len = Int(self.len)
        let ref=UnsafeBufferPointer(start: waterui_str_get_head(self), count: len)
        let data = Data(ref)
        
        waterui_free_str(self)
        
        return String(decoding: data, as: UTF8.self)
    }
}

extension waterui_type_id: Equatable {
    public static func == (lhs: waterui_type_id, rhs: waterui_type_id) -> Bool {
        return lhs.inner == rhs.inner
    }
}

@MainActor
protocol Component:View{
    static var id:waterui_type_id {get}
    init(anyview: OpaquePointer,env:WaterUI.Environment)
}






@MainActor
public func mainWidget() -> some View{
    let env=Environment(waterui_init())
    return NavigationStack{
        WaterUI.AnyView(anyview: waterui_widget_main(), env: env)
    }
    
}
