//
//  Text.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import SwiftUI



@MainActor
struct Text: View {
    @ObservedObject var content: ComputedStr
    init(text: waterui_text) {
        self.content = ComputedStr(inner: text.content)
    }
    
    init(view: OpaquePointer, env: WaterUI.Environment) {
        self.init(text: waterui_view_force_as_text(view))
    }
    
    func toText() -> SwiftUI.Text{
        return SwiftUI.Text(content.value)
    }

    var body: some View {
        toText()
    }
}
