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
