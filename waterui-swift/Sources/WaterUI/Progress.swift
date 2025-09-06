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
        case WATERUI_STYLE_PROGRESS_LINEAR:
            self = .Linear
        case WATERUI_STYLE_PROGRESS_CIRCULAR:
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

struct Progress:View,Component{
    static var id=waterui_view_progress_id()
    var label:AnyView
    @StateObject var value:ComputedDouble
    var style:ProgressStyle
    
    init(progress: waterui_progress,env:Environment) {
        label = AnyView(anyview: progress.label, env: env)
        style=ProgressStyle(progress.style)
        _value = StateObject(wrappedValue:ComputedDouble(inner: progress.value))
    }
    
    init(anyview:OpaquePointer,env:Environment) {
        self.init(progress: waterui_view_force_as_progress(anyview),env:env)
    }
    
    var body:some View{
        VStack{
            if value.value.isNaN{
                SwiftUI.ProgressView{
                    label
                }
            }else{
                SwiftUI.ProgressView(value: value.value, label: {
                    label
                })
                .animation(.default, value: value.value)
                
            }
        }.progressViewStyle(style)
       
        
    }
}


