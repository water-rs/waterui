//
//  util.swift
//  tour
//
//  Created by Lexo Liu on 30/11/2023.
//

import Foundation
import SwiftUI
extension WaterUIBuf{
    func to_data() -> Data{
        let pointer=UnsafeBufferPointer(start: self.head, count: Int(self.len))
        return Data(buffer: pointer)
    }
    
    func to_string() -> String{
        let data=self.to_data()
        return String(data: data, encoding:.utf8)!
    }
}


extension WaterUIViews{
    func to_views() -> [WaterUIViewObject]{
        let pointer=UnsafeBufferPointer(start: self.head, count: Int(self.len))
        return Array(pointer)
    }
}
