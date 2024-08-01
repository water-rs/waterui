//
//  Button.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//
import CWaterUI
import SwiftUI

struct Button: View {
    private var label: AnyView
    private var action: Action
    private var app: App

    init(button: waterui_button, app: App) {
        label = AnyView(view: button.label, app: app)
        action = Action(inner: button.action, app: app)
        self.app = app
    }

    var body: some View {
        SwiftUI.Button {
            action.call()
        } label: {
            label
        }
    }
}

class Action {
    private var inner: OpaquePointer
    private var app: App
    init(inner: OpaquePointer, app: App) {
        self.inner = inner
        self.app = app
    }

    func call() {
        app.spawn {
            waterui_call_action(self.inner, self.app.env)
        }
    }

    deinit {
        app.spawn {
            waterui_drop_action(self.inner)
        }
    }
}
