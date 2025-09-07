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
struct Render{
    var map:[WuiTypeId:any Component.Type]
    
    init() {
        self.map = [:]
    }
    
    init(_ components: [any Component.Type]){
        self.init()
        for component in components {
            self.map[component.id]=component
        }
    }
    
    static var main:Render{
        .init([
            WaterUI.EmptyView.self,
            WaterUI.Text.self,
            WaterUI.Button.self,
            WaterUI.Stack.self,
            WaterUI.TextField.self,
            WaterUI.Stepper.self,
            WaterUI.Divider.self,
            WaterUI.Spacer.self,
           WaterUI.Progress.self,
            WaterUI.Toggle.self,
           // WaterUI.NavigationView.self,
            WaterUI.Dynamic.self,
           // WaterUI.WithEnv.self,
           // WaterUI.NavigationLink.self,
            WaterUI.ScrollView.self,
           // WaterUI.Picker.self,
            //WaterUI.BackgroundColor.self,
            //WaterUI.Rectangle.self,
          //  WaterUI.ForegroundColor.self,
           // WaterUI.Frame.self,
            WaterUI.Slider.self,
            WaterUI.Label.self,
           // WaterUI.ColorPicker.self,
            //WaterUI.Padding.self,
           // WaterUI.Icon.self
        ])
    }
    
    mutating func register(_ component:any Component.Type){
        self.map[component.id]=component
    }
    
    @ViewBuilder
    func render(anyview: OpaquePointer, env: WaterUI.Environment) -> some View{
        let id = waterui_view_id(anyview)
        if let ty = map[id]{
            let component=ty.init(anyview: anyview, env: env) as (any View)
            SwiftUI.AnyView(component)
        }
        else{
            SwiftUI.AnyView(render(anyview: waterui_view_body(anyview, waterui_clone_env(env.inner)), env: env))
        }
    }
}

extension WuiTypeId:@retroactive Hashable{
    public func hash(into hasher: inout Hasher) {
        self.inner.0.hash(into: &hasher)
        self.inner.1.hash(into: &hasher)
    }
}



@MainActor
public struct AnyView: View,Identifiable {
    public var id = UUID()
    var main: any View
    
    init(anyview: OpaquePointer, env : WaterUI.Environment) {
        main = Render.main.render(anyview: anyview, env : env)
    }
    
    public var body: some View {
        SwiftUI.AnyView(main).id(id)
    }
}
