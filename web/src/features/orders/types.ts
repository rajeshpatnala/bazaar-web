// PROVISIONAL: replace with `components["schemas"]` once the F13 server slice publishes OpenAPI.
// Money is a decimal string. The server supplies total, charges and max loss; the UI never computes them.
export type PaperPreview = {
  idea_id: string;
  symbol: string;
  side: "buy" | "sell";
  quantity: number;
  total: string;
  charges: string;
  max_loss: string;
  as_of: string;
};

export type ApprovalResult = { status: "paper_filled" | "rejected"; message: string };
