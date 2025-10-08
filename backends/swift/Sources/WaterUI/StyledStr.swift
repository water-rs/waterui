import CWaterUI
import SwiftUI

struct WuiStyledStr {
    var chunks: WuiArray<WuiStyledChunk>
    init(_ inner: CWaterUI.WuiStyledStr) {
        self.chunks = WuiArray(inner.chunks)
    }

    @MainActor
    func toAttributedString(env:WuiEnvironment) -> AttributedString {
        var result = AttributedString()
        
        for chunk in chunks.toArray() {
            result.append(chunk.toAttributedString(env: env))
        }
        
        return result
    }

}

extension WuiStyledChunk {
    @MainActor
    func toAttributedString(env:WuiEnvironment) -> AttributedString {
        var attrStr = AttributedString(WuiStr(self.text).toString())
        let style = self.style
        let font = WuiFont(style.font).resolve(in: env).value.toSwiftUI()
        
        let foreground = WuiColor(style.foreground).resolve(in: env).value.toSwiftUI()
        
        let background = WuiColor(style.background).resolve(in: env).value.toSwiftUI()
        
        let underline = style.underline
        
        let strikethrough = style.strikethrough
        
        let italic = style.italic
        
        attrStr.font = font
        attrStr.foregroundColor = foreground
        attrStr.backgroundColor = background
        if underline {
            attrStr.underlineStyle = .single
        }
        
        if strikethrough {
            attrStr.strikethroughStyle = .single
        }
        
        if italic {
            attrStr.font = font.italic()
        }
        
        return attrStr
    }
}

extension WuiResolvedFont{
    func toSwiftUI() -> SwiftUI.Font {
        let size = CGFloat(self.size)
        let weight = self.weight.toSwiftUI()
        
        return .system(size: size,weight: weight)
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

@MainActor
class WuiFont {
    var inner: OpaquePointer

    init(_ inner: OpaquePointer) {
        self.inner = inner
    }

    func resolve(in env: WuiEnvironment) -> WuiComputed<CWaterUI.WuiResolvedFont> {
        let computedPtr = waterui_resolve_font(self.inner, env.inner)
        return WuiComputed<CWaterUI.WuiResolvedFont>(computedPtr!)
    }

    @MainActor deinit {
        waterui_drop_font(inner)
    }
}
