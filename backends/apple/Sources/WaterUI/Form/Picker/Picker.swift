//
//  Picker.swift
//
//
//  Created by Lexo Liu on 8/2/24.
//

import Foundation
import CWaterUI
import SwiftUI
/*
struct Picker:View,WuiComponent {
    static var id=waterui_picker_id()
    @ObservedObject var selection:BindingInt
    @ObservedObject var items:ComputedPickerItems
    init(picker: waterui_picker, env: Environment) {
        self.selection=BindingInt(inner: picker.selection)
        self.items=ComputedPickerItems(inner: picker.items)
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(picker: force_as_picker(anyview), env: env)
    }
    var body: some View {
        SwiftUI.Text("\(selection.value.wrappedValue)")
        SwiftUI.Picker("Picker",selection: selection.value){
            
            ForEach(items.value, id: \.tag, content: {item in
                item.label.tag(item.tag)
            })
        }
    }
}
 struct PickerItem {
    var label: WaterUI.Text
    var tag: Int32
}


@MainActor
class ComputedPickerItems:ObservableObject{
    private var inner: OpaquePointer
    private var watcher:WatcherGuard!
    var value:Array<PickerItem>{
        self.compute()
    }
    
    init(inner: OpaquePointer) {
        self.inner = inner
        self.watcher=self.watch{new,animation in
            self.objectWillChange.send()
        }
    }
    
    func compute() -> Array<PickerItem>{
        waterui_read_computed_picker_items(self.inner).toArray()
    }
    
    func watch(_ f:@escaping (Array<PickerItem>,Animation?)->()) -> WatcherGuard{
        let g = waterui_watch_computed_picker_items(self.inner, waterui_watcher_waterui_array_waterui_picker_item({value,animation in
            f(value,animation)
        }))
        return WatcherGuard(g!)
    }

    deinit {
   
        weak var this=self
        Task{@MainActor in
            if let this=this{
                waterui_drop_computed_data(this.inner)
            }
        }
        
        
    }
}

@MainActor
extension waterui_array_waterui_picker_item{
    func toArray() -> Array<PickerItem>{
        let array = Array(UnsafeBufferPointer<waterui_picker_item>(start: self.head, count: Int(self.len)))
        return array.map{item in
            PickerItem(label: Text(text:  item.label), tag: Int32(item.tag))
        }
    }
}

@MainActor
extension waterui_watcher_waterui_array_waterui_picker_item {
    init(_ f: @escaping (Array<PickerItem>,Animation?) -> Void) {
        class Wrapper {
            var inner: (Array<PickerItem>,Animation?) -> Void
            init(_ inner: @escaping (Array<PickerItem>,Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value, metadata in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value.toArray(),Animation(waterui_get_animation(metadata)))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}
 */
