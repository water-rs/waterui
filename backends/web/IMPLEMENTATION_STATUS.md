# Web Backend Implementation Status

This document tracks how the experimental web backend maps the Rust APIs exported by
WaterUI onto browser primitives. Status values:

- **✅ complete** – Behaviour matches the Rust contract in day-to-day scenarios.
- **⚠️ partial** – Some scaffolding exists, but important behaviour is stubbed or missing.
- **❌ missing** – No meaningful implementation yet.

> File references use the sources under `backends/web/src` unless noted otherwise.

## Core Infrastructure

| Rust Feature | Web Status | Notes |
| --- | --- | --- |
| Workspace registration / cargo integration | ✅ complete | `Cargo.toml` exposes the crate with optional `wasm32` tooling. |
| DOM bootstrap (`waterui_app` equivalent) | ⚠️ partial | `WebApp` locates a root element, injects default styles, and mounts a surface, but dispatcher handlers still terminate in `todo!()`. |
| Environment lifecycle (`Environment` ownership) | ⚠️ partial | `WebApp` owns an `Environment`, yet no browser-specific resources are attached or torn down. |
| `AnyView` rendering entry point | ❌ missing | `WebRenderer::render` forwards to a dispatcher whose first handler still calls `todo!("Render text views")`. |
| Fine-grained reactivity dispatcher | ⚠️ partial | The dispatcher from `waterui_render_utils` is registered, but only a text route stub exists. |

## Layout

| Rust Feature | Web Status | Notes |
| --- | --- | --- |
| Container/layout trait bridging | ❌ missing | No DOM translation for layout trait objects yet. |
| Scroll/stack/grid helpers | ❌ missing | Awaiting dispatcher registrations. |
| Spacer / padding utilities | ❌ missing | Not connected to DOM elements. |

## Text & Styling

| Rust Feature | Web Status | Notes |
| --- | --- | --- |
| Plain text (`waterui_text`) | ❌ missing | Dispatcher TODO prevents text nodes from being created. |
| Styled strings | ❌ missing | No mapping for `StyledStr` runs. |
| Theming / palette bindings | ❌ missing | Styling limited to embedded CSS with no reactive updates. |

## Controls

| Rust Feature | Web Status | Notes |
| --- | --- | --- |
| Button (`waterui_button`) | ❌ missing | No DOM translation has been implemented; dispatcher registration remains a `todo!()`. |
| Input controls (text field, toggle, slider, etc.) | ❌ missing | No DOM widgets registered. |
| Collection / dynamic views | ❌ missing | Dispatcher has no array or dynamic handlers. |

## Navigation & Presentation

| Rust Feature | Web Status | Notes |
| --- | --- | --- |
| Navigation stack / links | ❌ missing | No browser navigation integration. |
| Overlays / dialogs | ❌ missing | Awaiting design. |
| Alerts / sheets | ❌ missing | Not implemented. |

## Media & Graphics

| Rust Feature | Web Status | Notes |
| --- | --- | --- |
| Color view (`waterui_color`) | ❌ missing | CSS variables not wired. |
| Image / video surfaces | ❌ missing | No HTML media elements registered. |
| Canvas / drawing APIs | ❌ missing | No `canvas` integration. |

## Miscellaneous

| Rust Feature | Web Status | Notes |
| --- | --- | --- |
| Asset pipeline (CSS/JS bundling) | ⚠️ partial | `styles/default.css` is embedded at compile time; external asset story undefined. |
| Testing (`wasm-bindgen-test`) | ❌ missing | No automated tests configured. |
| Error reporting / logging | ⚠️ partial | `WebError` captures basic DOM failures, but there is no logging bridge. |

## Summary

The web backend currently provides mounting scaffolding, DOM helpers, and a dispatcher
hook, but all real rendering work is still TODO. The next milestones involve teaching the
dispatcher how to translate core views (text, button, layout containers) into real DOM
nodes and wiring browser events into WaterUI's fine-grained reactivity model.
