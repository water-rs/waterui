// Type surface for i18n.js — localized text helpers.

import type { Accessor } from "./signals.js";

/** A primitive value interpolated into a localized text template. */
export type TextArgValue = string | number | bigint;

/** A static or reactive interpolation argument for a localized text template. */
export type TextArg = TextArgValue | (() => TextArgValue) | Accessor<TextArgValue>;

/**
 * A localized text entry resolved through the active translation catalog.
 *
 * Produced by the JSX transform for literal / interpolated text children and by
 * {@link t}. Plain runtime strings are verbatim and never looked up.
 */
export interface LocalizedText {
  readonly key: string;
  readonly args?: Readonly<Record<string, TextArg>>;
}

/**
 * Marks a template literal or key string for translation catalog lookup.
 */
export declare function t(
  strings: TemplateStringsArray,
  ...values: TextArg[]
): LocalizedText;
export declare function t(
  key: string,
  args?: Readonly<Record<string, TextArg>>,
): LocalizedText;
