import AppKit
import Foundation

// Launches an .app through LaunchServices with activation and prints the new
// process's pid. Executing Contents/MacOS/<exe> directly bypasses
// LaunchServices, so the app is never activated and its windows never become
// key — activation is the launcher's job since macOS 14's cooperative
// activation. The returned pid is a launchd child, not a child of the caller:
// the shell kills it and polls for its disappearance, it cannot `wait` it.

guard CommandLine.arguments.count >= 2 else {
  FileHandle.standardError.write(Data("usage: launch-app <app-path> [args...]\n".utf8))
  exit(2)
}

let appURL = URL(fileURLWithPath: CommandLine.arguments[1])
let configuration = NSWorkspace.OpenConfiguration()
configuration.activates = true
configuration.createsNewApplicationInstance = true
configuration.arguments = Array(CommandLine.arguments.dropFirst(2))

// The completion handler's delivery queue is undocumented, so it hands its
// result to the main queue, and the main run loop runs until that arrives.
var launched: NSRunningApplication?
var launchError: Error?
var finished = false
NSWorkspace.shared.openApplication(at: appURL, configuration: configuration) { app, error in
  DispatchQueue.main.async {
    launched = app
    launchError = error
    finished = true
  }
}
while !finished {
  RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.1))
}

guard let launched else {
  FileHandle.standardError.write(
    Data(
      "launch-app: \(launchError?.localizedDescription ?? "openApplication returned no application and no error")\n"
        .utf8))
  exit(1)
}
print(launched.processIdentifier)
