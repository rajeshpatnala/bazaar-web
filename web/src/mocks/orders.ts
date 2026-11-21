import { http, HttpResponse, type RequestHandler } from "msw";
import type { ApprovalResult, PaperPreview } from "../features/orders/types";

/** Fake data. `expires_at` is built per request so the mock preview is always fresh (5 minutes). */
export const makePreview = (over: Partial<PaperPreview> = {}): PaperPreview => ({
  approval_id: "appr-1",
  execution_mode: "paper",
  as_of: "2026-11-19T10:00:00+05:30",
  version: 1,
  gate: { verdict: "allow", reasons: ["Position size is within your limit.", "Daily loss limit not reached."] },
  expires_at: new Date(Date.now() + 300_000).toISOString(),
  order: {
    symbol: "RELIANCE",
    side: "buy",
    quantity: 20,
    entry: "2850.00",
    stop: "2800.00",
    max_loss: "1000.00",
    charges: "42.50",
    total: "57000.00",
  },
  low_evidence: false,
  ...over,
});

export const approvalFixture: ApprovalResult = {
  status: "paper_filled",
  message: "Recorded as a practice order; no real money was used.",
};

export const ordersHandlers: RequestHandler[] = [
  http.post("/api/v1/ideas/:id/paper-preview", () => HttpResponse.json(makePreview())),
  http.post("/api/v1/approvals/:id/approve", () => HttpResponse.json(approvalFixture)),
];
