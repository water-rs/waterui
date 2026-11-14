// Layout twins — WaterUI frame recorder.
//
// Hosts the packaged app's `waterui_app` entry in-process (the same mechanism
// as the apple-backend device suite: the archive is linked into this test
// bundle via OTHER_LDFLAGS and `waterui_*` resolves by dynamic lookup), shows
// its root view in a real window, pumps the run loop, then walks the native
// view tree and writes every view's class, frame (window coordinates), and
// `.a11y_label` name as JSON.
//
// `TWINS_CASE` selects the app's root fixture (see src/lib.rs), `TWINS_WIN`
// optionally overrides the hosted window size as "WxH", and `TWINS_OUT` gives
// a file path the JSON is also written to (it is always printed between
// TWINS_JSON_BEGIN / TWINS_JSON_END markers on stdout).

import Foundation
import XCTest

import WaterUI

#if canImport(UIKit)
  import UIKit
#elseif canImport(AppKit)
  import AppKit
#endif

private func parseSize(_ s: String?) -> CGSize? {
  guard let s, let x = s.firstIndex(of: "x") else { return nil }
  guard let w = Double(s[..<x]), let h = Double(s[s.index(after: x)...]) else { return nil }
  return CGSize(width: w, height: h)
}

// JSONSerialization rejects non-finite numbers; frames and scroll metrics can
// legitimately contain ±inf, so map them to strings before writing.
private func sanitizeJSON(_ v: Any) -> Any {
  switch v {
  case let d as Double:
    if d.isNaN { return "nan" }
    if d == .infinity { return "inf" }
    if d == -.infinity { return "-inf" }
    return d
  case let f as CGFloat:
    return sanitizeJSON(Double(f))
  case let a as [Any]:
    return a.map(sanitizeJSON)
  case let d as [String: Any]:
    return d.mapValues(sanitizeJSON)
  default:
    return v
  }
}

private func emitJSON(_ root: [String: Any], to path: String?) {
  let root = root.mapValues(sanitizeJSON)
  guard JSONSerialization.isValidJSONObject(root),
    let data = try? JSONSerialization.data(
      withJSONObject: root, options: [.prettyPrinted, .sortedKeys]),
    let text = String(data: data, encoding: .utf8)
  else { return }
  print("TWINS_JSON_BEGIN\n\(text)\nTWINS_JSON_END")
  if let path {
    try? data.write(to: URL(fileURLWithPath: path))
  }
}

@MainActor
final class TwinsTests: XCTestCase {

  // Objects we must never release: the backend's reactive teardown currently
  // crashes (a watcher released twice) when a live root is deallocated, which
  // would kill the runner after the dump. Leaking the context and window is
  // intentional; the process exits right after the test.
  private static var leaked: [Any] = []

  // One case per test so `-only-testing:` selects the fixture; env vars do
  // not reach a simulator test runner. TWINS_CASE still overrides for
  // launches where env does propagate (macOS).
  func testCaseAll() async throws { try await runCase("all") }
  func testCaseA12Scroll() async throws { try await runCase("a12.scroll") }
  func testCaseA12Ignore() async throws { try await runCase("a12.ignore") }
  func testCaseA12Bar() async throws { try await runCase("a12.bar") }

