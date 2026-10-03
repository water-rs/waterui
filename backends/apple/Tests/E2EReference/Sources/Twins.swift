// Twin registry and shared translation helpers.
//
// Each twin is a hand-written SwiftUI reproduction of a waterui example's first
// screen. The translation rules are fixed and mechanical:
//
//   vstack/hstack bare            -> VStack/HStack with spacing 10 (WaterUI's
//                                    default stack spacing)
//   .spacing(n)                    -> spacing: n
//   .alignment(Leading)            -> alignment: .leading
//   .padding()                     -> .padding(14) (WaterUI's default padding)
//   .padding_with(all(n))          -> .padding(n)
//   .padding_with(symmetric(v, h)) -> .padding(.vertical, v).padding(.horizontal, h)
//   .width/.height/.size           -> .frame(...)
//   .min_width/.min_height         -> .frame(minWidth:, minHeight:)
//   text(...).title()/.headline()  -> .font(.title)/.font(.headline) ...
//   text(...).size(n)              -> .font(.system(size: n))
//   .bold()                        -> .fontWeight(.bold): WaterUI's bold is
//                                    FontWeight::Bold (700), while SwiftUI's
//                                    .bold() trait resolves to the semibold
//                                    face on system fonts
//   Srgb::from_hex("#RRGGBB")      -> Color(.sRGB, red:..., green:..., blue:...)
//   ResolvedColor{...} / color()   -> linearSrgb(r, g, b) — ResolvedColor
//                                    stores linear sRGB components; Color(.sRGB)
//                                    would render them gamma-shifted dark
//   .with_opacity(x)               -> .opacity(x) on the Color
//   Foreground / MutedForeground   -> .primary / .secondary
//   Divider                        -> wuiDivider() (stack separators only;
//                                    inside Menu/contextMenu content a WaterUI
//                                    Divider maps to a native menu separator,
//                                    which stays `Divider()`)
//   spacer() / spacer().height(n)  -> Spacer(minLength: 0) / the same framed;
//                                    WaterUI's spacer has a zero minimum while
//                                    SwiftUI's Spacer() defaults to ~8pt
//   button("X").action(...)        -> Button("X") {}
//   Toggle::new("X", &b)           -> Toggle("X", isOn:)
//   TextField::new("X", &b)        -> label above + TextField, matching
//                                    WuiTextField's label-over-field layout
//
// Interactive bindings become @State with their initial values; gesture
// handlers are omitted because only the settled first screen is compared.

import SwiftUI

struct TwinRoot: View {
  private var example: String {
    UserDefaults.standard.string(forKey: "E2EExample") ?? ""
  }

  var body: some View {
    switch example {
    case "animation": AnimationTwin()
    case "drag_drop": DragDropTwin()
    case "edge_layout": EdgeLayoutTwin()
    case "edge_list": EdgeListTwin()
    case "edge_text": EdgeTextTwin()
    case "filter": FilterTwin()
    case "form": FormTwin()
    case "gesture": GestureTwin()
    case "gradient": GradientTwin()
    case "hover": HoverTwin()
    case "image": ImageTwin()
    case "liquid_glass": LiquidGlassTwin()
    case "list": ListTwin()
    case "locale": LocaleTwin()
    case "markdown": MarkdownTwin()
    case "menu": MenuTwin()
    case "multi_window": MultiWindowTwin()
    case "navigation": NavigationTwin()
    case "picker": PickerTwin()
    case "reminders": RemindersTwin()
    case "shape": ShapeTwin()
    case "snackbar": SnackbarTwin()
    case "typography-rtl": TypographyRtlTwin()
    default:
      Text("No SwiftUI twin registered for example '\(example)'")
    }
  }
}

// MARK: - Translation helpers

/// WaterUI's `Divider` is a 1pt bar filled with the theme's Border slot —
/// `UIColor.separator` / `NSColor.separatorColor` on this backend — that spans
/// the stack's cross axis. SwiftUI's `Divider` is a different thing: a 1-pixel
/// hairline in the opaque separator color, ~2px thinner at 3x.
func wuiDivider() -> some View {
  #if os(iOS)
    return Color(uiColor: .separator).frame(height: 1)
  #else
    return Color(nsColor: .separatorColor).frame(height: 1)
  #endif
}

/// `ResolvedColor` components — the waterui color currency is *linear* sRGB,
/// so values authored into `ResolvedColor`/`color()`/`palette_color` (e.g. the
/// gradient example's palettes) must be handed to SwiftUI as sRGBLinear, not
/// sRGB: `Color(.sRGB)` re-encodes them and renders the palette too dark.
func linearSrgb(_ r: Double, _ g: Double, _ b: Double) -> Color {
  Color(.sRGBLinear, red: r, green: g, blue: b, opacity: 1)
}

/// `Color::srgb_hex("#RRGGBB")`
func srgbHex(_ hex: UInt32) -> Color {
  Color(
    .sRGB,
    red: Double((hex >> 16) & 0xFF) / 255,
    green: Double((hex >> 8) & 0xFF) / 255,
    blue: Double(hex & 0xFF) / 255
  )
}
