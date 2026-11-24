// The entry the test library bundle is built from: the public `waterui`
// authoring module on `globalThis.waterui` and the internal `waterui/runtime`
// plumbing on `globalThis.wateruiRuntime`.
//
// An application bundle ends with `installRuntimeGlobal(modules)`, imported
// from `waterui/runtime` by the CLI. A test writes that call itself, against
// the same library, so what the Rust side drives is the library as an
// application gets it.

import * as waterui from "../../src/js/index.js";
import * as wateruiRuntime from "../../src/js/runtime.js";

globalThis.waterui = waterui;
globalThis.wateruiRuntime = wateruiRuntime;
