import SwiftUI
import Foundation

// Case fixtures for the layout twins measurement run.
// Every measured part carries a tag("<case>.<part>") so its global frame is
// recorded. Where a proposal must be observed directly, the child sits inside
// a ProbeLayout that forwards explicit ProposedViewSize probes.

let textFont = Font.system(size: 16.7, design: .monospaced)

// A2 text fixture — wraps to two 10-glyph lines at width 100.
@MainActor func a2Text() -> Text { Text("mmmmmmmmmm mmmmmmmmmm").font(textFont) }
// Shorter-last-line text: wraps with a trailing short line.
@MainActor func a2ShortText() -> Text { Text("mm mm mm mm mm mm mm m").font(textFont) }

// Proposals the ProbeLayout fixtures probe their child with.
let textProbes: [ProposedViewSize] = [
    ProposedViewSize(width: 100, height: 20),
    ProposedViewSize(width: 100, height: 40),
    ProposedViewSize(width: 0, height: 20),
    ProposedViewSize(width: 100, height: 0),
    ProposedViewSize(width: 0, height: 0),
    ProposedViewSize(width: 0, height: nil),
    ProposedViewSize(width: nil, height: 0),
    ProposedViewSize(width: 100, height: nil),
    ProposedViewSize(width: nil, height: 20),
    ProposedViewSize(width: nil, height: nil),
    ProposedViewSize(width: .infinity, height: 20),
    ProposedViewSize(width: 100, height: .infinity),
]
let scrollProbes: [ProposedViewSize] = [
    ProposedViewSize(width: 100, height: 80),
    ProposedViewSize(width: 0, height: 80),
    ProposedViewSize(width: nil, height: 80),
    ProposedViewSize(width: .infinity, height: 80),
    ProposedViewSize(width: 100, height: nil),
    ProposedViewSize(width: 100, height: .infinity),
]

@MainActor func a3Row(_ id: String, _ wrapped: some View) -> some View {
    HStack(spacing: 10) {
        R(20, 10, id + ".l")
        wrapped
        R(20, 10, id + ".r")
    }
    .frame(width: 100, height: 60)
    .tag(id + ".stack")
}

