//
//  Divider.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/20/24.
//

import CWaterUI
import SwiftUI

struct Divider: View, Component {
    static var id = waterui_divider_id()

    var body: some View {
        SwiftUI.Divider()
    }

    init(anyview: OpaquePointer, env: Environment) {

    }
}
