// The public `waterui` module surface. The compile-time transform resolves
// `import … from "waterui"` to this module and `waterui/jsx-runtime` to
// `jsx-runtime.js`; the engine injects both as virtual modules over the host
// table installed through `installHost`.

export {
  batch,
  createContext,
  createEffect,
  createMemo,
  createRoot,
  createScope,
  createSignal,
  createStore,
  getOwner,
  isAccessor,
  isMemo,
  isSignal,
  onCleanup,
  runWithOwner,
  untrack,
  useContext,
} from "./signals.js";

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

export { Box, For, Show, Suspense } from "./components.js";
export { mount, useLocale, useSafeArea, useTheme } from "./contexts.js";
export { Fragment, jsx, jsxDEV, jsxs, spreadProps } from "./jsx-runtime.js";
