// PROVISIONAL: replace with `components["schemas"]` once the F13 server slice publishes OpenAPI.
// Money is a decimal string; the server supplies every figure, the UI computes nothing.
export type Overview = {
  as_of: string;
  capital: string;
  day_pnl: string;
  open_positions: number;
  ideas_today: number;
  regime: string;
  /** Server mode; the practice label shows only when this is "paper". */
  execution_mode: string;
  data_delayed: boolean;
};
