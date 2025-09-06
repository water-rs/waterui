//
//  Stack.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import SwiftUI

enum StackMode{
    case Default
    case Vertical
    case Horizontal
    case Layered
}

extension StackMode{
    init(_ mode:waterui_stack_mode){
        switch mode {
        case STACK_MODE_HORIZONAL:
            self = .Horizontal
        case STACK_MODE_LAYERED:
            self = .Layered
        default:
            self = .Default
        }
    }
}

extension waterui_array_____waterui_anyview{
    func toArray() -> Array<OpaquePointer?>{
        
        let array =  Array(UnsafeBufferPointer<OpaquePointer?>(start: self.head, count: Int(self.len)))
        waterui_free_array(self.head, self.len)
        return array
    }
}

@MainActor
struct Stack: View,Component {
    static var id=waterui_view_stack_id()
    private var contents:[AnyView]
    private var mode:StackMode
    init(stack: waterui_stack, env: Environment) {
        let array = stack.contents.toArray()

        contents = array.map { view in
            AnyView(anyview: view!, env: env)
        }
        mode = StackMode(stack.mode)
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(stack: waterui_view_force_as_stack(anyview), env: env)
    }

    var body: some View {
        let each=SwiftUI.ForEach(contents, id: \.id, content: {content in
            content
        })
       
        VStack{
            switch mode{
            case .Horizontal:
                HStack{each}
            case .Layered:
                ZStack{each}
                    
            default:
                VStack{each}
                
                
            }
        }.padding()
    }
}
