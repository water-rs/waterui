//
//  Slider.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//

import SwiftUI
import CWaterUI
struct Slider:View,Component{
    static var id=waterui_view_slider_id()
    var label:WaterUI.AnyView
    var min_value_label: WaterUI.AnyView
    var max_value_label: WaterUI.AnyView
    var range: waterui_range_inclusive_f64
    @ObservedObject var value: BindingDouble
    var body: some View {
        SwiftUI.Slider(value: value.value, in: range.start...range.end, label: {
            label
        }, minimumValueLabel: {
            min_value_label
        }, maximumValueLabel: {
            max_value_label
        })
    }
    
    init(slider: waterui_slider,env:WaterUI.Environment){
        self.label=AnyView(anyview: slider.label, env: env)
        self.min_value_label=AnyView(anyview: slider.min_value_label, env: env)
        self.max_value_label=AnyView(anyview: slider.max_value_label, env: env)
        self.range=slider.range
        self.value=BindingDouble(inner: slider.value)
    }
    
    init(anyview: OpaquePointer,env:WaterUI.Environment){
        self.init(slider: waterui_view_force_as_slider(anyview), env: env)
    }
    
}
