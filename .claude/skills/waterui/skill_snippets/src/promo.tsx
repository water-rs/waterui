// The module the `tsx!` snippet in SKILL.md § "TypeScript views" mounts.
// `tsx!("./promo.tsx", …)` resolves the literal against `ref_ts.rs` and stats
// the file at expansion, so the transcription needs a real module to name —
// this one, which is the same shape as the guide's example.

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
