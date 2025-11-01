//
//  Stepper.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

struct WuiStepper: View, WuiComponent {
    static let id: String = decodeViewIdentifier(waterui_stepper_id())
    @State var step: WuiComputed<Int32>
    @State var value: WuiBinding<Int32>
    
    init(stepper: CWaterUI.WuiStepper, env: WuiEnvironment){
        self.step = WuiComputed(stepper.step)
        self.value = WuiBinding(stepper.value)
    }

    init(anyview: OpaquePointer, env:WuiEnvironment) {
        self.init(stepper: waterui_force_as_stepper(anyview), env: env)
    }

    var body: some View {
        SwiftUI.Stepper(value: $value.value, in: 0...100, step: Int(step.value)) {
            SwiftUI.Text(value.value.description)
        }
    }
}
