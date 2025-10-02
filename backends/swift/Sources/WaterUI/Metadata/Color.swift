//
//  Color.swift
//  waterui-swift
//
//  Created by Lexo Liu on 10/21/24.
//
import SwiftUI
import CWaterUI


extension WuiColor{
    func toSwiftUIColor() -> SwiftUI.Color{
        Color(color_space.toSwiftUIColorSpace(),red: Double(red), green: Double(green), blue: Double(blue), opacity: Double(opacity))
    }
    
    init(_ color: SwiftUI.Color){
        fatalError()
        //let resolvedHDR=color.res
    }

}


extension WuiColorSpace{
    func toSwiftUIColorSpace() -> Color.RGBColorSpace{
        switch self{
        case WuiColorSpace_P3:
            return .displayP3
        default:
            return .sRGB
        }
    }
}

@MainActor
class BindingColor: ObservableObject {
    private var inner: OpaquePointer
    private var watcher: WatcherGuard!

    var value: Binding<Color> {
        Binding(
            get: {
                self.get()
            },
            set: { new in
                self.set(new)
            })
    }
    init(inner: OpaquePointer) {
        self.inner = inner

        self.watcher = self.watch { new, animation in
            useAnimation(animation: animation, publisher: self.objectWillChange)
        }
    }

    func get() -> Color {
        waterui_binding_read_color(self.inner).toSwiftUIColor()
    }

    func watch(_ f: @escaping (Color, Animation?) -> Void) -> WatcherGuard {
        let g = waterui_binding_watch_color(
            self.inner,
            WuiWatcher_WuiColor({ value, animation in
                f(value, animation)
            }))
        return WatcherGuard(g!)
    }

    func set(_ value: Color) {
        waterui_binding_set_color(self.inner, .init(value))

    }

    @MainActor  deinit {
        waterui_drop_binding_double(inner)

    }
}


extension WuiWatcher_WuiColor {
    init(_ f: @escaping (Color,Animation?) -> Void) {
        class Wrapper {
            var inner: (Color,Animation?) -> Void
            init(_ inner: @escaping (Color,Animation?) -> Void) {
                self.inner = inner
            }
        }

        let data = UnsafeMutableRawPointer(Unmanaged.passRetained(Wrapper(f)).toOpaque())

        self.init(data: data, call: { data, value,metadata in
            let f = Unmanaged<Wrapper>.fromOpaque(data!).takeUnretainedValue().inner
            f(value.toSwiftUIColor(),Animation(waterui_get_animation(metadata)))

        }, drop: { data in
            _ = Unmanaged<Wrapper>.fromOpaque(data!).takeRetainedValue()
        })
    }
}


