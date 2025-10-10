# WaterUI Web Backend

This crate hosts the experimental WebAssembly backend for WaterUI. The goal is to provide a
browser-native renderer that consumes `waterui-core` view trees and produces HTML + CSS updates.
The current implementation focuses on scaffolding:

- ✅ Rust crate configured for `wasm32-unknown-unknown`
- ✅ DOM bootstrapping with automatic stylesheet injection
- ✅ Pretty default styling using standard web components (buttons, headings, paragraphs)
- ✅ `WebApp` entry point mirrored after the Swift backend's modular design
- 🔄 TODO: dispatcher registrations that translate `AnyView` nodes into DOM elements (current handlers end with `todo!()`)
- 🔄 TODO: event propagation and reactive state reconciliation

## Building

Ensure the `wasm32-unknown-unknown` target is installed:

```bash
rustup target add wasm32-unknown-unknown
```

You can build the crate directly using Cargo:

```bash
cargo build -p waterui-web --target wasm32-unknown-unknown
```

For browser integration with bundlers, `wasm-pack` offers a convenient workflow:

```bash
wasm-pack build backends/web --target web
```

## Usage

The generated WASM module exposes a `WebApp` type. From JavaScript you can mount the application
like so:

```javascript
import init, { WebApp } from "./pkg/waterui_web.js";

async function bootstrap() {
  await init();
  const app = new WebApp();
  await app.wasm_mount();
}

bootstrap();
```

By default the renderer will create a host `<div>` when no root id is supplied. To mount into an
existing element, call `WebAppBuilder::new().with_root_id("app-root")` from Rust before exporting
the instance.

## Project Layout

- `src/app.rs`: Entry point and builder responsible for managing the `Environment` and renderer.
- `src/dom.rs`: Thin wrappers around `web_sys` to simplify DOM mutation and stylesheet injection.
- `src/renderer.rs`: Dispatcher-driven rendering surface that currently ends with explicit
  `todo!()` placeholders for each view type.
- `styles/default.css`: Standalone stylesheet bundled via `include_str!` and injected during
  mounting to provide a pleasant baseline for native web controls.
- `IMPLEMENTATION_STATUS.md`: Detailed tracker mirroring other backends and documenting which
  dispatcher routes still need to be implemented.

Please update this document as the backend evolves (e.g., once the dispatcher can render core
widgets and browser events feed back into WaterUI's fine-grained reactivity).
