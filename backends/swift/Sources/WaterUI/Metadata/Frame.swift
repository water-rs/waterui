//
//  Frame.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//

import CWaterUI
import SwiftUI

struct Frame:View,Component{
    static var id=waterui_metadata_frame_id()
    var content:WaterUI.AnyView
    @ObservedObject var frame:ComputedFrame

    var body: some View{
        let frame=frame.value
        if frame.isFixed(){
            content.frame(width:frame.width.checkNaN(),height:frame.height.checkNaN(),alignment: SwiftUI.Alignment(frame.alignment))
        }
        else{
            content.frame(minWidth:frame.min_width.checkNaN(), idealWidth: frame.width.checkNaN(), maxWidth: frame.max_width.checkNaN(), minHeight:frame.min_height.checkNaN(), idealHeight:frame.height.checkNaN(), maxHeight: frame.max_height.checkNaN(),alignment: SwiftUI.Alignment(frame.alignment))
        }

    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(metadata: waterui_metadata_force_as_frame(anyview), env: env)
    }
    
    init(metadata: waterui_metadata_____waterui_computed_frame, env: Environment) {
        self.content=WaterUI.AnyView(anyview:metadata.content , env: env)
        self.frame=ComputedFrame(inner: metadata.value)
    }
}

extension Double{
    func checkNaN()-> CGFloat?{
        if self>0{
            return self
        }
        else{
            return nil
        }
    }
}

extension waterui_frame{
    func isFixed() -> Bool{
        self.min_width.isNaN && self.max_width.isNaN && self.min_height.isNaN && self.max_height.isNaN
    }
}

@MainActor
class ComputedFrame:ObservableObject{
    private var inner: OpaquePointer
    private var watcher:WatcherGuard!
    var value:waterui_frame{
        self.compute()
    }
    
    init(inner: OpaquePointer) {
        self.inner = inner
        self.watcher=self.watch{new,animation in
            self.objectWillChange.send()
        }
    }
    
    func compute() -> waterui_frame{
        waterui_read_computed_frame(self.inner)
    }
    
    func watch(_ f:@escaping (waterui_frame,Animation?)->()) -> WatcherGuard{
        let g = waterui_watch_computed_frame(self.inner, waterui_watcher_waterui_frame({value,animation in
            f(value,animation)
        }))
        return WatcherGuard(g!)
    }

    deinit {
    
        weak var this=self
        Task{@MainActor in
            if let this=this{
                waterui_drop_computed_frame(this.inner)
            }
        }
        
        
        
    }
}

extension waterui_watcher_waterui_frame {
    init(_ f: @escaping (waterui_frame,Animation?) -> Void) {
        class Wrapper {
            var inner: (waterui_frame,Animation?) -> Void
            init(_ inner: @escaping (waterui_frame,Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value, metadata in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value,Animation(waterui_get_animation(metadata)))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()
        })
    }
}

extension SwiftUI.Alignment{
    init (_ alignment: CWaterUI.Alignment){
        switch alignment{
        case ALIGNMENT_LEADING:
            self = .leading
        case ALIGNMENT_CENTER:
            self = .center
        case ALIGNMENT_TRAILING:
            self = .trailing
        default:
            self = .center
        }
    }
}
