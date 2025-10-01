//
//  Stepper.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

struct WuiStepper: View,WuiComponent {
    static var id = waterui_stepper_id()
    @ObservedObject var step: ComputedInt
    @ObservedObject var value: BindingInt
    init(stepper: CWaterUI.WuiStepper, env: WuiEnvironment){
        step = ComputedInt(inner: stepper.step)
        value = BindingInt(inner: stepper.value)
    }

    init(anyview: OpaquePointer, env:WuiEnvironment) {
        self.init(stepper: waterui_force_as_stepper(anyview), env: env)
    }

    var body: some View {
        SwiftUI.Stepper(value:value.value, step:1, label: {
            SwiftUI.Text(value.value.wrappedValue.description)
        })
    }
}
