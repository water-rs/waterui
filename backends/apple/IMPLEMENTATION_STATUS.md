# Apple Backend Implementation Status

This document tracks how the Apple backend maps onto the APIs exported by the core WaterUI Rust crates. Components and utilities are grouped roughly by crate/module. Status values:

- **✅ complete** – Behaviour matches the Rust/FFI contract in production.
- **⚠️ partial** – Present but missing behaviour that the Rust type can express.
- **❌ missing** – No implementation yet.

> File references use the Apple backend sources under `backends/apple/Sources/WaterUI`.

## Core Infrastructure

| Rust Feature | Swift Status | Notes |
| --- | --- | --- |
| `waterui_init` / environment lifecycle | ✅ complete | `WaterUI.App` owns `WuiEnvironment` and disposes it in `deinit`. |
| `WuiAnyView` rendering | ✅ complete | `WuiAnyView` rebuilds via `Render.main` and can fall back to body evaluation. |
| `WuiAnyView` type ID / force cast helpers | ✅ complete | Implemented through `Render` registry and `forceAs` helpers in `AnyView.swift`. |
| Reactive `Binding<T>` (bool, int, double, string) | ✅ complete | Implemented in `Reactive/Binding.swift` with watcher plumbing. |
| Reactive `Computed<T>` (color/font/string/etc.) | ✅ complete | `Reactive/Computed.swift` covers the exported computed FFI functions. |
| Watcher metadata + animation bridge | ✅ complete | `Animation.swift` and `Watcher.swift` translate `WuiWatcherMetadata` into SwiftUI animations. |
| Dynamic view (`waterui_dynamic_*`) | ✅ complete | `Core/Dynamic.swift` registers callbacks and rebuilds child views. |

## Layout

| Rust Feature | Swift Status | Notes |
| --- | --- | --- |
| `waterui_container` / arbitrary layout trait objects | ✅ complete | `Layout/Layout.swift` (`WuiContainer` + `RustLayout`) mirrors the three-pass propose/measure/place flow. |
| Spacer component | ✅ complete | `Layout/Spacer.swift`. |
| Scroll view | ✅ complete | `Layout/Scroll.swift`. |
| Stack/grid/overlay/padding helpers | ❌ missing | Awaiting FFI support; registry comments mark them as TODO. |
| For-each / lazy collections | ⚠️ partial | `WuiAnyViewArray` transfers child buffers, but no higher-level `ForEach` wrapper exists yet. |

## Text & Styling

| Rust Feature | Swift Status | Notes |
| --- | --- | --- |
| Plain text (`waterui_text`) | ✅ complete | `Text.swift` renders `WuiStyledStr` with resolved fonts. |
| Label (`waterui_label`) | ✅ complete | `Label.swift`. |
| Styled strings (foreground/background/underline etc.) | ⚠️ partial | Font + underline/strikethrough are respected; computed foreground/background colours ignored pending palette support. |
| Localised / locale-aware text | ❌ missing | No Swift counterpart for `components/text/src/locale.rs`. |

## Controls

| Rust Feature | Swift Status | Notes |
| --- | --- | --- |
| Button (`waterui_button`) | ✅ complete | `Button.swift` handles label/action. |
| Progress view (`waterui_progress`) | ✅ complete | Linear and circular variants map to SwiftUI styles. |
| Text field (`waterui_text_field`) | ✅ complete | `Form/TextField.swift` keeps binding in sync and forwards keyboard type. |
| Toggle (`waterui_toggle`) | ✅ complete | `Form/Toggle.swift`. |
| Slider (`waterui_slider`) | ✅ complete | `Form/Slider.swift` with value, range, labels. |
| Stepper (`waterui_stepper`) | ⚠️ partial | UI present, but computed `step` binding currently unused. |
| Picker / colour picker (`waterui_picker*`) | ❌ missing | FFI bindings exist; Swift views not implemented yet. |
| Form metadata helpers (validation, etc.) | ❌ missing | No Swift representation. |

## Navigation & Presentation

| Rust Feature | Swift Status | Notes |
| --- | --- | --- |
| Navigation view / link | ❌ missing | Prototype commented out in `Navigation/Navigation.swift`; awaiting FFI updates. |
| Search field / navigation chrome | ❌ missing | No Swift counterpart yet. |
| Alerts, overlays, sheets | ❌ missing | `Alert.swift` placeholder only. |

## Media & Graphics

| Rust Feature | Swift Status | Notes |
| --- | --- | --- |
| Static colour view (`waterui_color`) | ⚠️ partial | `Color.swift` resolves computed colours but lacks shared colour binding support. |
| Image / video / live photo | ❌ missing | No Swift implementations yet. |
| Canvas / graphics contexts | ⚠️ partial | `Graphics/RendererView.swift` renders CPU surfaces; GPU path pending. |

## Miscellaneous

| Rust Feature | Swift Status | Notes |
| --- | --- | --- |
| Lazy / memoised components (`lazy`, `with_env`) | ❌ missing | `WuiAnyViewArray` placeholders note TODO. |
| Hot reload support | ⚠️ partial | Hooks exist in Rust (`waterui_main_reloadble`) but Swift uses direct `waterui_main` call. |
| Gesture / accessibility APIs | ❌ missing | No surface in Swift layer. |

## Summary

The Apple backend currently covers the core layout loop, dynamic rendering, and primary controls exposed by the Rust crates. Work remaining focuses on richer controls (picker, navigation, media) and filling gaps around styling, layout utilities, and hot-reload integrations.
