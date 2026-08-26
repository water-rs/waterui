# Road map

## 0.1.0 - First glance

- [x] Basic widgets: stack, text, scroll, form, ...
- [x] SwiftUI backend
- [x] MVP of gtk4 backend
- [x] Stabilized the design of the core

## 0.2.0 - Usable

- [x] Fix memory leak — regression tests cover the async task system to guard against regressions.
- [x] Stabilized the layout system — now exercised by the [`components/foundation/layout`](../components/foundation/layout/) crate.
- [x] MVP of Android backend
- [x] CLI — shipped via the [`cli`](../cli/) crate; future plugin scaffolding continues under 0.3 milestones.
- [x] Gesture support
- [x] Preview system
- [x] Locale and layout-direction foundations — implemented by [`waterui-locale`](../utils/locale/) and the text/layout stack.
- [x] Styling (Theme system)
- [ ] Document all completed features in our book (👷WIP)

## 0.3.0 - Practical

- [x] Media widget — core playback components live in [`components/multimedia/media`](../components/multimedia/media/).
- [x] Resource manager — typed planning, embedding, and runtime loading live in [`components/assets`](../components/assets/).
- [x] Canvas API
- [x] Persistence — typed navigation paths support serde restoration and atomic reprojection through [`waterui-navigation`](../components/foundation/navigation/).
- [x] Automation UI test — accessibility-first interaction, waiting, snapshots, and benchmarks live in [`waterui-testing`](../testing/).
- [x] Platform-specific APIs — WaterKit provides camera, notification, permission, location, media, identity, and sharing services under [`kit`](../kit/).
- [x] Accessibility — semantic testing and native backend projection cover the public component model.

## 0.4.0 - Self-Rendering MVP

- [ ] MVP of self-rendering backend

## 0.5.0 - Rich text

- [x] RichText (👷WIP) — the base renderer ships in [`components/foundation/text`](../components/foundation/text/); editing support is tracked below.
  - [ ] RichTextField — interactive editing surface, caret management, and selection APIs.
  - [x] Built-in markdown support

## 0.6.0 - Self-Rendering Enhancements

- [ ] Support more widgets in self-rendering backend

# 0.7.0 - Developing Enhancement

- [x] Preview a view
- [ ] VSCode plugin

## 0.8.0 - Animated Self-Rendering

- [ ] Support animation in self-rendering backend
- [ ] Inspector
