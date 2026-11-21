// PROVISIONAL: replace with `components["schemas"]` once the F13 server slice publishes OpenAPI.
// Money is a decimal string, never a number.
export type Idea = {
  id: string;
  symbol: string;
  side: "buy" | "sell";
  entry: string;
  stop: string;
  target: string;
  quantity: number;
  max_loss: string;
  low_evidence: boolean;
  regime: string;
  data_delayed: boolean;
  why: string[];
};

export type IdeasResponse = { ideas: Idea[]; as_of: string };
