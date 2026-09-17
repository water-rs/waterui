// The built-in environment contexts.
//
// Theme, locale, and safe area are framework-owned environment values. The
// host provides them at mount through `environment()`; `mount` materializes
// each through `toSignal` — seeded from the host's `read()` and kept live
// through the host's `subscribe()` — and publishes them on the root owner,
// where `useContext` resolves them for the whole tree. They are read-only
// accessors from the JS side: writes live on the host.

import { getHost, toSignal } from "./host.js";
import { createContext, createRoot, getOwner, useContext } from "./signals.js";

const ThemeContext = createContext();
const LocaleContext = createContext();
const SafeAreaContext = createContext();

function required(context, name) {
  const value = useContext(context);
  if (value === undefined) {
    throw new Error(`${name}() is only available inside a mounted WaterUI tree`);
  }
  return value;
}

/** The ambient theme: color scheme and theme tokens. */
export function useTheme() {
  return required(ThemeContext, "useTheme");
}

/** The ambient locale. */
export function useLocale() {
  return required(LocaleContext, "useLocale");
}

/** The ambient safe-area insets. */
export function useSafeArea() {
  return required(SafeAreaContext, "useSafeArea");
}

/**
 * Mounts a TS module: creates a detached root scope, seeds the environment
 * contexts from `host.environment()`, runs `render`, and returns
 * `{ handle, dispose }`. The engine calls this once per mounted module;
 * `dispose` tears the whole tree down.
 */
export function mount(render) {
  const host = getHost();
  const environment = host.environment();
  return createRoot((dispose) => {
    const owner = getOwner();
    const contexts = new Map();
    contexts.set(ThemeContext.id, toSignal(environment.theme));
    contexts.set(LocaleContext.id, toSignal(environment.locale));
    contexts.set(SafeAreaContext.id, toSignal(environment.safeArea));
    owner.contexts = contexts;
    return { handle: render(), dispose };
  });
}