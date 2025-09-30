//
//  TextField.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

public struct WuiTextField: View,WuiComponent {
    static var id=waterui_text_field_id()
    private var label: WuiAnyView
    
    private var prompt: Text
    @ObservedObject var value: BindingStr


    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(field: waterui_force_as_text_field(anyview), env: env)
    }

    init(field: CWaterUI.WuiTextField, env: WuiEnvironment) {
        label = WuiAnyView(anyview: field.label, env: env)
        prompt = WuiText(text: field.prompt).toText()
        value =  BindingStr(inner: field.value)
    }

    public var body: some View {
        TextField(text:value.value, prompt: prompt, label: {label})
    }
}
