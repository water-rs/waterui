// Localized text construction for WaterUI TypeScript views.
//
// Runtime JS strings are always verbatim so user data never reaches the
// translation catalog. Only literal JSX text (which the compile-time transform
// emits as `{ key }` or `{ key, args }`) and explicit `t` calls perform
// catalog lookups.

/**
 * Marks text for translation catalog lookup, with optional named or positional
 * interpolation arguments.
 *
 * Accepts either a tagged template literal:
 *
 * ```ts
 * t`Settings`                   // { key: "Settings" }
 * t`Order ${id}`                // { key: "Order {0}", args: { "0": id } }
 * ```
 *
 * or an explicit key and named arguments map:
 *
 * ```ts
 * t("Hello, {name}", { name })  // { key: "Hello, {name}", args: { name } }
 * ```
 *
 * Argument values may be plain strings/numbers/bigints or reactive accessors
 * (`() => T` / `Signal<T>`), which the host subscribes to directly.
 */
export function t(stringsOrKey, ...values) {
  if (typeof stringsOrKey === "string") {
    if (values.length > 1) {
      throw new TypeError("t(key, args?) takes at most two arguments");
    }
    const args = values[0];
    if (args === undefined) {
      return { key: stringsOrKey };
    }
    if (args === null || typeof args !== "object" || Array.isArray(args)) {
      throw new TypeError("t(key, args) expects an arguments object");
    }
    return { key: stringsOrKey, args };
  }

  if (!Array.isArray(stringsOrKey) || stringsOrKey.length === 0) {
    throw new TypeError("t must be called as a tagged template or with a key string");
  }
  if (typeof stringsOrKey[0] !== "string") {
    throw new TypeError("t template segments must be strings");
  }
  if (values.length === 0) {
    return { key: stringsOrKey[0] };
  }

  let key = stringsOrKey[0];
  const args = {};
  for (let i = 0; i < values.length; i += 1) {
    const slot = String(i);
    const segment = stringsOrKey[i + 1];
    if (typeof segment !== "string") {
      throw new TypeError("t template segments must be strings");
    }
    key += `{${slot}}${segment}`;
    args[slot] = values[i];
  }
  return { key, args };
}
