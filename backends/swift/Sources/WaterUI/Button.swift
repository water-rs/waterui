//
//  Button.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//
import CWaterUI
import SwiftUI

@MainActor
struct Button: View, Component {
    static var id = waterui_button_id()
    private var label: AnyView
    private var action: Action

    init(button: WuiButton, env: WaterUI.Environment) {
        label = AnyView(anyview: button.label, env: env)
        action = Action(inner: button.action, env: env)
    }

    init(anyview: OpaquePointer, env: Environment) {
        self.init(button: waterui_force_as_button(anyview), env: env)
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
    private var env: WaterUI.Environment
    init(inner: OpaquePointer, env: WaterUI.Environment) {
        self.inner = inner
        self.env = env
    }

    func call() {
        waterui_call_action(self.inner, env.inner)
    }

    deinit {

        weak var this = self
        Task { @MainActor in
            if let this = this {
                waterui_drop_action(this.inner)
            }
        }

    }
}
