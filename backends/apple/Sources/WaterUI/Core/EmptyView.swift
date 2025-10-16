//
//  EmptyView.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/20/24.
//
import SwiftUI
import CWaterUI
struct WuiEmptyView:View,WuiComponent{
    static var id:WuiTypeId{
        waterui_empty_id()
    }
    
    init(anyview: OpaquePointer, env: WuiEnvironment) {
        
    }
    var body: some View {
        EmptyView()
    }
}
