// The public `waterui` authoring module surface.
//
// Application views import only from `waterui` (`index.js`) and
// `waterui/jsx-runtime` (`jsx-runtime.js`). Host installation, mounting, and
// bundle prologue plumbing live in the separate `waterui/runtime` entry
// (`runtime.js`).

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

export { Box, For, Show } from "./components.js";
export { useLocale, useTheme } from "./contexts.js";
export { Fragment, jsx, jsxDEV, jsxs, spreadProps } from "./jsx-runtime.js";
