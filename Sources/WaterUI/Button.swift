//
//  Button.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//
import CWaterUI
import SwiftUI

@MainActor
struct Button: View {
    private var label: AnyView
    private var action: Action

    init(button: waterui_button, env: WaterUI.Environment) {
        label = AnyView(view: button.label, env: env)
        action = Action(inner: button.action)
    }

    var body: some View {
        SwiftUI.Button {
            action.call()
        } label: {
            label
        }
    }
}

@MainActor
class Action {
    private var inner: OpaquePointer
    init(inner: OpaquePointer) {
        self.inner = inner
    }

    func call() {
        waterui_call_action(self.inner)
    }

    deinit {
        waterui_drop_action(self.inner)
    }
}
