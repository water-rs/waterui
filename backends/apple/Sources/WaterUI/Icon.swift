//
//  Icon.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//

/*
import SwiftUI
import CWaterUI
struct Icon:View,Component{
    static var id=icon_id()
    @State var name:ComputedStr
    @ObservedObject var size:ComputedDouble
    init(anyview: OpaquePointer, env: Environment) {
        self.init(icon: force_as_icon(anyview), env: env)
    }
    
    init(icon: waterui_icon, env: Environment) {
        self.name=ComputedStr(inner: icon.name)
        self.size=ComputedDouble(inner: icon.size)
    }
    
    var body: some View {
        let font=size.value.checkNaN().map{
            SwiftUI.Font.system(size:$0)
        }
    
        SwiftUI.Image(systemName: name.value).font(font)
    }
}
*/
