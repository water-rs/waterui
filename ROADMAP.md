# Road map

## 0.1.0 - First glance

- [x] Basic widgets: stack, text, scroll, form, ...
- [x] SwiftUI backend
- [x] MVP of gtk4 backend
- [x] Stabilized the design of the core

## 0.2.0 - Usable

- [x] Fix memory leak — regression tests cover the async task system to guard against regressions.
- [x] Stabilized the layout system — now exercised by the [`components/layout`](components/layout/) crate.
- [x] MVP of Android backend
- [x] CLI — shipped via the [`cli`](cli/) crate; future plugin scaffolding continues under 0.3 milestones.
- [ ] Gesture support
  - [ ] Gesture integration: wire descriptors in [`src/gesture.rs`](src/gesture.rs) into remaining platform backends and ensure metadata propagation via [`src/view.rs`](src/view.rs).
- [ ] Hot reload
  - [ ] Faster hot reload: reuse dev-server state in [`cli/src/run.rs`](cli/src/run.rs) to avoid full rebuilds.
- [x] i18n — baseline plugin available in [`plugins/i18n`](plugins/i18n/); cookbook coverage still needed.
- [ ] Styling (Theme system)
  - [ ] Theming primitives: design shared tokens for [`components/text`](components/text/) and [`components/layout`](components/layout/).
  - [ ] Runtime overrides: expose environment hooks once the dedicated `theme` module lands.
- [ ] Document all completed features in our book (👷WIP)

## 0.3.0 - Practical

- [x] Media widget — core playback components live in [`components/media`](components/media/); streaming & backend parity still in progress.
  - [ ] Streaming parity: implement buffering/resume paths for Android/iOS in [`components/media/src`](components/media/src/).
  - [ ] Controls polish: align overlays with the navigation stack and expose callbacks for accessibility hooks.
- [ ] Resource manager
- [ ] Canvas API (👷WIP)
- [ ] Persistence
- [ ] Some platform-specific APIs (notification, camera, etc.)
- [ ] Faster hot reload
  - [ ] Tooling: integrate filesystem diff watching to shorten rebuild cycles.
- [ ] Accessibility (👷WIP)
  - [ ] Accessibility polish: expand semantics in [`src/accessibility.rs`](src/accessibility.rs) for custom widgets (media, rich text, etc.).

## 0.4.0 - Self-Rendering MVP

- [ ] MVP of self-rendering backend

## 0.5.0 - Rich text

- [x] RichText (👷WIP) — base renderer shipped in [`src/widget/rich_text.rs`](src/widget/rich_text.rs); editing support tracked below.
  - [ ] RichTextField — interactive editing surface, caret management, and selection APIs.
  - [ ] Built-in markdown support (👷WIP) — extend parser coverage (tables, callouts) and surface helpers in [`components/text`](components/text/).

## 0.6.0 - Self-Rendering Enhancements

- [ ] Support more widgets in self-rendering backend

# 0.7.0 - Developing Enhancement

- [ ] Preview a view
- [ ] VSCode plugin

## 0.8.0 - Animated Self-Rendering

- [ ] Support animation in self-rendering backend
- [ ] Inspector
