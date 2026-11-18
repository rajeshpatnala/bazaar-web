import { useQuery } from "@tanstack/react-query";
import type { components } from "../api/schema";
import { statusQuery } from "../api/queries";

type Mode = components["schemas"]["ModeDto"];

const LABEL: Record<Mode, string> = { paper: "PAPER", semi_auto: "SEMI-AUTO", auto: "AUTO" };

export function ModeBadge() {
  const { data } = useQuery(statusQuery);
  if (!data) return null;
  return (
    <span className={`mode-badge mode-${data.mode}`} data-mode={data.mode}>
      {LABEL[data.mode]}
    </span>
  );
}
