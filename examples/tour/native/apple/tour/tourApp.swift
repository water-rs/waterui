//
//  tourApp.swift
//  tour
//
//  Created by Lexo Liu on 29/11/2023.
//

import SwiftUI
import Foundation

@main
struct tourApp: App {
    var body: some Scene {
        WindowGroup(id:"waterui-0") {
            WaterView(waterui_main())
        }
    }
}

struct WaterView:View{
    var view:WaterUIViewObject
    @StateObject var reloadTrigger = ReloadViewTrigger()

    var body:some View{
        AnyView(build_view(view:view, trigger: reloadTrigger))
    }
    
    init(_ view: WaterUIViewObject) {
        self.view = view
    }
   
}

class ReloadViewTrigger:ObservableObject{
    func reloadView() {
        self.objectWillChange.send()
    }
}

func build_view(view: WaterUIViewObject, trigger:ReloadViewTrigger) -> any View{
    var result:(any View)?=nil
    
    if waterui_view_to_empty(view) == 0{
        return EmptyView()
    }

    var text=WaterUIText()
    withUnsafeMutablePointer(to: &text){text in
        if waterui_view_to_text(view, text) == 0{
            result = Text(text.pointee.buf.to_string())
        }
    }


    var button=WaterUIButton()
    withUnsafeMutablePointer(to: &button){button in
        if waterui_view_to_button(view, button) == 0{
            let eventObject=button.pointee.action

            result = Button(button.pointee.label.to_string()){
                waterui_call_event_object(eventObject)
            }
        }
    }

    var stack=WaterUIStack()
    
    withUnsafeMutablePointer(to: &stack){stack in
        if waterui_view_to_stack(view, stack) == 0{
            result = VStack{
                let contents=stack.pointee.contents.to_views().map({IdentifiableViewObject($0)})
                ForEach(contents) {content in
                    WaterView(content.view)
                }
            }
        }
    }
    
    
    var tapGesture=WaterUITapGesture()


    withUnsafeMutablePointer(to: &tapGesture){tapGesture in
        if waterui_view_to_tap_gesture(view, tapGesture) == 0{
            let eventObject=tapGesture.pointee.event
            result=WaterView(tapGesture.pointee.view).onTapGesture{
                waterui_call_event_object(eventObject)
            }
        }
    }
    
    if let result=result{
        return result
    }
    

    let triggerPointer=UnsafeMutablePointer<ReloadViewTrigger>.allocate(capacity: 1)
    triggerPointer.initialize(to: trigger)

    let builder=WaterUISubscriberBuilderObject(state: triggerPointer,subscriber:subscriber_builder)
    waterui_add_subscriber(view, builder)
    

    return WaterView(waterui_call_view(view))
    

}

func subscriber_builder(trigger:UnsafeRawPointer!) -> WaterUISubscriberObject{
    let trigger = unsafeBitCast(trigger,to:UnsafePointer<ReloadViewTrigger>.self)
    let new_trigger=trigger.pointee
    let pointer = UnsafeMutablePointer<ReloadViewTrigger>.allocate(capacity: 1)
    pointer.initialize(to: new_trigger)
    return WaterUISubscriberObject(state: pointer,subscriber: subscriber)
}

func subscriber(trigger:UnsafeRawPointer!){
    let trigger = unsafeBitCast(trigger,to:UnsafePointer<ReloadViewTrigger>.self)
    trigger.pointee.reloadView()
}

struct IdentifiableViewObject:Identifiable{
    var id=UUID()
    var view:WaterUIViewObject
    
    init(_ view:WaterUIViewObject){
        self.view=view
    }
}


@_cdecl("waterui_create_window")
func create_window(view:WaterUIViewObject){
    
    let window=NSWindow(contentRect: NSRect(x: 0, y: 0, width: 0, height: 0), styleMask: [.titled, .closable,  .resizable, .fullSizeContentView,.miniaturizable], backing: .buffered, defer: false)
    
    
    window.contentView=NSHostingView(rootView:WaterView(view))
    window.makeKeyAndOrderFront(nil)
    
}

class AppWindow: NSWindow {
    init<V:View>(_ view:V) {
        super.init(contentRect: NSRect(x: 0, y: 0, width: 480, height: 300), styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        makeKeyAndOrderFront(nil)
        isReleasedWhenClosed = false
        styleMask.insert(NSWindow.StyleMask.fullSizeContentView)
        title = "title placeholder"
        contentView = NSHostingView(rootView: view)
    }
}

@_cdecl("waterui_close_window")
func close_window(id:size_t){

}


@_cdecl("waterui_window_closeable")
func window_closeable(){
    
}
