// Type surface for contexts.js — the built-in environment contexts.

import type { Accessor } from "./signals.js";
import type { Element } from "./jsx-runtime.js";
import type { Handle } from "./host.js";

export interface Color {
  /** Linear-light red, green and blue; outside 0–1 for wide-gamut colors. */
  red: number;
  green: number;
  blue: number;
  /** Extended-range headroom: above 1 the color is HDR. */
  headroom: number;
  opacity: number;
}

export interface Theme {
  colorScheme: "light" | "dark";
  /**
   * The theme's color tokens, keyed by slot name (`foreground`,
   * `background`, `surface`, `accent`, …). A slot the environment does not
   * install is absent rather than defaulted.
   */
  [token: string]: unknown;
}

export interface Locale {
  identifier: string;
  /** BCP-47 language tag, e.g. `"en"`, `"zh-Hans"`. */
  languageCode: string;
  textDirection: "ltr" | "rtl";
}

/**
 * The ambient theme. Read-only: the value is owned by the host's environment
 * (it may be a memo), so it is declared as an `Accessor`. Only available
 * inside a mounted tree.
 */
export declare function useTheme(): Accessor<Theme>;

/** The ambient locale. Read-only like `useTheme`; host-owned. */
export declare function useLocale(): Accessor<Locale>;

export interface Mounted {
  handle: Handle;
  dispose(): void;
}

/** The engine-side module entry: seeds the environment and runs `render`. */
export declare function mount(render: () => Element): Mounted;
