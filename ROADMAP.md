# Road map

## 0.1.0 - First glance

- [x] Basic widgets: stack, text, scroll, form, ...
- [x] SwiftUI backend
- [x] ~~MVP of gtk4 backend~~ (Warning: GTK4 backend is not supported no longer)
- [x] Stabilized the design of the core

## 0.2.0 - Usable

- [x] Fix memory leak — regression tests cover the async task system to guard against regressions.
- [x] Stabilized the layout system — now exercised by the [`components/layout`](components/layout/) crate.
- [x] MVP of Android backend
- [x] CLI — shipped via the [`cli`](cli/) crate; future plugin scaffolding continues under 0.3 milestones.
- [x] Gesture support
- [x] Hot reload
- [x] ~~ i18n — baseline plugin available in [`plugins/i18n`](plugins/i18n/); cookbook coverage still needed. ~~ We require more work for ergonomics, delaying to v0.3.0
- [x] Styling (Theme system)
- [ ] Document all completed features in our book (👷WIP)

## 0.3.0 - Practical

- [x] Media widget — core playback components live in [`components/media`](components/media/)
- [ ] Resource manager
- [x] Canvas API
- [ ] Persistence
- [ ] Automation UI test
- [ ] Some platform-specific APIs (notification, camera, etc.)
- [ ] Faster hot reload
- [x] Accessibility

## 0.4.0 - Self-Rendering MVP

- [ ] MVP of self-rendering backend

## 0.5.0 - Rich text

- [x] RichText (👷WIP) — base renderer shipped in [`src/widget/rich_text.rs`](src/widget/rich_text.rs); editing support tracked below.
  - [ ] RichTextField — interactive editing surface, caret management, and selection APIs.
  - [x] Built-in markdown support

## 0.6.0 - Self-Rendering Enhancements

- [ ] Support more widgets in self-rendering backend

# 0.7.0 - Developing Enhancement

- [ ] Preview a view
- [ ] VSCode plugin

## 0.8.0 - Animated Self-Rendering

- [ ] Support animation in self-rendering backend
- [ ] Inspector
