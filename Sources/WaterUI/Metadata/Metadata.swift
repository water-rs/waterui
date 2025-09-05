//
//  Metadata.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//

import CWaterUI
import SwiftUI

struct WithEnv:View,Component{
    static var id=waterui_metadata_env_id()
    var content:WaterUI.AnyView
    
    var body: some View{
        content
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(withEnv: waterui_metadata_force_as_env(anyview), env: env)
    }
    
    init(withEnv: waterui_metadata_____waterui_env, env: Environment) {
        self.content=WaterUI.AnyView(anyview: withEnv.content, env: WaterUI.Environment(withEnv.value))
    }
}



struct BackgroundColor:View,Component{
    static var id=waterui_metadata_background_color_id()
    @ObservedObject var color:ComputedColor
    var content:WaterUI.AnyView
    
    var body: some View{
        content.background(color.value)
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(color: waterui_metadata_force_as_background_color(anyview), env: env)
    }
    
    init(color: waterui_metadata_waterui_background_color, env: Environment) {
        self.color = ComputedColor(inner: color.value.color)
        self.content=WaterUI.AnyView(anyview: color.content, env: env)
    }
}


extension SwiftUI.Color{
    init(_ color:waterui_color){
        self.init(hue: color.hue, saturation: color.saturation, brightness: color.brightness, opacity: color.opacity)
    }
}

@MainActor
class ComputedColor:ObservableObject{
    private var inner: OpaquePointer
    private var watcher:WatcherGuard!
    var value:SwiftUI.Color{
        self.compute()
    }
    
    init(inner: OpaquePointer) {
        self.inner = inner
        self.watcher=self.watch{new,animation in
            self.objectWillChange.send()
        }
    }
    
    func compute() -> SwiftUI.Color{
        SwiftUI.Color(waterui_read_computed_color(self.inner))
    }
    
    func watch(_ f:@escaping (SwiftUI.Color,Animation?)->()) -> WatcherGuard{
        let g = waterui_watch_computed_color(self.inner, waterui_watcher_waterui_color({value,animation in
            f(value,animation)
        }))
        return WatcherGuard(g!)
    }

    deinit {
        let this=self
        Task{@MainActor in
            waterui_drop_computed_color(this.inner)
        }
        
    }
}

extension waterui_watcher_waterui_color {
    init(_ f: @escaping (SwiftUI.Color,Animation?) -> Void) {
        class Wrapper {
            var inner: (SwiftUI.Color,Animation?) -> Void
            init(_ inner: @escaping (SwiftUI.Color,Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value, metadata in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(SwiftUI.Color(value),Animation(waterui_get_animation(metadata)))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()

        })
    }
}
