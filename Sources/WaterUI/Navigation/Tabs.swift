//
//  SwiftUIView.swift
//  
//
//  Created by Lexo Liu on 8/27/24.
//

import SwiftUI
import CWaterUI

struct Tabs: View, Component {
    static var id=waterui_view_tabs_id()
    init(anyview: OpaquePointer, env: Environment) {
        <#code#>
    }
    
    init(tabs: waterui_tabs, env: Environment) {
        
    }
    var body: some View {
        SwiftUI.TabView(selection: .constant(1)){
            
        }
                       
            
        
    }
}

typealias Id=UInt

struct Tab{
    var label:WaterUI.AnyView
    var tag:Id
    var
    content: *mut waterui_navigation_view_builder,
}
