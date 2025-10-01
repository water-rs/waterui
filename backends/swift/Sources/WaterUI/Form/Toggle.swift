//
//  Toggle.swift
//
//
//  Created by Lexo Liu on 8/2/24.
//

import Foundation
import CWaterUI
import SwiftUI


public struct WuiToggle:View,WuiComponent{
    static var id=waterui_toggle_id()
    @ObservedObject private var isOn:BindingBool
    var label:WuiAnyView
    init(toggle:CWaterUI.WuiToggle,env:WuiEnvironment){
        isOn = BindingBool(inner: toggle.toggle)
        label = WuiAnyView(anyview: toggle.label, env: env)
    }
    
    init(anyview:OpaquePointer,env:WuiEnvironment){
        self.init(toggle: waterui_force_as_toggle(anyview), env: env)
    }
    
    public var body:some View{
        SwiftUI.Toggle(isOn:isOn.value, label: {
            label
        })
    }
}

