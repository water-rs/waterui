//
//  Label.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/8/25.
//

import SwiftUI
import CWaterUI

struct WuiLabel: WuiComponent,View{
    var label: WuiStr
    static var id: WuiTypeId {
        waterui_label_id()
    }
    
    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.label = WuiStr(waterui_force_as_label(anyview))
    }
    
    var body: some View{
        SwiftUI.Text(label.toString())
    }
}
