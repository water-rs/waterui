//
//  Image.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/20/24.
//


import CWaterUI
import SwiftUI

/*
struct Image:View,Component{
    static var id=image_id()
    @ObservedObject var data:ComputedData
    
    var body: some View{
        #if canImport(UIKit)
        SwiftUI.Image(uiImage: UIImage(data: data.value)!)
        #elseif canImport(AppKit)
        SwiftUI.Image(nsImage: NSImage(data: data.value)!)
        #endif
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(image: force_as_image(anyview), env: env)
    }
    
    init(image: waterui_image, env: Environment) {
        self.data=ComputedData(inner: image.data)
    }
}


extension Data{
    init(_ data:waterui_data){
        self.init(buffer: UnsafeBufferPointer<UInt8>(start: data.head, count: Int(data.len)))
    }
}
*/
