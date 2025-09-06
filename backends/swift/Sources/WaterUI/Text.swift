//
//  Text.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import SwiftUI



@MainActor
struct Text: View,Component {
    static var id = waterui_text_id()
    @State var content: ComputedStr
    
    init(text: WuiText) {
        
        self.content = ComputedStr(inner: text.content)
    }
    
    init(anyview: OpaquePointer,env: Environment) {
        self.init(text: waterui_force_as_text(anyview))
    }
    
    func toText() -> SwiftUI.Text{
        return SwiftUI.Text(content.value)
    }

    var body: some View {
        let lines=content.value.split(separator: "\n").map{line in
            (line,UUID())
        }
        LazyVStack{
            ForEach(lines,id:\.1){(line,_) in
                SwiftUI.Text(line)
            }
        }
        
    }
    
}


struct Label:View,Component{
    static var id = waterui_label_id()
    var label:WuiStr
    init(label:WuiStr){
        self.label=label
    }
    
    init(anyview: OpaquePointer,env: Environment) {
        self.init(label: WuiStr(waterui_force_as_label(anyview)))
    }
    
    var body: some View{
        SwiftUI.Text(label.toString())
    }
    
}
