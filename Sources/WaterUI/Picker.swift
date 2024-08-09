//
//  Picker.swift
//
//
//  Created by Lexo Liu on 8/2/24.
//

import Foundation
import CWaterUI
import SwiftUI

struct Picker:View {
    @State var swiftSelection:Int32 = -1
    var label:AnyView
    @State var swiftItems:[(Text,Int32)]
    var body: some View {
        SwiftUI.Picker(selection: $swiftSelection){
            
        }label: {label}
    }
}
