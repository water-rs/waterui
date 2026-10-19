// The TypeScript module `tests/mount.rs` mounts, and the file `tsx!` names.
//
// This is the source; `mount.entry.js` beside it is the same module written
// the way the reactive JSX transform emits it, and `mount.js` is that entry
// bundled. The transform and the bundler are the CLI's (water-rs/waterui#1048)
// and are not built here, so the two forms are kept side by side: this one is
// what an application author writes, and the one the Rust test loads is what
// the toolchain would hand the engine.

import { Button, Text, VStack } from "waterui";
import type { Signal } from "waterui";

export interface PromoProps {
  headline: string;
  unread: Signal<number>;
  onDismiss: () => void;
}

export default function Promo({ headline, unread, onDismiss }: PromoProps) {
  return (
    <VStack spacing={8}>
      <Text>{headline}</Text>
      <Text>{() => `${unread()} unread`}</Text>
      <Button onTap={onDismiss}>Dismiss</Button>
    </VStack>
  );
}
