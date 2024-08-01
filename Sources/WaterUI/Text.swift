//
//  Text.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import SwiftUI

struct Text: View {
    var computedContent: ComputedStr
    @State var content: String = ""

    init(text: waterui_text, app: App) {
        computedContent = ComputedStr(inner: text.content, app: app)
    }

    var body: some View {
        VStack {
            SwiftUI.Text(content)

        }.task {
            content = await computedContent.compute()
            computedContent.watch { value in
                content = value
            }
        }
    }
}
