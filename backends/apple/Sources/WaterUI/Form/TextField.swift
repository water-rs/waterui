//
//  TextField.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

public struct WuiTextField: View, WuiComponent {
    public static let id: String = decodeViewIdentifier(waterui_text_field_id())
    
    private var label: WuiAnyView
    @State private var prompt: WuiText
    @State private var value: WuiBinding<WuiStr>

    public init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(field: waterui_force_as_text_field(anyview), env: env)
    }
    
    init(field: CWaterUI.WuiTextField, env: WuiEnvironment) {
        self.label = WuiAnyView(anyview: field.label, env: env)
        self.value = WuiBinding(field.value)
        self.prompt = WuiText(text: field.prompt, env: env)
        
    }

    public var body: some View {
        SwiftUI.TextField(text: Binding(get: {
            value.value.toString()
        }, set: {
            value.value = WuiStr(string:$0)
        }), prompt: prompt.toText(), label: {
            label
        })
        
    }

}
