//
//  Animation.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/6/25.
//

extension SwiftUI.Animation{
    init?(_ animation: WuiAnimation){
        switch animation{
            case WuiAnimation_Default:
                    self = .default
            case WuiAnimation_None:
                return nil
            default:
                return nil
        }
    }
}
