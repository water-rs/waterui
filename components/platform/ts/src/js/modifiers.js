// The modifier attribute table.
//
// JSX attribute names listed here are *modifiers*: they apply to the element
// in written order through `host.modify`, exactly like a Rust modifier chain.
// Every other attribute is component configuration passed to `host.create`
// and does not participate in ordering.
//
// This seed covers the view-level modifiers the framework exposes today. The
// authoritative table is generated from the component catalog (#670) — when
// that lands, regenerate this file rather than editing it by hand.

export const MODIFIER_NAMES = new Set([
  "accessibilityLabel",
  "accessibilityValue",
  "aspectRatio",
  "background",
  "blur",
  "bold",
  "border",
  "clipped",
  "cornerRadius",
  "disabled",
  "font",
  "foreground",
  "frame",
  "hidden",
  "italic",
  "offset",
  "opacity",
  "overlay",
  "padding",
  "safeArea",
  "shadow",
]);
