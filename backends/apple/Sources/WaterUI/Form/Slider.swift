//
//  Slider.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//

import SwiftUI
import CWaterUI
struct WuiSlider: View, WuiComponent{
    static let id: String = decodeViewIdentifier(waterui_slider_id())
    var label:WuiAnyView
    var min_value_label: WuiAnyView
    var max_value_label: WuiAnyView
    var range: WuiRange_f64
    @State var value: WuiBinding<Double>
    var body: some View {
        SwiftUI.Slider(value: $value.value, in: range.start...range.end, label: {
            label
        }, minimumValueLabel: {
            min_value_label
        }, maximumValueLabel: {
            max_value_label
        })
    }
    
    init(slider: CWaterUI.WuiSlider, env:WuiEnvironment){
        self.label=WuiAnyView(anyview: slider.label, env: env)
        self.min_value_label=WuiAnyView(anyview: slider.min_value_label, env: env)
        self.max_value_label=WuiAnyView(anyview: slider.max_value_label, env: env)
        self.range=slider.range
        self.value=WuiBinding(slider.value)
    }
    
    init(anyview: OpaquePointer,env:WuiEnvironment){
        self.init(slider: waterui_force_as_slider(anyview), env: env)
    }
    
}
