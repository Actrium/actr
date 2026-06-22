import type { ReactNode } from "react";
import { CollapsibleCard } from "./CollapsibleCard";

export function HowItWorks({
  storageKey,
  children,
  defaultExpanded = true,
}: {
  storageKey: string;
  children: ReactNode;
  defaultExpanded?: boolean;
}) {
  return (
    <CollapsibleCard
      storageKey={`howit_${storageKey}`}
      title="How it works"
      defaultExpanded={defaultExpanded}
    >
      {children}
    </CollapsibleCard>
  );
}
