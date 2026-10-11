// Twin of examples/menu: menu sections inside a padded scroll stack.
//
// Initial bindings render their defaults: "None" for the menu selection and
// "No action yet" / "No toolbar action yet" for the action logs. Context menus
// and popup contents never appear in the settled first screen, so only the
// presenting rows need to exist.

import SwiftUI

struct MenuTwin: View {
  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("WaterUI Menu Examples").font(.headline)
        Text("Demonstrating popup menus, nested menus, and context menus")
          .font(.body)
          .foregroundStyle(.secondary)
        wuiDivider()
        Spacer().frame(height: 8)
        menuSection
        wuiDivider()
        styledMenuSection
        wuiDivider()
        contextMenuSection
        wuiDivider()
        VStack(spacing: 10) {
          contextMenuViewsSection
          wuiDivider()
          selectionMenuSection
        }
        wuiDivider()
        toolbarSceneSection
        Spacer().frame(height: 40)
      }
      .padding(16)
    }
    // window_toolbar: the window's own actions — the same hstack of three
    // semantic-label buttons the content section shows, which the backend
    // splits into one NSToolbarItem per child at the toolbar's trailing edge.
    .toolbar {
      ToolbarItem(placement: .primaryAction) {
        toolbarButton("Search", icon: "[S]", iconOnly: true)
      }
      ToolbarItem(placement: .primaryAction) {
        toolbarButton("Compose", icon: "[+]", iconOnly: true)
      }
      ToolbarItem(placement: .primaryAction) {
        toolbarButton("Settings", icon: "[=]", iconOnly: true)
      }
    }
    // The example declares `Window::new("WaterUI Menu Examples", ...)`; the
    // reference host would otherwise title the window after the Water.toml
    // name ("Menu Example").
    .navigationTitle("WaterUI Menu Examples")
  }

  private var menuSection: some View {
    section("Menu Component", "Tap the menu button to see nested buttons, submenus, and separators") {
      Menu("Choose an Option") {
        Button("Option A") {}
        Button("Option B") {}
        Divider()
        Menu("More Options") {
          Button("Option C") {}
          Button("Reset") {}
        }
      }
      Spacer().frame(height: 12)
      HStack(spacing: 10) {
        Text("Selected: ").font(.caption).foregroundStyle(.secondary)
        Text("None").font(.body)
      }
    }
  }

  private var styledMenuSection: some View {
    section("Styled Menu", "The popup label stays a normal label, and menu rows can now be plain buttons") {
      Menu {
        Button("Edit") {}
        Button("Duplicate") {}
        Divider()
        Menu("Danger Zone") {
          Button("Delete") {}
        }
      } label: {
        Text("Actions").fontWeight(.bold)
      }
      Spacer().frame(height: 12)
      Text("No action yet").font(.caption).foregroundStyle(.secondary)
    }
  }

  private var contextMenuSection: some View {
    section("Context Menu", "Long press the box below to see context menu") {
      Text("Long Press Me")
        .padding(24)
        .background(srgbHex(0xFFF3E0))
        .foregroundStyle(srgbHex(0xE65100))
        .contextMenu {
          Button("Copy") {}
          Button("Cut") {}
          Divider()
          Button("Paste") {}
          Button("Select All") {}
        }
      Spacer().frame(height: 12)
      Text("No action yet").font(.caption).foregroundStyle(.secondary)
    }
  }

  private var contextMenuViewsSection: some View {
    section("Context Menu on Views", "Long press any colored box") {
      HStack(spacing: 0) {
        contextColorBox("Red", color: srgbHex(0xF44336))
        Spacer().frame(width: 12)
        contextColorBox("Green", color: srgbHex(0x4CAF50))
        Spacer().frame(width: 12)
        contextColorBox("Blue", color: srgbHex(0x2196F3))
      }
      Spacer().frame(height: 12)
      Text("No action yet").font(.caption).foregroundStyle(.secondary)
    }
  }

  private func contextColorBox(_ title: String, color: Color) -> some View {
    Text(title)
      .foregroundStyle(.white)
      .padding(14)
      .background(color)
      .contextMenu {
        Button("\(title) Action 1") {}
        Button("\(title) Action 2") {}
      }
  }

  private var selectionMenuSection: some View {
    section("Selection Menu", "Select text in the field and use its context menu") {
      VStack(alignment: .leading, spacing: 4) {
        Text("Draft")
        TextField("", text: .constant("Select some of this text, then open the menu"))
          .contextMenu {
            Button("Shout") {}
            Button("Whisper") {}
          }
      }
      Spacer().frame(height: 12)
      Text("No action yet").font(.caption).foregroundStyle(.secondary)
    }
  }

  private var toolbarSceneSection: some View {
    section(
      "Toolbar Labels",
      "The same semantic labels can render with text in content and icon-only in compact chrome. Screen readers keep reading the semantic title."
    ) {
      Text("Regular content").font(.caption).foregroundStyle(.secondary)
      toolbarActions(iconOnly: false)
      Spacer().frame(height: 8)
      Text("Compact toolbar").font(.caption).foregroundStyle(.secondary)
      toolbarActions(iconOnly: true)
      Spacer().frame(height: 12)
      Text("Toolbar action: No toolbar action yet").font(.caption).foregroundStyle(.secondary)
    }
  }

  private func toolbarActions(iconOnly: Bool) -> some View {
    HStack(spacing: 8) {
      toolbarButton("Search", icon: "[S]", iconOnly: iconOnly)
      toolbarButton("Compose", icon: "[+]", iconOnly: iconOnly)
      toolbarButton("Settings", icon: "[=]", iconOnly: iconOnly)
    }
  }

  private func toolbarButton(_ title: String, icon: String, iconOnly: Bool) -> some View {
    Button {} label: {
      if iconOnly {
        Text(icon).font(.caption).fontWeight(.bold)
      } else {
        Label {
          Text(title)
        } icon: {
          Text(icon).font(.caption).fontWeight(.bold)
        }
      }
    }
    .buttonStyle(.borderless)
  }

  private func section<Content: View>(
    _ title: String, _ subtitle: String, @ViewBuilder content: () -> Content
  ) -> some View {
    VStack(spacing: 10) {
      Text(title).font(.subheadline)
      Text(subtitle).font(.body).foregroundStyle(.secondary)
      Spacer().frame(height: 12)
      content()
    }
    .padding(14)
  }
}
