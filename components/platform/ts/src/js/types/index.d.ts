// Type surface for index.js — the public `waterui` authoring module.

export * from "./signals.js";
export * from "./components.js";
export { useLocale, useTheme } from "./contexts.js";
export type { Color, Locale, Theme } from "./contexts.js";
export { t } from "./i18n.js";
export type { LocalizedText, TextArg, TextArgValue } from "./i18n.js";
export { Fragment, jsx, jsxDEV, jsxs, spreadProps } from "./jsx-runtime.js";
export type { Element } from "./jsx-runtime.js";