  func runCase(_ caseName: String) async throws {
    guard dlsym(dlopen(nil, RTLD_LAZY), "waterui_app") != nil else {
      throw XCTSkip(
        "no WaterUI app archive is linked — run measure.sh which passes "
          + "libwaterui_app.a via OTHER_LDFLAGS")
    }
    let env = ProcessInfo.processInfo.environment
    let selected = env["TWINS_CASE"] ?? caseName
    if env["TWINS_CASE_SET"] == nil {
      // Communicate the selected case to the Rust root: it reads TWINS_CASE
      // from the process environment.
      setenv("TWINS_CASE", selected, 1)
    }
    #if canImport(AppKit)
      // The context installs the app menu bar on creation, which dereferences
      // NSApp implicitly; xctest is not an NSApplication, so create the shared
      // instance first.
      _ = NSApplication.shared
    #endif
    let context = await WuiRootContext()
    Self.leaked.append(context)
    var nodes: [[String: Any]] = []
    var meta: [String: Any] = [
      "case": selected,
      "platform": "",
      "osVersion": ProcessInfo.processInfo.operatingSystemVersionString,
    ]

    #if canImport(UIKit)
      meta["platform"] = "ios"
      let scene = UIApplication.shared.connectedScenes
        .compactMap { $0 as? UIWindowScene }
        .first { $0.activationState == .foregroundActive }
      let size = parseSize(env["TWINS_WIN"])
        ?? scene?.screen.bounds.size
        ?? CGSize(width: 393, height: 852)
      let window = scene.map { UIWindow(windowScene: $0) } ?? UIWindow()
      window.frame = CGRect(origin: .zero, size: size)
      let controller = UIViewController()
      window.rootViewController = controller
      let rootView = context.rootView
      rootView.frame = window.bounds
      rootView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
      controller.view.addSubview(rootView)
      window.makeKeyAndVisible()
      Self.leaked.append(contentsOf: [rootView, window, controller])
      // Drive the run loop until layout and the display pass have settled.
      try await Task.sleep(for: .seconds(2.5))

      meta["window"] = [
        "x": window.frame.origin.x, "y": window.frame.origin.y,
        "w": window.frame.width, "h": window.frame.height,
      ]
      meta["safeInsets"] = [
        "top": window.safeAreaInsets.top, "leading": window.safeAreaInsets.left,
        "bottom": window.safeAreaInsets.bottom, "trailing": window.safeAreaInsets.right,
      ]
      meta["displayScale"] = Double(window.screen.scale)

      func walk(_ v: UIView, depth: Int) {
        var node: [String: Any] = [
          "depth": depth, "cls": String(describing: type(of: v)),
        ]
        let r = v.superview.map { $0.convert(v.frame, to: nil) } ?? v.frame
        node["frame"] = [
          r.origin.x, r.origin.y, r.width, r.height,
        ]
        if let l = v.accessibilityLabel, !l.isEmpty { node["label"] = l }
        if let i = v.accessibilityIdentifier, !i.isEmpty { node["id"] = i }
        if v.isHidden { node["hidden"] = true }
        if v.clipsToBounds { node["clips"] = true }
        if let s = v as? UIScrollView {
          node["contentSize"] = [s.contentSize.width, s.contentSize.height]
          node["contentOffset"] = [s.contentOffset.x, s.contentOffset.y]
          node["adjustedContentInset"] = [
            s.adjustedContentInset.top, s.adjustedContentInset.left,
            s.adjustedContentInset.bottom, s.adjustedContentInset.right,
          ]
        }
        if let l = v as? UILabel {
          node["numberOfLines"] = l.numberOfLines
          node["text"] = l.text ?? ""
        }
        nodes.append(node)
        for sub in v.subviews { walk(sub, depth: depth + 1) }
      }
      walk(rootView, depth: 0)

    #elseif canImport(AppKit)
      meta["platform"] = "macos"
      let size = parseSize(env["TWINS_WIN"]) ?? CGSize(width: 800, height: 600)
      let window = NSWindow(
        contentRect: NSRect(origin: .zero, size: size),
        styleMask: [.titled], backing: .buffered, defer: false)
      let controller = NSViewController()
      window.contentViewController = controller
      let rootView = context.rootView
      rootView.frame = controller.view.bounds
      rootView.autoresizingMask = [.width, .height]
      controller.view.addSubview(rootView)
      window.makeKeyAndOrderFront(nil)
      Self.leaked.append(contentsOf: [rootView, window, controller])
      try await Task.sleep(for: .seconds(2.5))

      let content = window.contentView ?? controller.view
      meta["window"] = [
        "x": 0, "y": 0, "w": content.frame.width, "h": content.frame.height,
      ]
      meta["displayScale"] = Double(window.backingScaleFactor)
      meta["flippedY"] = true

      func walk(_ v: NSView, depth: Int) {
        var node: [String: Any] = [
          "depth": depth, "cls": String(describing: type(of: v)),
        ]
        // Convert to window-content coordinates (AppKit Y grows upward).
        let r = v.superview.map { $0.convert(v.frame, to: content) } ?? v.frame
        node["frame"] = [
          r.origin.x, r.origin.y, r.width, r.height,
        ]
        if let l = v.accessibilityLabel(), !l.isEmpty { node["label"] = l }
        let i = v.accessibilityIdentifier()
        if !i.isEmpty { node["id"] = i }
        if v.isHidden { node["hidden"] = true }
        if let s = v as? NSScrollView {
          let doc = s.documentVisibleRect
          node["documentVisibleRect"] = [
            doc.origin.x, doc.origin.y, doc.width, doc.height,
          ]
          node["contentSize"] = [s.contentSize.width, s.contentSize.height]
          let insets = s.contentView.contentInsets
          node["contentInsets"] = [
            insets.top, insets.left, insets.bottom, insets.right,
          ]
        }
        if let t = v as? NSTextField {
          node["text"] = t.stringValue
        }
        nodes.append(node)
        for sub in v.subviews { walk(sub, depth: depth + 1) }
      }
      walk(rootView, depth: 0)
    #endif

    // Compact lookup: fixture label -> window-coordinate frame.
    var frames: [String: [CGFloat]] = [:]
    for node in nodes {
      if let label = node["label"] as? String, label.hasPrefix("twins."),
        let r = node["frame"] as? [CGFloat], r.count == 4
      {
        frames[label] = r
      }
    }
    emitJSON(["meta": meta, "frames": frames, "nodes": nodes], to: env["TWINS_OUT"])
  }
}
