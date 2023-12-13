//
//  water.swift
//  tour
//
//  Created by Lexo Liu on 12/12/2023.
//

import Foundation
import SwiftUI

func visitEmpty(_ view:WaterUIViewObject) -> EmptyView?{
    var result:EmptyView?=nil
    if waterui_view_to_empty(view) == 0{
        result = EmptyView()
    }
    return result
}


func visitText(_ view:WaterUIViewObject) -> Text?{
    var result:Text? = nil
    var text=WaterUIText()
    withUnsafeMutablePointer(to: &text){text in
        if waterui_view_to_text(view, text) == 0{
            result = Text(text.pointee.buf.to_string())
        }
    }
    return result
}


func visitButton(_ view:WaterUIViewObject) -> Button<WaterView>?{
    var result:Button<WaterView>? = nil
    var button = WaterUIButton()
    withUnsafeMutablePointer(to: &button){button in
        if waterui_view_to_button(view, button) == 0{
            let eventObject=button.pointee.action
            let label=button.pointee.label
            result = Button(){
                waterui_call_event_object(eventObject)
            }label:{
                WaterView(label)
            }
        }
    }
    return result
}



func visitStack(_ view:WaterUIViewObject) -> AnyView?{
    var result:(any View)? = nil
    var stack=WaterUIStack()
    
    withUnsafeMutablePointer(to: &stack){stack in
        if waterui_view_to_stack(view, stack) == 0{
            let contents=stack.pointee.contents.to_views().map({IdentifiableViewObject($0)})
            let view=ForEach(contents) {content in
                WaterView(content.view)
            }
            switch stack.pointee.mode{
            case WaterUIStackMode_Vertical:
                result = VStack{view}
            case WaterUIStackMode_Horizonal:
                result = HStack{view}
            default:
                abort()
            }
            
        }
    }
    
    return result.map({AnyView.init($0)})
}


func visitTapGesture(_ view:WaterUIViewObject) -> AnyView?{
    var result:(any View)? = nil
    
    var gesture=WaterUITapGesture()
    
    
    withUnsafeMutablePointer(to: &gesture){gesture in
        if waterui_view_to_tap_gesture(view, gesture) == 0{
            let eventObject=gesture.pointee.event
            result = WaterView(gesture.pointee.view).onTapGesture{
                waterui_call_event_object(eventObject)
            }
        }
    }
    
    return result.map({AnyView.init($0)})
}


func visitFrameModifier(_ view:WaterUIViewObject) -> AnyView?{
    var result:(any View)? = nil
    
    var modifier=WaterUIFrameModifier()
    
    withUnsafeMutablePointer(to: &modifier){modifier in
        if waterui_view_to_frame_modifier(view, modifier) == 0{
            let frame=modifier.pointee.frame
            result = WaterView(modifier.pointee.view).frame(minWidth: frame.min_width.toCGFloat(),
                                                            idealWidth: frame.width.toCGFloat(),maxWidth: frame.max_width.toCGFloat(),
                                                            minHeight:  frame.min_height.toCGFloat(),
                                                            idealHeight:  frame.height.toCGFloat(),
                                                            maxHeight:  frame.max_height.toCGFloat())
        }
    }
    
    return result.map({AnyView.init($0)})
}

struct IdentifiableItem<T>:Identifiable{
    var id = UUID()
    var item:T
    
    init (_ item:T){
        self.item=item
    }
}



func visitMenu(_ view:WaterUIViewObject) -> Menu<WaterView, ForEach<[IdentifiableItem<WaterUIAction>], UUID, Button<Text>>>?{
    var menu=WaterUIMenu()
    var result:Menu<WaterView, ForEach<[IdentifiableItem<WaterUIAction>], UUID, Button<Text>>>? = nil
    withUnsafeMutablePointer(to: &menu){menu in
        if waterui_view_to_menu(view,menu) == 0{
            let actions = menu.pointee.actions.to_array().map(IdentifiableItem.init)
            result = Menu{
                ForEach(actions,id:\.id){action in
                    Button(action.item.label.to_string()){
                        waterui_call_event_object(action.item.action)
                    }
                }
            }label:{
                WaterView(menu.pointee.label)
            }
        }
  
    }
    return result
}


func visitTextField(_ view:WaterUIViewObject) -> TextField<Text>?{
    var textField = WaterUITextField()
    var result:TextField<Text>?=nil
    withUnsafeMutablePointer(to: &textField){ textField in
        if waterui_view_to_text_field(view, textField) == 0{
            let binding=textField.pointee.value
            result = TextField(textField.pointee.label.to_string(), text: Binding(get: {waterui_get_string_binding(binding).to_string()}, set: {
                waterui_set_string_binding(binding, WaterUIBuf($0))
            }), prompt: Text(textField.pointee.prompt.to_string()))
        }
    }
    return result
}
