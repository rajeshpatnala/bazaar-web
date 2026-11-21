import { formatInr } from "../lib/format-inr";

/** The only way to show a rupee amount. Value is a decimal string from the server. */
export function Money({ value, decimals }: { value: string; decimals?: number }) {
  return <span className="ui-money">{formatInr(value, decimals === undefined ? {} : { decimals })}</span>;
}
