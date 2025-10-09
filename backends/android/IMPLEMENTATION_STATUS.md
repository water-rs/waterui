# Android Backend Implementation Status

This document compares the Jetpack Compose backend with the functionality exported by the WaterUI Rust crates. Status codes:

- **✅ complete** – Feature behaves equivalently to the Swift/Rust implementation.
- **⚠️ partial** – Implemented but missing behaviour or polish.
- **❌ missing** – Not wired up yet.

## Core Runtime

| Rust Feature | Android Status | Notes |
| --- | --- | --- |
| `waterui_init` / environment lifecycle | ✅ complete | `WaterUIApplication` initialises/drops `WuiEnvironment`; Gradle toolchain set to JDK 17 for Kotlin compatibility. |
| `WuiAnyView` render loop | ⚠️ partial | Registry-driven renderers exist, but fallback body traversal lacks error diagnostics and doesn’t handle nested updates yet. |
| `RenderRegistry` overrides | ✅ complete | Functional renderer map replaces Swift-style class tree. |
| `NativePointer` lifecycle helpers | ✅ complete | Guard classes drop native handles safely. |
| Dynamic view (`waterui_dynamic_*`) | ⚠️ partial | Wrapper exists but JNI watcher hookups are TODO, so content never updates after initial render. |

## Layout

| Rust Feature | Android Status | Notes |
| --- | --- | --- |
| Container layout (`waterui_container`) | ⚠️ partial | Rust propose/measure/place is hooked up; child iteration uses raw pointers but lacks diffing/priorities. |
| Spacer (`waterui_spacer`) | ✅ complete | Renders Compose `Spacer`. |
| Scroll view | ⚠️ partial | Renders single child in `LazyColumn`; axis selection ignored. |
| Stack/overlay/padding helpers | ❌ missing | No Compose counterparts yet. |

## Text & Styling

| Rust Feature | Android Status | Notes |
| --- | --- | --- |
| Styled text (`waterui_text`) | ⚠️ partial | Displays placeholder string; styled spans unresolved. |
| Label (`waterui_label`) | ✅ complete | Decodes UTF-8 label bytes. |
| Colour view (`waterui_color`) | ⚠️ partial | Always uses magenta placeholder; ignores resolved colours. |

## Controls

| Rust Feature | Android Status | Notes |
| --- | --- | --- |
| Button (`waterui_button`) | ✅ complete | Calls native action on click. |
| Progress view (`waterui_progress`) | ✅ complete | Maps style flag to Compose linear/circular indicators. |
| Text field (`waterui_text_field`) | ⚠️ partial | Two-way binding works, but prompt/label use placeholders; keyboard types ignored. |
| Toggle (`waterui_toggle`) | ⚠️ partial | Binding updates Compose state; label rendered, but watcher lifetimes rely on TODO trampoline. |
| Slider (`waterui_slider`) | ⚠️ partial | Value binding works; range/labels render, watchers use stubbed factories. |
| Stepper (`waterui_stepper`) | ⚠️ partial | Displays increment button but ignores actual binding and range. |
| Picker / colour picker | ❌ missing | No JNI bindings or Compose UI yet. |

## Reactive System

| Rust Feature | Android Status | Notes |
| --- | --- | --- |
| Bindings (bool/int/double/string) | ⚠️ partial | Wrapper types defined, but JNI watcher factories are stubbed so updates never fire. |
| Computed values | ⚠️ partial | Shares binding implementation; lacks dedicated read/watch JNI functions. |
| Animation metadata (`waterui_get_animation`) | ❌ missing | JNI function declared but not wired to Compose animation APIs. |

## Navigation, Media & Advanced Components

| Rust Feature | Android Status | Notes |
| --- | --- | --- |
| Navigation view/link/search | ❌ missing | No Compose implementation yet. |
| Lists/Lazy collections (`for_each`) | ❌ missing | Container provides raw child list but no higher-level list abstractions. |
| Media components (image, video, live photo) | ❌ missing | Placeholders not yet added. |
| Graphics/canvas APIs | ❌ missing | Awaiting future mapping. |

## Tooling & Build

| Topic | Status | Notes |
| --- | --- | --- |
| Gradle module | ✅ complete | `settings.gradle.kts` includes `:backends:android`; wrapper pinned to Gradle 8.7. |
| Android SDK integration | ✅ complete | `local.properties` expected to point at SDK; `.gitignore` updated accordingly. |
| Compose compiler alignment | ✅ complete | Uses Kotlin 1.9.22 with compiler extension 1.5.10. |

## Summary

The Android backend currently mirrors the layout pipeline and basic controls but many renderers still ship placeholder UI and watcher stubs. Completing watcher trampolines, styled text, navigation, pickers, and media components will bring parity with the Swift implementation and the underlying Rust crates.
