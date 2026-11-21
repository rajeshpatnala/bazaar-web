// PROVISIONAL contract (tech lead, CR-126-1). BZ-512 (server) implements it, BZ-509 swaps in the
// generated `components["schemas"]` types. Money is a decimal string; the UI computes nothing.
// TODO(BZ-512): the server must re-run the DecisionGate on approve and bind the approval to this
// exact order, version and expiry; stale, foreign or repeated approvals are refused or return the first outcome.
export type PaperPreview = {
  approval_id: string;
  execution_mode: string;
  as_of: string;
  version: number;
  gate: { verdict: string; reasons: string[] };
  expires_at: string;
  order: {
    symbol: string;
    side: "buy" | "sell";
    quantity: number;
    entry: string;
    stop: string;
    max_loss: string;
    charges: string;
    total: string;
  };
  low_evidence: boolean;
};

export type ApprovalResult = { status: "paper_filled" | "rejected"; message: string };
