/**
 * Format a decimal string (plain digits, optional leading "-", optional fraction; no "+", exponent
 * or separators) as Indian-grouped rupees. String/BigInt work only, no float arithmetic.
 * Extra fraction digits are rounded half-to-even (banker's rounding); the server already rounds
 * money, so this only matters for display with fewer decimals.
 */
export function formatInr(value: string, opts: { decimals?: number } = {}): string {
  const decimals = opts.decimals ?? 2;
  const m = /^(-)?(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!m) throw new Error(`bad money value: ${value}`);
  const neg = m[1] ?? "";
  const frac = m[3] ?? "";
  let digits = BigInt((m[2] ?? "0") + frac.padEnd(decimals, "0").slice(0, decimals));
  const rest = frac.slice(decimals);
  if (rest) {
    const first = rest.charCodeAt(0) - 48;
    const tail = /[1-9]/.test(rest.slice(1));
    if (first > 5 || (first === 5 && (tail || digits % 2n === 1n))) digits += 1n;
  }
  const s = digits.toString().padStart(decimals + 1, "0");
  const int = decimals ? s.slice(0, -decimals) : s;
  const cents = decimals ? s.slice(-decimals) : "";
  const head = int.slice(0, -3).replace(/\B(?=(\d{2})+(?!\d))/g, ",");
  const grouped = int.length > 3 ? `${head},${int.slice(-3)}` : int;
  const isZero = digits === 0n;
  return `${isZero ? "" : neg}₹${grouped}${decimals ? "." + cents : ""}`;
}
