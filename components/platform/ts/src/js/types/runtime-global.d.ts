// Type surface for runtime-global.js — the object the Rust bridge reads after
// evaluating a bundle. See HOST.md.

/**
 * Wraps a Rust callback, registered under `id`, as a plain JavaScript
 * function. Calling it crosses into the installed host's `invoke(id, …args)`
 * — the entry captured when the table was installed, never a property read
 * off `globalThis.__waterui_host` at call time.
 */
export declare function makeCallback(id: number): (...args: unknown[]) => unknown;

/**
 * Publishes the runtime on `globalThis.__waterui_runtime` and returns it.
 * `modules` maps each module id to that module's default export; the bundle
 * entry the CLI generates ends with this one call.
 */
export declare function installRuntimeGlobal(
  modules: Record<string, unknown>,
): Record<string, unknown>;
