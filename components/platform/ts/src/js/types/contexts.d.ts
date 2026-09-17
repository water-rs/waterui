// Type surface for contexts.js — the built-in environment contexts.

import type { Signal } from "./signals.js";
import type { Element } from "./jsx-runtime.js";
import type { Handle } from "./host.js";

export interface Theme {
  colorScheme: "light" | "dark";
  /** Theme token colors keyed by slot name; see the framework theme docs. */
  [token: string]: unknown;
}

export interface Locale {
  identifier: string;
  /** BCP-47 language tag, e.g. `"en"`, `"zh-Hans"`. */
  languageCode: string;
  textDirection: "ltr" | "rtl";
}

export interface SafeArea {
  top: number;
  bottom: number;
  leading: number;
  trailing: number;
}

/** The ambient theme signal. Only available inside a mounted tree. */
export declare function useTheme(): Signal<Theme>;

/** The ambient locale signal. Only available inside a mounted tree. */
export declare function useLocale(): Signal<Locale>;

/** The ambient safe-area insets signal. Only available inside a mounted tree. */
export declare function useSafeArea(): Signal<SafeArea>;

export interface Mounted {
  handle: Handle;
  dispose(): void;
}

/** The engine-side module entry: seeds the environment and runs `render`. */
export declare function mount(render: () => Element): Mounted;
