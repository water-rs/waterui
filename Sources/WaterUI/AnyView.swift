//
//  AnyView.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

// Must be called on Rust thread
@SwiftUI.ViewBuilder
private func render(view: OpaquePointer, app: App) -> some View {
    switch waterui_view_id(view) {
    case waterui_view_empty_id():
        EmptyView()
    case waterui_view_text_id():
        WaterUI.Text(text: waterui_view_force_as_text(view), app: app)
    case waterui_view_stack_id():
        WaterUI.Stack(stack: waterui_view_force_as_stack(view), app: app)
    case waterui_view_button_id():
        WaterUI.Button(button: waterui_view_force_as_button(view), app: app)
    default:
        SwiftUI.AnyView(render(view: waterui_view_body(view, app.env), app: app))
    }
}

public struct AnyView: View {
    var main: any View

    // Must be called on Rust thread
    init(view: OpaquePointer, app: App) {
        main = render(view: view, app: app)
    }

    public var body: some View {
        SwiftUI.AnyView(main)
    }
}
