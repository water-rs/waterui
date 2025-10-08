//
//  Animation.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/6/25.
//

import SwiftUI
import CWaterUI

@MainActor
func useAnimation(_ metadata: WuiWatcherMetadata, _ body: @escaping () -> Void) {
    if let animation = metadata.getAnimation() {
        withAnimation(animation) {
            body()
        }
    } else {
        body()  // No animation
    }
}

extension SwiftUI.Animation{
    init?(_ animation: CWaterUI.WuiAnimation) {
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
