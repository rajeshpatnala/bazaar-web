import { useQuery } from "@tanstack/react-query";
import type { components } from "../api/schema";
import { statusQuery } from "../api/queries";
import { formatIstIso } from "../shared/lib/format-ist";

type Mode = components["schemas"]["ModeDto"];

const LABEL: Record<Mode, string> = { paper: "PAPER", semi_auto: "SEMI-AUTO", auto: "AUTO" };

/** Mode badge, a read-only live note and the server time. Nothing here can change the mode. */
export function ModeBadge() {
  const { data } = useQuery(statusQuery);
  if (!data) return null;
  return (
    <div className="shell-status">
      <span className={`mode-badge mode-${data.mode}`} data-mode={data.mode}>
        {LABEL[data.mode]}
      </span>
      <span className="lbl">Live: {data.live_enabled ? "on" : "off"} (set on the server)</span>
      <time className="lbl" dateTime={data.server_time}>
        {formatIstIso(data.server_time)}
      </time>
    </div>
  );
}
