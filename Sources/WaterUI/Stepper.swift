//
//  Stepper.swift
//
//
//  Created by Lexo Liu on 8/1/24.
//

import CWaterUI
import Foundation
import SwiftUI

struct Stepper: View {
    @ObservedObject var step: ComputedInt
    @ObservedObject var value: BindingInt
    init(stepper: waterui_stepper){
        step = ComputedInt(inner: stepper.step)
        value = BindingInt(inner: stepper.value)
    }

    init(view: OpaquePointer) {
        self.init(stepper: waterui_view_force_as_stepper(view))
    }

    var body: some View {
        SwiftUI.Stepper(value:$value.value, step:1, label: {
            SwiftUI.Text(value.value.description)
        })
    }
}
