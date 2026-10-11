// Twin of Examples/liquid_glass: three tabs, the Surfaces pane on screen — a
// large-title navigation view whose scroll content is glass pills and a glass
// card over four offset colored discs. Only the settled first screen is
// rendered, so the tab bar's other panes and the interactive pill are inert.
//
//   Glass::regular()                          -> .glassEffect()
//   Glass::clear()                            -> .glassEffect(.clear)
//   .interactive(true)                        -> .interactive()
//   .tint(color)                              -> .tint(color)
//   .shape(RoundedRectangle::new(0.2))        -> in: .rect(cornerRadius:) resolved
//                                                against the card's shorter side
//   Circle.fill(c).size(n, n).offset(x, y)    -> Circle().fill(c).frame(n).offset(x, y)
//   .bottom_accessory(view)                   -> .tabViewBottomAccessory { view }

import SwiftUI

private enum Pane: Hashable {
  case surfaces, controls, about, search
}

struct LiquidGlassTwin: View {
  @State private var pane = Pane.surfaces
  @State private var query = ""
  /// First-paint signal the host injects — see `body`.
  @EnvironmentObject private var presentation: WuiReferencePresentation

  var body: some View {
    let tabs = TabView(selection: $pane) {
      Tab("Surfaces", systemImage: "square.on.square", value: .surfaces) {
        surfacesPage
      }
      Tab("Controls", systemImage: "button.horizontal", value: .controls) {
        controlsPage
      }
      Tab("About", systemImage: "info.circle", value: .about) {
        aboutPage
      }
      Tab("Search", systemImage: "magnifyingglass", value: .search, role: .search) {
        searchPage
      }
    }
    // The minimize behavior and bottom accessory exist only on iOS; on macOS
    // the tabs still render, which is what the twin's comparison needs.
    //
    // The accessory is inserted only after the host's first frame commits
    // rather than in the launch commit: when its materialise transition lands
    // inside the scene's own presentation on iOS 26 it can be dropped,
    // leaving the capsule flat and translucent for the rest of the process —
    // the iOS shard captured exactly that state twice as the 0.1688
    // liquid_glass parity diff (#256). Inserting into the already-live tab
    // bar is the path every real use of this API takes — a now-playing bar
    // appearing while the app runs.
    #if os(iOS)
      if presentation.painted {
        tabs
          .tabBarMinimizeBehavior(.onScrollDown)
          .tabViewBottomAccessory {
            nowPlaying
          }
      } else {
        tabs
          .tabBarMinimizeBehavior(.onScrollDown)
      }
    #else
      tabs
    #endif
  }

  /// The now-playing row the tab bar carries in its bottom accessory slot.
  private var nowPlaying: some View {
    HStack(spacing: 12) {
      Label("Play", systemImage: "play.fill")
      VStack(spacing: 1) {
        Text("Now Playing").fontWeight(.bold).font(.system(size: 15))
        Text("Liquid Glass — Surfaces").font(.system(size: 12))
      }
    }
    .padding(.vertical, 6)
    .padding(.horizontal, 16)
  }

  private var surfacesPage: some View {
    NavigationStack {
      ZStack {
        backdrop
        ScrollView {
          VStack(spacing: 12) {
            caption("Regular glass is the default: a capsule that stays legible over anything.")
            pill("Now Playing").glassEffect()
            caption("Clear glass diffuses less, for surfaces over media.")
            pill("Clear").glassEffect(.clear)
            caption("Interactive glass answers touch and hover with the platform's own effects.")
            pill("Tap me").glassEffect(.regular.interactive())
            caption("A tint washes the glass toward a color.")
            pill("Accent").glassEffect(.regular.tint(.accentColor))
            pill("Tomato")
              .glassEffect(.clear.tint(Color(.sRGB, red: 255 / 255, green: 99 / 255, blue: 71 / 255)))
            caption("The outline belongs to the glass, not to an outer clip.")
            card
            caption("The capsule follows the size of what it wraps.")
            Text("Small").fontWeight(.bold).font(.system(size: 13)).padding(8).glassEffect()
            pill("Medium").glassEffect()
            Text("Large").fontWeight(.bold).font(.system(size: 24)).padding(20).glassEffect()
          }
          .padding(14)
        }
      }
      .navigationTitle("Surfaces")
    }
  }

  private var controlsPage: some View {
    NavigationStack {
      ZStack {
        backdrop
        ScrollView {
          VStack(spacing: 12) {
            caption("Glass: the capsule is the emphasis, the label keeps the primary color.")
            HStack(spacing: 12) {
              Button("Glass") {}.buttonStyle(.glass)
              Button("Bordered") {}.buttonStyle(.bordered)
            }
            caption("Prominent glass: the accent fills the capsule, for the primary action.")
            HStack(spacing: 12) {
              Button("Glass Prominent") {}.buttonStyle(.glassProminent)
              Button("Bordered Prominent") {}.buttonStyle(.borderedProminent)
            }
            caption("Labels with symbols get the same capsule.")
            HStack(spacing: 12) {
              Button("Share", systemImage: "square.and.arrow.up") {}.buttonStyle(.glass)
              Button("Play", systemImage: "play.fill") {}.buttonStyle(.glassProminent)
            }
          }
          .padding(14)
        }
      }
      .navigationTitle("Controls")
    }
  }

  private var aboutPage: some View {
    NavigationStack {
      VStack(spacing: 12) {
        Text("Liquid Glass").font(.system(size: 24))
        Text(
          "Glass is the chrome-layer surface of iOS 26 and macOS 26. This app declares it; the Apple backend projects it."
        )
      }
      .padding(14)
      .navigationTitle("About")
    }
  }

  private var searchPage: some View {
    NavigationStack {
      VStack(spacing: 12) {
        Text("Results for “\(query)”")
        caption("Glass terms: regular, clear, interactive, tint, capsule, card.")
      }
      .padding(14)
      .navigationTitle("Search")
      .searchable(text: $query, prompt: "Search glass")
    }
  }

  private func caption(_ body: String) -> some View {
    Text(body).font(.system(size: 15))
  }

  private func pill(_ title: String) -> some View {
    Text(title).fontWeight(.bold).padding(14)
  }

  private var card: some View {
    VStack(spacing: 6) {
      Text("Rounded card").fontWeight(.bold)
      Text("Text inside glass keeps its full contrast; the glass adapts to what is behind it.")
    }
    .padding(14)
    // RoundedRectangle::new(0.2) is a fifth of the shorter side; the card is
    // two text lines plus padding, so the radius resolves to about 14 points.
    .glassEffect(in: .rect(cornerRadius: 14))
  }

  /// Large colored discs, so the lensing at each glass edge has edges to bend.
  private var backdrop: some View {
    ZStack {
      Circle().fill(Color(.sRGB, red: 255 / 255, green: 149 / 255, blue: 0))
        .frame(width: 320, height: 320).offset(x: -120, y: -160)
      Circle().fill(Color(.sRGB, red: 48 / 255, green: 176 / 255, blue: 199 / 255))
        .frame(width: 280, height: 280).offset(x: 140, y: 40)
      Circle().fill(Color(.sRGB, red: 175 / 255, green: 82 / 255, blue: 222 / 255))
        .frame(width: 360, height: 360).offset(x: -60, y: 320)
      Circle().fill(Color(.sRGB, red: 52 / 255, green: 199 / 255, blue: 89 / 255))
        .frame(width: 220, height: 220).offset(x: 150, y: 560)
    }
  }
}
