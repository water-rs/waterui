import AppKit
import Foundation
import SwiftUI
import Testing

private struct ContractProbeEvent: Encodable {
  let kind: String
  let name: String
  let proposalWidth: String
  let proposalHeight: String
  let x: Double
  let y: Double
  let width: Double
  let height: Double
}

private final class ContractProbeTrace {
  var events: [ContractProbeEvent] = []

  func record(_ kind: String, _ name: String, _ proposal: ProposedViewSize, _ bounds: CGRect) {
    events.append(ContractProbeEvent(
      kind: kind, name: name,
      proposalWidth: proposal.width.map { String(describing: $0) } ?? "unspecified",
      proposalHeight: proposal.height.map { String(describing: $0) } ?? "unspecified",
      x: Double(bounds.minX), y: Double(bounds.minY),
      width: Double(bounds.width), height: Double(bounds.height)
    ))
  }
}

private struct ContractProbeRecorder: Layout {
  let name: String
  let trace: ContractProbeTrace

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let size = subviews[0].sizeThatFits(proposal)
    trace.record("measure", name, proposal, CGRect(origin: .zero, size: size))
    return size
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    trace.record("place", name, proposal, bounds)
    subviews[0].place(at: bounds.origin, anchor: .topLeading, proposal: proposal)
  }
}

private struct ContractProbeProposal: Layout {
  let offered: ProposedViewSize
  let trace: ContractProbeTrace

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let size = subviews[0].sizeThatFits(offered)
    trace.record("root-measure", "root", offered, CGRect(origin: .zero, size: size))
    return size
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    let size = subviews[0].sizeThatFits(offered)
    trace.record("root-place", "root", offered, CGRect(origin: bounds.origin, size: size))
    subviews[0].place(at: bounds.origin, anchor: .topLeading, proposal: offered)
  }
}

private struct ContractProbeSections: View {
  let horizontal: Bool
  let trace: ContractProbeTrace

  @ViewBuilder
  private func section(_ extent: CGFloat) -> some View {
    if horizontal {
      HStack(spacing: 0) {
        Color.clear.frame(width: extent, height: 20)
        Spacer(minLength: 0)
      }
    } else {
      VStack(spacing: 0) {
        Color.clear.frame(width: 20, height: extent)
        Spacer(minLength: 0)
      }
    }
  }

  var body: some View {
    if horizontal {
      HStack(spacing: 0) {
        ContractProbeRecorder(name: "short", trace: trace) { section(40) }
        ContractProbeRecorder(name: "long", trace: trace) { section(120) }
      }
    } else {
      VStack(spacing: 0) {
        ContractProbeRecorder(name: "short", trace: trace) { section(40) }
        ContractProbeRecorder(name: "long", trace: trace) { section(120) }
      }
    }
  }
}

private struct ContractProbeCase: Encodable {
  let axis: String
  let mainProposal: String
  let events: [ContractProbeEvent]
}

@MainActor
struct LayoutContractProbeTests {
  @Test func captureNestedSectionNegotiation() throws {
    let output = try #require(ProcessInfo.processInfo.environment["WATERUI_LAYOUT_PROBE_OUTPUT"])
    var cases: [ContractProbeCase] = []
    for horizontal in [true, false] {
      for main: CGFloat? in [nil, 80, 160, 240, 320] {
        let trace = ContractProbeTrace()
        let proposal = ProposedViewSize(width: horizontal ? main : 100, height: horizontal ? 100 : main)
        let root = ContractProbeProposal(offered: proposal, trace: trace) {
          ContractProbeSections(horizontal: horizontal, trace: trace)
        }
        let host = NSHostingView(rootView: root)
        host.sizingOptions = []
        host.frame = CGRect(x: 0, y: 0, width: 400, height: 600)
        host.needsLayout = true
        host.layoutSubtreeIfNeeded()
        #expect(trace.events.contains { $0.kind == "place" && $0.name == "short" })
        #expect(trace.events.contains { $0.kind == "place" && $0.name == "long" })
        cases.append(ContractProbeCase(
          axis: horizontal ? "horizontal" : "vertical",
          mainProposal: main.map { String(describing: $0) } ?? "unspecified",
          events: trace.events
        ))
      }
    }
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    encoder.nonConformingFloatEncodingStrategy = .convertToString(positiveInfinity: "+infinity", negativeInfinity: "-infinity", nan: "nan")
    try encoder.encode(cases).write(to: URL(fileURLWithPath: output), options: .atomic)
  }
}
