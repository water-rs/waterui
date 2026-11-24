// Undocumented `waterui/runtime` entry for the bundle prologue and host
// bootstrap. Application views import only from `waterui` (`index.js`).

export {
  getHost,
  installHost,
  read,
  subscribe,
  toAccessor,
  toSignal,
  uninstallHost,
  write,
} from "./host.js";
export { mount } from "./contexts.js";
export { installRuntimeGlobal, makeCallback } from "./runtime-global.js";
