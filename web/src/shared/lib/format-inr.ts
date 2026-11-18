/** Format a decimal string as Indian-grouped rupees. String work only; no float arithmetic. */
export function formatInr(value: string, opts: { decimals?: number } = {}): string {
  const decimals = opts.decimals ?? 2;
  const m = /^(-)?(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!m) throw new Error(`bad money value: ${value}`);
  const neg = m[1] ?? "";
  const int = m[2] ?? "0";
  const frac = m[3] ?? "";
  const head = int.slice(0, -3).replace(/\B(?=(\d{2})+(?!\d))/g, ",");
  const grouped = int.length > 3 ? `${head},${int.slice(-3)}` : int;
  const cents = frac.padEnd(decimals, "0").slice(0, decimals);
  return `${neg}₹${grouped}${decimals ? "." + cents : ""}`;
}
