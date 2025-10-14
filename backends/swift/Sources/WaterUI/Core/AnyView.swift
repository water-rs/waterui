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
struct Render {
    var map: [WuiTypeId: any WuiComponent.Type]

    init() {
        self.map = [:]
    }

    init(_ components: [any WuiComponent.Type]) {
        self.init()
        for component in components {
            self.map[component.id] = component
        }
    }

    static var main: Render {
        .init([
            WuiEmptyView.self,
            WuiText.self,
            WuiLabel.self,
            WuiButton.self,
            WuiGestureView.self,
            WuiColorView.self,
            // Stack components will be added here when Rust FFI is implemented:
            WuiTextField.self,
            WuiStepper.self,
            // WaterUI.Spacer.self,
            WuiProgress.self,
            // WaterUI.Toggle.self,
            // WaterUI.NavigationView.self,
            WuiDynamic.self,
            // WaterUI.WithEnv.self,
            // WaterUI.NavigationLink.self,
            WuiScrollView.self,
            WuiContainer.self,
            WuiFixedContainer.self,
            WuiList.self,
            WuiTable.self,
            WuiToggle.self,
            WuiSpacer.self,
            WuiRendererView.self,
            //WuiPicker.self,
            //WaterUI.BackgroundColor.self,
            //WaterUI.Rectangle.self,
            //  WaterUI.ForegroundColor.self,
            WuiSlider.self,
            WuiLazy.self,
            WuiList.self,
            WuiListItem.self,

                // WaterUI.ColorPicker.self,
                // WaterUI.Icon.self
        ])
    }

    mutating func register(_ component: any WuiComponent.Type) {
        self.map[component.id] = component
    }

    func render(anyview: OpaquePointer, env: WuiEnvironment) -> SwiftUI.AnyView {
        let id = waterui_view_id(anyview)
        if let ty = map[id] {
            let component = ty.init(anyview: anyview, env: env) as (any View)
            return SwiftUI.AnyView(component)
        } else {

            let next = waterui_view_body(anyview, env.inner)
            return SwiftUI.AnyView(render(anyview: next!, env: env))
        }
    }
}

extension WuiTypeId: @retroactive Hashable {
    public func hash(into hasher: inout Hasher) {
        self.inner.0.hash(into: &hasher)
        self.inner.1.hash(into: &hasher)
    }
}

@MainActor
public struct WuiAnyView: View, Identifiable {
    public var id = UUID()
    var main: any View
    private var anyviewPtr: OpaquePointer
    private var env: WuiEnvironment
    public var typeId: WuiTypeId

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.anyviewPtr = anyview
        self.env = env
        self.typeId = waterui_view_id(anyview)
        self.main = Render.main.render(anyview: anyview, env: env)
    }

    /// Force downcast to a specific component type
    func forceAs<T: WuiComponent>(_ type: T.Type) -> T? {
        let currentId = waterui_view_id(anyviewPtr)
        if currentId == T.id {
            return T(anyview: anyviewPtr, env: env)
        }
        return nil
    }

    /// Check if this view is of a specific component type
    func isType<T: WuiComponent>(_ type: T.Type) -> Bool {
        let currentId = waterui_view_id(anyviewPtr)
        return currentId == T.id
    }

    public var body: some View {
        AnyView(main).id(id)
    }
}
