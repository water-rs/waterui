//
//  EmptyView.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/20/24.
//
import SwiftUI
import CWaterUI
struct EmptyView:View,Component{
    static var id=waterui_view_empty_id()
    init(anyview: OpaquePointer, env: Environment) {
        
    }
    var body: some View {
        SwiftUI.EmptyView()
    }
}
