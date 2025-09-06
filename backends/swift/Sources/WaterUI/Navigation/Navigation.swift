//
//  SwiftUIView.swift
//  
//
//  Created by Lexo Liu on 8/26/24.
//

import SwiftUI
import CWaterUI
/*
struct NavigationView: View,Component {
    static var id=waterui_navigation_view_id()
    var content:WaterUI.AnyView
    var title:SwiftUI.Text
    var body: some View {
        content.navigationTitle(title)
    }
    
    init(anyview:OpaquePointer,env:WaterUI.Environment) {
        self.init(navigationView: waterui_force_as_navigation_view(anyview),env:env)
    }
    
    init(navigationView:WuiNavigationView,env:WaterUI.Environment) {
        title=WaterUI.Text(text: navigationView.bar.title).toText()
        content=AnyView(anyview: navigationView.content, env: env)
    }
}

@MainActor
class NavigationViewBuilder{
    var inner:OpaquePointer
    var env:Environment
    init(inner: OpaquePointer,env:Environment) {
        self.inner = inner
        self.env = env
    }
    
    func view() -> NavigationView{
        return NavigationView(navigationView: waterui_navigation_view_builder_call(inner, waterui_clone_env(env.inner)), env: env)
    }
    
    deinit{
     
        
        weak var this=self
        Task{@MainActor in
            if let this=this{
                waterui_drop_navigation_view_builder(this.inner)
            }
        }
        
        
    }
    
    
}


@MainActor
struct NavigationLink: View,Component {
    static var id=navigation_link_id()
    var label:WaterUI.AnyView
    var content:NavigationViewBuilder
    var body: some View {
        SwiftUI.NavigationLink(destination: {
            VStack{
                Spacer()

                HStack{
                    Spacer()

                    content.view()
                    Spacer()

                }
                Spacer()
            }
            
        },label: {
            label
        })
    }
    
    init(anyview:OpaquePointer,env:WaterUI.Environment) {
        self.init(link: force_as_navigation_link(anyview),env:env)
    }
    
    init(link:waterui_navigation_link,env:WaterUI.Environment) {
        label=WaterUI.AnyView(anyview: link.label, env: env)
        content=NavigationViewBuilder(inner: link.content, env: env)
    }
}

*/
