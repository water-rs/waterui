//
//  Label.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/8/25.
//

import SwiftUI
import CWaterUI

struct WuiPlain: WuiComponent, View{
    var label: WuiStr
    static let id: String = Self.decodeId(waterui_plain_id())
    
    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.label = WuiStr(waterui_force_as_plain(anyview))
    }
    
    var body: some View{
        SwiftUI.Text(label.toString())
    }
}
