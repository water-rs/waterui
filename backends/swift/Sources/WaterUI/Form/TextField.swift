//
//  TextField.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

public struct TextField: View,Component {
    static var id=waterui_text_field_id()
    private var label: WaterUI.AnyView
    
    private var prompt: SwiftUI.Text
    @ObservedObject var value: BindingStr


    init(anyview: OpaquePointer, env: Environment) {
        self.init(field: waterui_force_as_text_field(anyview), env: env)
    }

    init(field: WuiTextField, env: Environment) {
        label = WaterUI.AnyView(anyview: field.label, env: env)
        prompt = WaterUI.Text(text: field.prompt).toText()
        value =  BindingStr(inner: field.value)
    }

    public var body: some View {
        SwiftUI.TextField(text:value.value, prompt: prompt, label: {label})
    }
}
