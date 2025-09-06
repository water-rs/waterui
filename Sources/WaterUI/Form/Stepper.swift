//
//  Stepper.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

struct Stepper: View,Component {
    static var id=waterui_view_stepper_id()
    @ObservedObject var step: ComputedInt
    @ObservedObject var value: BindingInt
    init(stepper: waterui_stepper){
        step = ComputedInt(inner: stepper.step)
        value = BindingInt(inner: stepper.value)
    }

    init(anyview: OpaquePointer,env:Environment) {
        self.init(stepper: waterui_view_force_as_stepper(anyview))
    }

    var body: some View {
        SwiftUI.Stepper(value:value.value, step:1, label: {
            SwiftUI.Text(value.value.wrappedValue.description)
        })
    }
}
