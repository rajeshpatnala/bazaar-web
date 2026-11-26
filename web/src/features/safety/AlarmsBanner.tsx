import { useQuery } from "@tanstack/react-query";
import { formatIstIso } from "../../shared/lib/format-ist";
import { alarmsQuery } from "./queries";

function since(iso: string): string {
  try {
    return formatIstIso(iso);
  } catch {
    return "time unavailable";
  }
}

/** Server alarms. An unreadable list is said out loud and never shown as "no alarms". */
export function AlarmsBanner() {
  const query = useQuery(alarmsQuery);
  const alarms = query.data ?? [];
  if (!query.isError && alarms.length === 0) return null;
  return (
    <div role="alert" className="safety-alarms">
      {query.isError && (
        <p>
          <strong>Alarms unknown.</strong> The alarm list cannot be read, so do not assume all is well.
          {alarms.length > 0 && " Below is the last list that was read; it may be out of date."}
        </p>
      )}
      {alarms.length > 0 && (
        <>
          <p>
            <strong>{alarms.length === 1 ? "1 alarm needs attention" : `${alarms.length} alarms need attention`}</strong>
          </p>
          <ul>
            {alarms.map((a, n) => (
              <li key={`${n}-${a.code}`}>
                <bdi>{a.message}</bdi>{" "}
                <span className="safety-alarm-meta">
                  Since {since(a.since)} (<bdi>{a.code}</bdi>)
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
