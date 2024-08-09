//
//  TextField.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

public struct TextField: View {
    private var label: WaterUI.AnyView
    
    private var prompt: WaterUI.Text
    @ObservedObject var value: BindingStr


    init(view: OpaquePointer, env: Environment) {
        self.init(field: waterui_view_force_as_text_field(view), env: env)
    }

    init(field: waterui_text_field, env: Environment) {
        label = WaterUI.AnyView(view: field.label, env: env)
        prompt = WaterUI.Text(text: field.prompt)
        value = BindingStr(inner: field.value)
    }

    public var body: some View {
        SwiftUI.TextField(text:$value.value, prompt: prompt.toText(), label: {label})
    }
}
