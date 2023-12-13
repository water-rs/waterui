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
    
    init(_ string:String){
        self.init()
        let pointer=UnsafeMutablePointer<UInt8>.allocate(capacity: string.count)
        string.data(using: .utf8)!.copyBytes(to: pointer,count:string.count)
        head = pointer
        len = UInt(string.count)
        capacity = UInt(string.count)
    }
}


extension WaterUIViews{
    func to_views() -> [WaterUIViewObject]{
        let pointer=UnsafeBufferPointer(start: self.head, count: Int(self.len))
        return Array(pointer)
    }
}

extension WaterUIActions{
    func to_array() -> [WaterUIAction]{
        let pointer=UnsafeBufferPointer(start: self.head, count: Int(self.len))
        return Array(pointer)
    }
}

extension WaterUISize{
    func toCGFloat() -> CGFloat?{
        switch self.tag{
        case WaterUISize_Default:
            return nil
        case WaterUISize_Size:
            return CGFloat(self.size)
        default:
            abort()
        }
    }
}


extension WaterUIAlignment{
    func toAlignment() -> Alignment?{
        switch self{
        case WaterUIAlignment_Default:
            return nil
        case WaterUIAlignment_Leading:
            return .leading
        case WaterUIAlignment_Trailing:
            return .trailing
        case WaterUIAlignment_Center:
            return .center
            
        default:
            abort()
        }
    }
}
