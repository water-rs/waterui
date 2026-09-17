// The entry the test library bundle is built from: every public export of the
// `waterui` module, published on one global.
//
// An application bundle ends with `installRuntimeGlobal(modules)`, which the
// CLI generates. A test writes that call itself, against the same library, so
// what the Rust side drives is the library as an application gets it.

import * as waterui from "../../src/js/index.js";

globalThis.waterui = waterui;
