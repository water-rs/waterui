//
//  Color.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//
import SwiftUI
import CWaterUI
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

struct ForegroundColor:View,Component{
    static var id=waterui_metadata_foreground_color_id()
    @ObservedObject var color:ComputedColor
    var content:WaterUI.AnyView
    
    var body: some View{
        content.foregroundStyle(color.value)
    }
    
    init(anyview: OpaquePointer, env: Environment) {
        self.init(color: waterui_metadata_force_as_foreground_color(anyview), env: env)
    }
    
    init(color: waterui_metadata_waterui_foreground_color, env: Environment) {
        self.color = ComputedColor(inner: color.value.color)
        self.content=WaterUI.AnyView(anyview: color.content, env: env)
    }
}



extension SwiftUI.Color{
    init(_ color:waterui_color){
        let color=CGColor(colorSpace: color.space.toCGColorSpace(), components: [color.red/255.0,color.green/255.0,color.blue/255.0,color.opacity])!
        self.init(color)
    }
}

extension waterui_color{
    init(_ color:SwiftUI.Color){
        #if canImport(AppKit)
        let color=NSColor(color)
        #elseif canImport(UIKit)
        let color=UIColor(color)
        #endif
        
        let cgColor=color.cgColor
        
        let red=cgColor.components![0]
        let green=cgColor.components![1]
        let blue=cgColor.components![2]

        self.init(space: CWaterUI.ColorSpace(cgColor.colorSpace!), red: red, green: green, blue: blue, opacity: cgColor.alpha)
        
    }
}

extension CWaterUI.ColorSpace{
    init(_ colorSpace:CGColorSpace){
        switch colorSpace.name!{
            case CGColorSpace.displayP3:
                self=COLOR_SPACE_P3
            case CGColorSpace.sRGB:
                self=COLOR_SPACE_S_RGB
            default:
                self=COLOR_SPACE_S_RGB
        }
    }

    func toCGColorSpace() -> CGColorSpace{
        switch self{
        case COLOR_SPACE_P3:
            return CGColorSpace(name: CGColorSpace.displayP3)!
        default:
            return CGColorSpace(name: CGColorSpace.sRGB)!
        }
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
            useAnimation(animation: animation, publisher: self.objectWillChange)
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
        weak var this=self
        Task{@MainActor in
            if let this=this{
                waterui_drop_computed_color(this.inner)
            }
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


@MainActor
class BindingColor:ObservableObject{
    private var inner: OpaquePointer
    private var watcher:WatcherGuard!

    var value:Binding<SwiftUI.Color>{
        Binding(get: {
            self.compute()
        }, set: {new in
            self.set(new)
        })
    }
    init(inner: OpaquePointer) {
        self.inner = inner

        self.watcher=self.watch{new,animation in
            useAnimation(animation: animation, publisher: self.objectWillChange)
        }
    }
    
    
    func compute()  -> SwiftUI.Color{
        .init(waterui_read_binding_color(self.inner))
    }
    
    func watch(_ f:@escaping (SwiftUI.Color,Animation?)->()) -> WatcherGuard{
        let g = waterui_watch_binding_color(self.inner, waterui_watcher_waterui_color({value,animation in
            f(value,animation)
        }))
        return WatcherGuard(g!)
    }
    
    func set(_ value:SwiftUI.Color){
        waterui_set_binding_color(self.inner, .init(value))

    }



    deinit {
    
        weak var this=self
        Task{@MainActor in
            if let this=this{
                waterui_drop_binding_bool(this.inner)
            }
        }
        
        
    }
}
