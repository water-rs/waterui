// The entry of the bundle `tests/mount.rs` loads: one application module and
// the `installRuntimeGlobal` call that publishes it.
//
// This is `promo.tsx` beside it, written the way the reactive JSX transform
// emits it — `jsx(name, props)` with the children in a `children` property —
// because the transform is the CLI's (water-rs/waterui#1048) and is not built
// here. Everything below it is the real library, bundled in, so what the Rust
// side drives is a whole application bundle rather than a fixture that
// imitates one.
//
// The contract table comes from `globalThis.__waterui_test_contracts`, which
// the Rust test sets from the props type's own `CONTRACT_HASH` before it
// loads the bundle. That stands in for the CLI, which reads the same constant
// out of the compiled artifact's `waterui_meta_tsprops_*` static and writes it
// into the entry it generates.

import { jsx } from "../../src/js/jsx-runtime.js";
import { installRuntimeGlobal } from "../../src/js/runtime-global.js";
import { onCleanup } from "../../src/js/signals.js";

/** The id `tsx!("fixtures/promo.tsx", …)` in `tests/mount.rs` resolves to. */
const MODULE = "tests/fixtures/promo.tsx";

function Promo(props) {
  // How the Rust side sees that the mount was torn down: the root scope's
  // cleanup runs when `dispose` is called, and the mount guard calls it when
  // the view is dropped.
  globalThis.__waterui_test_disposals = 0;
  onCleanup(() => {
    globalThis.__waterui_test_disposals += 1;
  });
  return jsx("VStack", {
    spacing: 8,
    children: [
      jsx("Text", { children: props.headline }),
      jsx("Text", { children: () => `${props.unread()} unread` }),
      jsx("Button", { onTap: props.onDismiss, children: "Dismiss" }),
    ],
  });
}

installRuntimeGlobal({ [MODULE]: Promo }, globalThis.__waterui_test_contracts);
