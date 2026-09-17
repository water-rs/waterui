// The global object the Rust bridge reads after it evaluates a bundle.
//
// A bundle is a classic script: nothing it declares is reachable from Rust,
// because the engine hands back only the completion value. The CLI-generated
// bundle entry therefore ends with one call — `installRuntimeGlobal(modules)` —
// and everything the bridge needs is on `globalThis.__waterui_runtime` from
// that moment on: the reactive helpers it drives the seam with, the host
// installer, `mount`, and the module table.
//
// The Rust side reads each entry by name and refuses a bundle that is missing
// one, so this file and `waterui-ts`'s `RuntimeGlobal` are two halves of the
// same contract; HOST.md spells it out.

import { createMemo, createSignal, isAccessor, isSignal } from "./signals.js";
import {
  getHost,
  installHost,
  read,
  subscribe,
  toAccessor,
  toSignal,
  uninstallHost,
  write,
} from "./host.js";
import { mount } from "./contexts.js";

/** Where the runtime object is published. */
const RUNTIME_GLOBAL = "__waterui_runtime";

/**
 * Wraps a Rust callback as a plain JavaScript function.
 *
 * The bridge keeps Rust closures in a registry keyed by `id` — a signal
 * subscription, a prop callback — and hands JavaScript this wrapper instead of
 * the closure itself, because a host function can only be registered under a
 * name, never passed as a value. Calling the wrapper crosses into
 * `invoke(id, …args)`, which dispatches on the id.
 *
 * The `invoke` it calls is the installed host's, captured here once. It is
 * deliberately not `globalThis.__waterui_host.invoke`: that global is
 * writable, and a bundle that reassigns the entry while it evaluates — before
 * any callback is made — would otherwise be what every wrapper dispatches
 * through. The host table carries the function the engine registered.
 *
 * @param {number} id - The registry id the bridge assigned to the callback.
 * @returns {(...args: unknown[]) => unknown}
 */
export function makeCallback(id) {
  if (typeof id !== "number") {
    throw new TypeError("makeCallback(id) expects the numeric id the bridge assigned");
  }
  const { invoke } = getHost();
  return (...args) => invoke(id, ...args);
}

/**
 * Publishes the runtime on `globalThis.__waterui_runtime` and returns it.
 *
 * `modules` maps a module id — the path `tsx!` names, as the bundler spells
 * it — to that module's default export. The bridge looks a mounted module up
 * there; an id the table does not carry is a typed error on the Rust side.
 *
 * `contracts` maps the same ids to the props contract hash each module was
 * built against, as hexadecimal text. Mounting refuses a module whose hash
 * differs from the one the binary's props type carries, which is what stops a
 * bundle from one build being handed props shaped by another. The hash is
 * text rather than a number because it is 64 bits wide and a JavaScript
 * number holds only 53 of them exactly. An omitted table is an empty one: no
 * module can be mounted from that bundle, and the Rust side says so naming
 * the module rather than failing here for a bundle nothing may mount yet.
 *
 * One context evaluates one bundle, so installing twice replaces the table
 * wholesale. That is not a way to swap bundles at runtime: the host
 * installation guards against it (`installHost` throws on a second install),
 * and an update takes effect at the next launch with a fresh context.
 *
 * @param {Record<string, unknown>} modules
 * @param {Record<string, string>} [contracts]
 * @returns {Record<string, unknown>} The published runtime object.
 */
export function installRuntimeGlobal(modules, contracts = {}) {
  if (modules === null || typeof modules !== "object") {
    throw new TypeError(
      "installRuntimeGlobal(modules) expects an object mapping each module id to its default export",
    );
  }
  if (contracts === null || typeof contracts !== "object") {
    throw new TypeError(
      "installRuntimeGlobal(modules, contracts) expects an object mapping each module id to the hexadecimal props contract hash it was built against",
    );
  }
  const runtime = Object.freeze({
    installHost,
    uninstallHost,
    mount,
    isSignal,
    isAccessor,
    read,
    write,
    subscribe,
    toSignal,
    toAccessor,
    createSignal,
    createMemo,
    makeCallback,
    modules,
    contracts,
  });
  globalThis[RUNTIME_GLOBAL] = runtime;
  return runtime;
}
