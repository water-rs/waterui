//
//  Toggle.swift
//
//
//  Created by Lexo Liu on 8/2/24.
//

import Foundation
import CWaterUI
import SwiftUI


public struct Toggle:View,Component{
    static var id=waterui_view_toggle_id()
    @ObservedObject private var isOn:BindingBool
    var label:AnyView
    init(toggle:waterui_toggle,env:Environment){
        isOn = BindingBool(inner: toggle.toggle)
        label = AnyView(anyview: toggle.label, env: env)
    }
    
    init(anyview:OpaquePointer,env:Environment){
        self.init(toggle: waterui_view_force_as_toggle(anyview), env: env)
    }
    
    public var body:some View{
        SwiftUI.Toggle(isOn:isOn.value, label: {
            label
        })
    }
}

