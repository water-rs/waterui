//
//  Text.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import Foundation
import SwiftUI

// MARK: - Text View

struct WuiText: View, WuiComponent {
    static var id: CWaterUI.WuiTypeId {
        waterui_text_id()
    }

    @State private var content: WuiComputed<WuiStyledStr>
    private var env: WuiEnvironment

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(text: waterui_force_as_text(anyview), env: env)
    }

    init(text: CWaterUI.WuiText, env: WuiEnvironment) {
        self.env = env
        self.content = WuiComputed(text.content)
    }

    init(styledStr: WuiComputed<WuiStyledStr>, env: WuiEnvironment) {
        self.env = env
        self.content = styledStr
    }

    func toText() -> SwiftUI.Text {
        SwiftUI.Text(content.value.toAttributedString(env: env))
    }

    var body: some View {
        toText()
    }

}
