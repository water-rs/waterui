//
//  Padding.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//

import CWaterUI
import SwiftUI

/*
struct Padding:View,Component{
    static var id=waterui_metadata_padding_id()
    var content:WaterUI.AnyView
    var edge:waterui_edge
    
    var body: some View{
        content.padding([.top],edge.top.checkNaN()).padding([.leading],edge.left.checkNaN()).padding([.trailing],edge.right.checkNaN()).padding([.bottom],edge.bottom.checkNaN())
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(metadata: waterui_metadata_force_as_padding(anyview), env: env)
    }
    
    init(metadata: waterui_metadata_waterui_edge, env: Environment) {
        self.content=WaterUI.AnyView(anyview:metadata.content , env: env)
        self.edge=metadata.value
    }
}
*/
