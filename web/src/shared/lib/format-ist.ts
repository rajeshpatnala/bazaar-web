const IST = new Intl.DateTimeFormat("en-IN", {
  timeZone: "Asia/Kolkata",
  day: "2-digit",
  month: "short",
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});

/** UTC epoch seconds -> IST display string, whatever the browser zone is. */
export const formatIst = (epochSeconds: number): string => `${IST.format(epochSeconds * 1000)} IST`;

/** ISO-8601 timestamp with offset (e.g. the server's `server_time`) -> IST display string. */
export function formatIstIso(iso: string): string {
  if (!/(Z|[+-]\d{2}:\d{2})$/.test(iso)) throw new Error(`timestamp needs Z or an offset: ${iso}`);
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) throw new Error(`bad timestamp: ${iso}`);
  return formatIst(Math.floor(ms / 1000));
}
