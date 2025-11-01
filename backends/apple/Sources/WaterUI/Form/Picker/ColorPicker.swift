//
//  ColorPicker.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//

import SwiftUI
import CWaterUI


/*
struct WuiColorPicker:View,WuiComponent {
    static let id: String = decodeViewIdentifier(waterui_color_picker_id())
    
    var label:WuiAnyView
    @ObservedObject var value:BindingColor
    @State var color=Color.red
    init(picker: WuiColorPicker, env: Environment) {
        self.label=WuiAnyView(anyview: picker.label, env: env)
        self.value=BindingColor(inner: picker.value)
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(picker: waterui_force_as_color_picker(anyview), env: env)
    }
    var body: some View {
        SwiftUI.ColorPicker(selection:value.value, supportsOpacity: true, label: {
            label
        })
    }
}
*/
