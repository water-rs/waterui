//
//  Toggle.swift
//
//
//  Created by Lexo Liu on 8/2/24.
//

import Foundation
import CWaterUI
import SwiftUI


public struct Toggle:View{
    @ObservedObject private var isOn:BindingBool
    var label:AnyView
    init(toggle:waterui_toggle,env:Environment){
        isOn = BindingBool(inner: toggle.toggle)
        label = AnyView(view: toggle.label, env: env)
    }
    
    init(view:OpaquePointer,env:Environment){
        self.init(toggle: waterui_view_force_as_toggle(view), env: env)
    }
    
    public var body:some View{
        SwiftUI.Toggle(isOn:$isOn.value, label: {
            label
        })
    }
}

