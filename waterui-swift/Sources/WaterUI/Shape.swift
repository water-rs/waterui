//
//  Shape.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/20/24.
//
import SwiftUI
import CWaterUI
struct Rectangle:View,Component{
    static var id=waterui_view_rectangle_id()
    var body: some View {
        SwiftUI.Rectangle()
    }
    
    init(anyview: OpaquePointer,env:WaterUI.Environment){
        
    }

}

struct RoundedRectangle:View,Component{
    static var id=waterui_view_rounded_rectangle_id()
    @ObservedObject var radius:ComputedDouble
    var body: some View {
        SwiftUI.RoundedRectangle(cornerRadius: radius.value)
    }
    
    init(anyview: OpaquePointer,env:WaterUI.Environment){
        let view=waterui_view_force_as_rounded_rectangle(anyview)
        self.radius=ComputedDouble(inner: view.radius)
    }

}

struct Circle:View,Component{
    static var id=waterui_view_circle_id()
    
    var body: some View {
        SwiftUI.Circle()
    }
    
    init(anyview: OpaquePointer,env:WaterUI.Environment){
       
    
    }

}
