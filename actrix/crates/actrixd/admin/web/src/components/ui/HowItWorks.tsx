import type { ReactNode } from "react";
import { CollapsibleCard } from "./CollapsibleCard";

export function HowItWorks({
  storageKey,
  children,
}: {
  storageKey: string;
  children: ReactNode;
}) {
  return (
    <CollapsibleCard storageKey={`howit_${storageKey}`} title="How it works">
      {children}
    </CollapsibleCard>
  );
}
