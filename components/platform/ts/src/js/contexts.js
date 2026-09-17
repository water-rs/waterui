// The built-in environment contexts.
//
// Theme and locale are framework-owned environment values. The host provides
// them at mount through `environment()`; `mount` materializes each through
// `toSignal` — seeded from the host's `read()` and kept live through the
// host's `subscribe()` — and publishes them on the root owner, where
// `useContext` resolves them for the whole tree. They are read-only accessors
// from the JS side: writes live on the host.
//
// Safe-area insets are deliberately not among them. WaterUI publishes no
// ambient inset value: a backend places content clear of the hardware at the
// container level — a stack lays its children out inside the safe area and
// extends the scroll surfaces and chrome containers that touch its edges — so
// neither the framework nor a view ever reads an inset number. A
// `useSafeArea()` that could only ever answer zeroes would fake a primitive
// that does not exist, so the asymmetry is documented instead.

import { getHost, toSignal } from "./host.js";
import { createContext, createRoot, getOwner, useContext } from "./signals.js";

const ThemeContext = createContext();
const LocaleContext = createContext();

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
    owner.contexts = contexts;
    return { handle: render(), dispose };
  });
}
