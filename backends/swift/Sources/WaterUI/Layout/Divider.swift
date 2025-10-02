//
//  Divider.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/20/24.
//

import CWaterUI
import SwiftUI

struct WuiDivider: View, WuiComponent {
    static var id: WuiTypeId {
        waterui_divider_id()
    }

    var body: some View {
        SwiftUI.Divider()
    }

    init(anyview: OpaquePointer, env: WuiEnvironment) {

    }
}