@MainActor @ViewBuilder
func listCases() -> some View {
    // ---------- A2: Text ----------
    HStack(spacing: 0) { a2Text().tag("a2.wrap.text") }
        .frame(width: 100, height: 20).tag("a2.wrap.host")
    HStack(spacing: 0) { a2Text().lineLimit(1).tag("a2.limit1.text") }
        .frame(width: 100, height: 60).tag("a2.limit1.host")
    HStack(spacing: 0) { a2Text().lineLimit(2).tag("a2.limit2.text") }
        .frame(width: 100, height: 60).tag("a2.limit2.host")
    HStack(spacing: 0) { a2Text().tag("a2.subline.text") }
        .frame(width: 100, height: 8).tag("a2.subline.host")
    HStack(spacing: 0) { a2Text().tag("a2.w0.text") }
        .frame(width: 0, height: 20).tag("a2.w0.host")
    HStack(spacing: 0) { a2Text().tag("a2.h0.text") }
        .frame(width: 100, height: 0).tag("a2.h0.host")
    HStack(spacing: 0) { a2ShortText().tag("a2.short.text") }
        .frame(width: 100, height: 60).tag("a2.short.host")
    ProbeLayout(name: "a2.probe", proposals: textProbes) {
        a2Text().tag("a2.probe.text")
    }.frame(width: 100, height: 60).tag("a2.probe.host")

    // ---------- A3: wrappers around empty / rigid ----------
    // Membership probe: empty view inside a fixed-width frame in a spaced
    // stack at a tight proposal.
    HStack(spacing: 10) {
        R(20, 10, "a3.mem.l")
        E("a3.mem.e").frame(width: 40)
        R(20, 10, "a3.mem.r")
    }
    .frame(width: 100, height: 10).tag("a3.mem.stack")

    // Bare empty member (no wrapper) and rigid member — membership controls.
    HStack(spacing: 10) {
        R(20, 10, "a3.bare.l")
        E("a3.bare.e")
        R(20, 10, "a3.bare.r")
    }
    .frame(width: 100, height: 10).tag("a3.bare.stack")
    HStack(spacing: 10) {
        R(20, 10, "a3.memr.l")
        R(20, 10, "a3.memr.d")
        R(20, 10, "a3.memr.r")
    }
    .frame(width: 100, height: 10).tag("a3.memr.stack")

    a3Row("a3.frame.full", R(20, 10, "a3.frame.full.c").frame(width: 40).tag("a3.frame.full.w"))
    a3Row("a3.frame.empty", E("a3.frame.empty.c").frame(width: 40).tag("a3.frame.empty.w"))
    a3Row("a3.pad.full", R(20, 10, "a3.pad.full.c").padding(10).tag("a3.pad.full.w"))
    a3Row("a3.pad.empty", E("a3.pad.empty.c").padding(10).tag("a3.pad.empty.w"))
    a3Row("a3.aspect.full", R(20, 10, "a3.aspect.full.c").aspectRatio(2, contentMode: .fit).tag("a3.aspect.full.w"))
    a3Row("a3.aspect.empty", E("a3.aspect.empty.c").aspectRatio(2, contentMode: .fit).tag("a3.aspect.empty.w"))
    a3Row("a3.zstack.full", ZStack { R(20, 10, "a3.zstack.full.c") }.tag("a3.zstack.full.w"))
    a3Row("a3.zstack.empty", ZStack { E("a3.zstack.empty.c") }.tag("a3.zstack.empty.w"))
    a3Row("a3.overlay.full", R(20, 10, "a3.overlay.full.c").overlay(R(20, 10, "a3.overlay.full.d")).tag("a3.overlay.full.w"))
    a3Row("a3.overlay.empty", E("a3.overlay.empty.c").overlay(R(20, 10, "a3.overlay.empty.d")).tag("a3.overlay.empty.w"))
    a3Row("a3.bg.full", R(20, 10, "a3.bg.full.c").background(R(20, 10, "a3.bg.full.d")).tag("a3.bg.full.w"))
    a3Row("a3.bg.empty", E("a3.bg.empty.c").background(R(20, 10, "a3.bg.empty.d")).tag("a3.bg.empty.w"))
    a3Row("a3.guide.full", R(20, 10, "a3.guide.full.c")
        .alignmentGuide(.leading) { d in d[.leading] - 5 }
        .tag("a3.guide.full.w"))
    a3Row("a3.guide.empty", E("a3.guide.empty.c")
        .alignmentGuide(.leading) { d in d[.leading] - 5 }
        .tag("a3.guide.empty.w"))
    a3Row("a3.grid.full", Grid { GridRow { R(20, 10, "a3.grid.full.c") } }.tag("a3.grid.full.w"))
    a3Row("a3.grid.empty", Grid { GridRow { E("a3.grid.empty.c") } }.tag("a3.grid.empty.w"))

    // ---------- A11: ScrollView ----------
    ScrollView { R(60, 300, "a11.v.content") }
        .frame(width: 100, height: 80).tag("a11.v.scroll")
    ScrollView { R(140, 300, "a11.wide.content") }
        .frame(width: 100, height: 80).tag("a11.wide.scroll")
    ScrollView(.horizontal) { R(300, 60, "a11.h.content") }
        .frame(width: 100, height: 80).tag("a11.h.scroll")
    ScrollView([.horizontal, .vertical]) { R(60, 30, "a11.both.content") }
        .frame(width: 100, height: 80).tag("a11.both.scroll")
    ScrollView { R(60, 300, "a11.w0.content") }
        .frame(width: 0, height: 80).tag("a11.w0.scroll")
    ProbeLayout(name: "a11.probe", proposals: scrollProbes) {
        ScrollView { R(60, 300, "a11.probe.content") }
    }.frame(width: 100, height: 80).tag("a11.probe.host")

    // ---------- A13: scale snapping ----------
    HStack(spacing: 0) {
        Fill("a13.f1"); Fill("a13.f2"); Fill("a13.f3")
    }.frame(width: 100, height: 10).tag("a13.host")
    VStack(spacing: 0) {
        Text("aaaaaaaaaa").font(.system(size: 8.4, design: .monospaced)).tag("a13.t1")
        Text("aaaaaaaaaa").font(.system(size: 8.4, design: .monospaced)).tag("a13.t2")
        Text("aaaaaaaaaa").font(.system(size: 8.4, design: .monospaced)).tag("a13.t3")
    }.frame(width: 100).tag("a13.thost")

    // ---------- D: divergences ----------
    Grid { GridRow { R(20, 10, "d1.c1"); R(80, 10, "d1.c2") } }
        .frame(width: 200, height: 50).tag("d1.grid")
    R(30, 10, "d2.child")
        .aspectRatio(2, contentMode: .fit)
        .frame(width: 100, height: 100).tag("d2.aspect")
    R(100, 50, "d3.main")
        .background(R(20, 10, "d3.deco"))
        .frame(width: 100, height: 50).tag("d3.host")
    ZStack {
        R(100, 100, "d5.big").layoutPriority(0)
        R(20, 20, "d5.small").layoutPriority(1)
    }.frame(width: 200, height: 200).tag("d5.z")
}

// ---------- A12: safe area (rendered as root) ----------
@MainActor @ViewBuilder
func rootCase(_ name: String) -> some View {
    switch name {
    case "a12.scroll":
        ScrollView { R(60, 300, "a12.content") }
            .tag("a12.scroll")
    case "a12.ignore":
        ScrollView { R(60, 300, "a12.content") }
            .ignoresSafeArea()
            .tag("a12.scroll")
    case "a12.bar":
        ZStack {
            ScrollView { R(60, 300, "a12.content") }
                .tag("a12.scroll")
            VStack(spacing: 0) {
                Rectangle().fill(Color.red).frame(height: 44).tag("a12.bar")
                Spacer()
            }.ignoresSafeArea()
        }
    default:
        ScrollView {
            VStack(spacing: 4) { listCases() }
        }
    }
}
