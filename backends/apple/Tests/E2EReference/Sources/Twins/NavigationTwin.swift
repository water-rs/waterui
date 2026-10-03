// Twin of examples/navigation: four toolbar tabs, the Inbox pane on screen —
// a NavigationStack with a large title, an unread-count subtitle, a search
// field, an Edit leading item, a Compose primary action, a Mark All Read
// bottom-bar item and a status item, over a list of message rows. Only the
// settled first screen is rendered, so routes and actions are inert.
//
// The example's icons are Material Design Icons drawn from their SVG paths
// (`MaterialIcon`): the backend rasterises them at 25 pt for tab items, 24 pt
// for iOS bar buttons and 18 pt for the Mac toolbar, and inline content takes
// the shape at the size the example frames it to.

import SwiftUI

private struct Message: Identifiable {
  let id: Int
  let sender: String
  let subject: String
  let preview: String
  var unread: Bool
  var flagged: Bool
}

private let seedMessages: [Message] = [
  ("Ada Lovelace", "Analytical engine notes", "The engine weaves algebraic patterns."),
  ("Grace Hopper", "Compiler timings", "Shaved another pass off the linker."),
  ("Alan Kay", "On messaging", "The big idea is messaging, not objects."),
  ("Barbara Liskov", "Substitution review", "Subtypes must not surprise their callers."),
  ("Ken Thompson", "Pipes", "One tool, one job, composed by the shell."),
  ("Margaret Hamilton", "Priority displays", "Overload handling saved the landing."),
].enumerated().map { index, fields in
  Message(
    id: index,
    sender: fields.0,
    subject: fields.1,
    preview: fields.2,
    unread: index % 2 == 0,
    flagged: false
  )
}

private enum Pane: Hashable {
  case inbox, library, gallery, settings
}

struct NavigationTwin: View {
  @State private var pane = Pane.inbox
  @State private var query = ""
  @State private var editing = false

  private var unreadCount: Int {
    seedMessages.filter(\.unread).count
  }

  /// The size the backend rasterises a navigation bar icon at: 24 pt for a
  /// `UIBarButtonItem` image, 18 pt for an `NSToolbarItem` image.
  private var composeIconSize: CGFloat {
    #if os(iOS)
      24
    #else
      18
    #endif
  }

  var body: some View {
    TabView(selection: $pane) {
      Tab(value: .inbox) {
        inboxPane
      } label: {
        tabLabel("Inbox", icon: .inbox)
      }
      .badge(unreadCount)
      Tab(value: .library) {
        librarySplit
      } label: {
        tabLabel("Library", icon: .imageAlbum)
      }
      Tab(value: .gallery) {
        Text("Gallery")
      } label: {
        tabLabel("Gallery", icon: .viewGallery)
      }
      Tab(value: .settings) {
        Text("Settings")
      } label: {
        tabLabel("Settings", icon: .cog)
      }
    }
  }

  /// A tab's label: the Material glyph the example declares, rasterised at
  /// the 25 pt the backend gives a tab bar image.
  private func tabLabel(_ title: String, icon: MaterialIcon) -> Label<Text, Image> {
    Label {
      Text(title)
    } icon: {
      icon.templateImage(size: 25)
    }
  }

  private enum Album: String, Hashable {
    case recents = "Recents"
    case favorites = "Favorites"
    case shared = "Shared with You"
    var count: Int {
      switch self {
      case .recents: return 128
      case .favorites: return 12
      case .shared: return 41
      }
    }
  }

  @State private var album: Album? = .recents

  private var librarySplit: some View {
    NavigationSplitView {
      List(selection: $album) {
        Section("Albums") {
          ForEach([Album.recents, .favorites, .shared], id: \.self) { album in
            Label {
              HStack {
                Text(album.rawValue)
                Spacer(minLength: 0)
                Text("\(album.count)").foregroundStyle(.secondary)
              }
            } icon: {
              MaterialIcon.album.view(size: 20)
                .foregroundStyle(Color.accentColor)
            }
            .tag(album)
          }
        }
      }
      .navigationTitle("Albums")
      .navigationSplitViewColumnWidth(min: 220, ideal: 280, max: 360)
    } detail: {
      VStack(alignment: .leading, spacing: 6) {
        Text("128 photos")
        Text(
          "On a wide window this is the trailing column beside the sidebar; on a phone the same declaration collapses into a pushed page with a back button."
        )
        .foregroundStyle(.secondary)
        Spacer(minLength: 0)
      }
      .padding()
      .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
  }

  private var inboxPane: some View {
    NavigationStack {
      List(seedMessages) { message in
        NavigationLink(value: message.id) {
          HStack(alignment: .top, spacing: 6) {
            Circle()
              .fill(Color.accentColor)
              .frame(width: 8, height: 8)
              .opacity(message.unread ? 1 : 0)
              .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 2) {
              HStack(spacing: 6) {
                Text(message.sender).font(.subheadline)
                Spacer(minLength: 0)
                if message.flagged {
                  MaterialIcon.flag.view(size: 14)
                    .foregroundStyle(Color.accentColor)
                }
              }
              Text(message.subject).font(.body)
              Text(message.preview).font(.caption).foregroundStyle(.secondary)
            }
          }
          .padding(.vertical, 8)
        }
      }
      .navigationTitle("Inbox")
      .navigationSubtitle("\(unreadCount) unread")
      // iOS 26's automatic placement inside TabView hoists the field toward
      // the tab-bar search slot and leaves an empty drawer stub; the
      // navigation-bar drawer is the field this twin is for. macOS has no
      // navigationBarDrawer — its automatic placement already draws it.
      #if os(iOS)
        .searchable(
          text: $query,
          placement: .navigationBarDrawer(displayMode: .always),
          prompt: "Search mail"
        )
      #else
        .searchable(text: $query, prompt: "Search mail")
      #endif
      .toolbar {
        ToolbarItem(placement: .navigation) {
          Button(editing ? "Done" : "Edit") {}
            .buttonStyle(.plain)
        }
        ToolbarItem(placement: .primaryAction) {
          Button {} label: {
            Label {
              Text("Compose")
            } icon: {
              MaterialIcon.pencil.templateImage(size: composeIconSize)
            }
          }
        }
        // The bottom toolbar is an iOS construct; on the Mac the example's
        // bottom-bar item has no slot to land in and is not shown.
        #if os(iOS)
          ToolbarItem(placement: .bottomBar) {
            Button("Mark All Read") {}
              .buttonStyle(.plain)
          }
        #endif
        ToolbarItem(placement: .status) {
          Text("\(unreadCount) unread")
            .font(.caption)
            .foregroundStyle(.secondary)
        }
      }
      .navigationDestination(for: Int.self) { _ in
        Text("Message")
      }
    }
  }
}
