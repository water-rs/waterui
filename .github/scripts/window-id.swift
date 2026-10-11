import CoreGraphics
import Foundation

// Prints the CGWindowID of the largest on-screen, normal-layer window owned by
// the process given as argv[1]. Between "the app process started" and "the
// window is on the window server" there is no log line to wait on — the window
// existing is the readiness signal, so poll the window server until it appears
// or the deadline passes.

guard CommandLine.arguments.count == 2, let pid = Int32(CommandLine.arguments[2 - 1]) else {
  FileHandle.standardError.write(Data("usage: window-id.swift <pid>\n".utf8))
  exit(2)
}

func largestWindowID(forPID pid: Int32) -> Int? {
  let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
  guard
    let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]]
  else {
    return nil
  }
  var best: (id: Int, area: CGFloat)?
  for window in list {
    guard
      let owner = window[kCGWindowOwnerPID as String] as? Int,
      owner == Int(pid),
      let layer = window[kCGWindowLayer as String] as? Int,
      layer == 0,
      let number = window[kCGWindowNumber as String] as? Int,
      let boundsDict = window[kCGWindowBounds as String] as? [String: Any],
      let bounds = CGRect(dictionaryRepresentation: boundsDict as CFDictionary),
      bounds.width >= 50, bounds.height >= 50
    else { continue }
    let area = bounds.width * bounds.height
    if let current = best, area <= current.area { continue }
    best = (number, area)
  }
  return best?.id
}

let deadline = Date().addingTimeInterval(30)
while true {
  if let windowID = largestWindowID(forPID: pid) {
    print(windowID)
    exit(0)
  }
  if Date() >= deadline {
    FileHandle.standardError.write(
      Data("no on-screen window for pid \(pid) within 30s\n".utf8)
    )
    exit(1)
  }
  Thread.sleep(forTimeInterval: 0.5)
}
