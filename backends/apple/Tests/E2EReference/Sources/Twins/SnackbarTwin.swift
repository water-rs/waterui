// Twin of examples/snackbar: title + seven buttons inside a padded scroll
// stack. Only the settled first screen is rendered, so snackbar actions are
// inert.

import SwiftUI

struct SnackbarTwin: View {
  var body: some View {
    ScrollView {
      VStack(spacing: 10) {
        Text("Snackbar Demo").font(.title).fontWeight(.bold)
        Spacer(minLength: 0)
        Button("Simple Snackbar") {}
        Button("With Icon") {}
        Button("With Action Button") {}
        Button("Top Position") {}
        Button("Queue Multiple") {}
        Button("Top + Bottom") {}
        Button("Closeable") {}
        Spacer(minLength: 0)
      }
      .padding(14)
    }
  }
}
