//
//  SwiftUIView.swift
//  
//
//  Created by Lexo Liu on 8/27/24.
//

import SwiftUI
import CWaterUI
/*
struct Tabs: View, Component {
    static var id=tabs_id()
    var selection:BindingInt
    var tabs:Array<Tab>
    init(anyview: OpaquePointer, env: Environment) {
        self.init(tabs: force_as_tabs(anyview), env: env)
    }
    
    init(tabs: waterui_tabs, env: Environment) {
        self.tabs=tabs.tabs.toArray(env: env)
        self.selection=BindingInt(inner: tabs.selection)
    }
    
    var body: some View {
        SwiftUI.TabView(selection: selection.value){
            ForEach(tabs, id: \.tag, content: {tab in
                tab.content.view()
            })
        }
                       
            
        
    }
}

@MainActor
extension waterui_array_waterui_tab{
    func toArray(env:Environment) -> Array<Tab>{
        let array=Array(UnsafeBufferPointer<waterui_tab>(start: self.head, count: Int(self.len)))
        return array.map{tab in
            WaterUI.Tab(tab: tab, env: env)
        }
    }
}


@MainActor
struct Tab{
    
    var label:WaterUI.AnyView
    var tag:Int
    var content:NavigationViewBuilder
    
    init(tab:waterui_tab,env:Environment){
        self.label=AnyView(anyview: tab.label, env: env)
        self.tag=Int(tab.tag)
        self.content=NavigationViewBuilder(inner: tab.content, env: env)
    }
}
*/
