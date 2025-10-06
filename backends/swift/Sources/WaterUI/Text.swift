//
//  Text.swift
//
//
//  Created by Lexo Liu on 5/14/24.
//

import CWaterUI
import SwiftUI
import Foundation

// MARK: - Text View

struct WuiText: View, WuiComponent {
    static var id: CWaterUI.WuiTypeId {
        waterui_text_id()
    }

    @ObservedObject private var content: WuiComputed<CWaterUI.WuiStyledStr>
    private var env: WuiEnvironment
    
    @State private var attributedString: AttributedString

    init(anyview: OpaquePointer, env: WuiEnvironment) {
        self.init(text: waterui_force_as_text(anyview), env: env)
    }
    
    init(text:CWaterUI.WuiText, env: WuiEnvironment) {
        self.env = env
        let computed = WuiComputed<CWaterUI.WuiStyledStr>(
            inner: text.content,
            read: waterui_read_computed_attributed_str,
            watch: { ptr, f in
                let watcher = CWaterUI.WuiWatcher_WuiStyledStr(f)
                let guardPtr = waterui_watch_computed_attributed_str(ptr, watcher)
                return WatcherGuard(guardPtr!)
            },
            drop: waterui_drop_computed_attributed_str
        )
        self._content = ObservedObject(wrappedValue: computed)
        self._attributedString = State(initialValue: Self.toAttributedString(from: computed.value, in: env))
    }

    var body: some View {
        SwiftUI.Text(attributedString)
            .onReceive(content.$value) { newValue in
                self.attributedString = Self.toAttributedString(from: newValue, in: env)
            }
    }

    static func toAttributedString(from styledStr: CWaterUI.WuiStyledStr, in env: WuiEnvironment) -> AttributedString {
        let cChunks = styledStr.chunks
        let slice = cChunks.vtable.slice(cChunks.data)
        let buffer = UnsafeBufferPointer(start: slice.head, count: Int(slice.len))
        
        var result = AttributedString()
        
        for chunk in buffer {
            let swiftString = WuiStr(chunk.text).toString()
            var chunkAS = AttributedString(swiftString)
            
            let style = chunk.style
            
            if let fontPtr = style.font {
                let resolvedFont = WuiFont(inner: fontPtr).resolve(in: env).value
                chunkAS.font = .system(size: CGFloat(resolvedFont.size), weight: resolvedFont.weight.toSwiftUI())
            }
            
            if let fg = style.foreground {
                chunkAS.foregroundColor = WuiColor(fg).resolve(in: env).value.toSwiftUI()
            }
            
            if style.italic {
                chunkAS.obliqueness = 0.2
            }
            if style.underline {
                chunkAS.underlineStyle = .single
            }
            if style.strikethrough {
                chunkAS.strikethroughStyle = .single
            }
            
            result.append(chunkAS)
        }
        
        return result
    }
}

// MARK: - Helpers

@MainActor
class WuiFont {
    var inner: OpaquePointer

    init(inner: OpaquePointer) {
        self.inner = inner
    }

    func resolve(in env: WuiEnvironment) -> WuiComputed<CWaterUI.WuiResolvedFont> {
        let computedPtr = waterui_resolve_font(self.inner, env.inner)
        return WuiComputed<CWaterUI.WuiResolvedFont>(inner: computedPtr!)
    }

    deinit {
        waterui_drop_font(inner)
    }
}

extension CWaterUI.WuiFontWeight {
    func toSwiftUI() -> Font.Weight {
        switch self {
        case WuiFontWeight_Thin: return .thin
        case WuiFontWeight_UltraLight: return .ultraLight
        case WuiFontWeight_Light: return .light
        case WuiFontWeight_Normal: return .regular
        case WuiFontWeight_Medium: return .medium
        case WuiFontWeight_SemiBold: return .semibold
        case WuiFontWeight_Bold: return .bold
        case WuiFontWeight_UltraBold: return .heavy
        case WuiFontWeight_Black: return .black
        default: return .regular
        }
    }
}

extension WuiComputed where T == CWaterUI.WuiResolvedFont {
    convenience init(inner: OpaquePointer) {
        self.init(
            inner: inner,
            read: waterui_read_computed_resolved_font,
            watch: { ptr, f in
                let watcher = CWaterUI.WuiWatcher_WuiResolvedFont(f)
                let guardPtr = waterui_watch_computed_resolved_font(ptr, watcher)
                return WatcherGuard(guardPtr!)
            },
            drop: waterui_drop_computed_resolved_font
        )
    }
}