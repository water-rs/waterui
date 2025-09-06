//
//  Scroll.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/20/24.
//
import SwiftUI
import CWaterUI

struct ScrollView:View,Component{
    static var id=waterui_view_scroll_id()
    var axis:SwiftUI.Axis.Set
    var content:WaterUI.AnyView
    var body: some View{
        SwiftUI.ScrollView(axis, content: {
            content
        })
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(scroll: waterui_view_force_as_scroll(anyview), env: env)
    }
    
    init(scroll: waterui_scroll, env: Environment) {
        switch scroll.axis{
        case WATERUI_AXIS_VERTICAL:
            self.axis=[.vertical]
        case WATERUI_AXIS_HORIZONTAL:
            self.axis=[.horizontal]
            
        default:
            self.axis=[.vertical,.horizontal]
        }
        
        self.content=AnyView(anyview: scroll.content, env: env)
    }
}
