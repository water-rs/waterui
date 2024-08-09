//
//  Progress.swift
//
//
//  Created by Lexo Liu on 8/2/24.
//

import Foundation
import CWaterUI
import SwiftUI
enum ProgressStyle{
    case Default
    case Circular
    case Linear
}

extension ProgressStyle{
    init(_ style:waterui_style_progress){
        switch style{
            case waterui_style_progress_LINEAR:
            self = .Linear
        case waterui_style_progress_CIRCULAR:
            self = .Circular
        default:
            self = .Default
        }
    }
}

extension ProgressStyle:SwiftUI.ProgressViewStyle{
    func makeBody(configuration: Configuration) -> some View {
        let view = SwiftUI.ProgressView(value: configuration.fractionCompleted, label: {configuration.label},currentValueLabel: {configuration.currentValueLabel})
        switch self{
            case .Circular:
                view.progressViewStyle(.circular)
            case .Linear:
                view.progressViewStyle(.linear)
            default:
                view.progressViewStyle(.automatic)
        }
    }
}

struct Progress:View{
    var label:AnyView
    @ObservedObject var value:ComputedInt
    var style:ProgressStyle
    
    init(progress: waterui_progress,env:Environment) {
        label = AnyView(view: progress.label, env: env)
        style=ProgressStyle(progress.style)
        value = ComputedInt(inner: progress.value)
    }
    
    init(view:OpaquePointer,env:Environment) {
        self.init(progress: waterui_view_force_as_progress(view),env:env)
    }
    
    var body:some View{
        VStack{
            if value.value > 0{
                SwiftUI.ProgressView(value: 1.0/Double(Int(Int32.max)/value.value) , label: {
                    label
                })
            }
            else{
                SwiftUI.ProgressView{
                    label
                }
                
            }
        }.progressViewStyle(style)
       
        
    }
}


