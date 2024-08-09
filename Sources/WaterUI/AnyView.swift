//
//  AnyView.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI




@MainActor
@SwiftUI.ViewBuilder
private func render(view: OpaquePointer, env: WaterUI.Environment)-> some View {
    
    switch waterui_view_id(view) {
    case waterui_view_empty_id():
        EmptyView()
    case waterui_view_text_id():
        WaterUI.Text(text: waterui_view_force_as_text(view))
    case waterui_view_stack_id():
        WaterUI.Stack(stack: waterui_view_force_as_stack(view), env : env)
    case waterui_view_button_id():
        WaterUI.Button(button: waterui_view_force_as_button(view), env : env)
    case waterui_view_text_field_id():
        WaterUI.TextField(view: view, env : env)
    case waterui_view_stepper_id():
        WaterUI.Stepper(view: view)
        EmptyView()
    case waterui_view_progress_id():
        WaterUI.Progress(view:view,env:env)
    
    case waterui_view_toggle_id():
         WaterUI.Toggle(view:view, env : env)
    case waterui_view_with_env_id():
        let v=waterui_view_force_as_with_env(view)
        WaterUI.AnyView(view:v.view,env:Environment(v.env))
        
    default:
        SwiftUI.AnyView(render(view: waterui_view_body(view, waterui_clone_env(env.inner)), env : env))
    }
}

@MainActor
public struct AnyView: View {
    var main: any View

    // Must be called on Rust thread
    init(view: OpaquePointer, env : WaterUI.Environment) {
        main = render(view: view, env : env)
    }

    public var body: some View {
        SwiftUI.AnyView(main)
    }
}
