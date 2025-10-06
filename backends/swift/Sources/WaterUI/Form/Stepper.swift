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
    static var id:WuiTypeId{
        waterui_stepper_id()
    }
    @ObservedObject var step: WuiComputedInt
    @ObservedObject var value: WuiBindingInt
    
    init(stepper: CWaterUI.WuiStepper, env: WuiEnvironment){
        self.step = WuiComputedInt(inner: stepper.step)
        self.value = WuiBindingInt(inner: stepper.value)
    }

    init(anyview: OpaquePointer, env:WuiEnvironment) {
        self.init(stepper: waterui_force_as_stepper(anyview), env: env)
    }

    var body: some View {
        SwiftUI.Stepper(value: value.value, in: 0...100, step: step.value) {
            SwiftUI.Text(value.value.wrappedValue.description)
        }
    }
}