import CoreGraphics
import Foundation

// Prints the backing scale of the main display, in pixels per point: the
// scale `screencapture` captures a window at on the E2E runner's single
// screen. A fractional scale is an error, since the screenshot comparer sizes
// its twin block in whole pixels.

guard let mode = CGDisplayCopyDisplayMode(CGMainDisplayID()) else {
  FileHandle.standardError.write(Data("the main display reports no mode\n".utf8))
  exit(1)
}
guard mode.width > 0, mode.pixelWidth % mode.width == 0 else {
  FileHandle.standardError.write(
    Data("main display scale \(mode.pixelWidth)/\(mode.width) is not a whole number\n".utf8)
  )
  exit(1)
}
print(mode.pixelWidth / mode.width)
