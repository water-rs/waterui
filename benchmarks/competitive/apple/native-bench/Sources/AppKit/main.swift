import AppKit

// Explicit entry point, not @main: @main on an NSObject
// NSApplicationDelegate compiles to the two-argument
// NSApplicationMain(argc, argv) overlay, which for a nibless app
// (GENERATE_INFOPLIST_FILE, no NSMainNibFile/NSMainStoryboardFile)
// leaves NSApp.delegate nil — no will/didFinishLaunching, no window —
// while the process idles in its event loop (reproduced on Xcode 26.6).
// The runner's ready assertion could still pass on a stale latched
// post from an earlier cell, so this failure mode was silent.
let benchApp = NSApplication.shared
benchApp.setActivationPolicy(.regular)
let benchDelegate = AppDelegate()
benchApp.delegate = benchDelegate
benchApp.run()
