import type { ReactNode } from "react";

export type TagTone = "buy" | "sell" | "neutral" | "warn";

/** Small pill. Meaning is always in the text, never colour alone. */
export function Tag({ tone = "neutral", children }: { tone?: TagTone; children: ReactNode }) {
  return <span className={`ui-tag ui-tag-${tone}`}>{children}</span>;
}
