import { http, HttpResponse, type RequestHandler } from "msw";
import type { ApprovalResult, PaperPreview } from "../features/orders/types";

export const previewFixture: PaperPreview = {
  idea_id: "idea-1",
  symbol: "RELIANCE",
  side: "buy",
  quantity: 20,
  total: "57000.00",
  charges: "42.50",
  max_loss: "1000.00",
  as_of: "2026-11-19T10:00:00+05:30",
};

export const approvalFixture: ApprovalResult = {
  status: "paper_filled",
  message: "Recorded as a practice order; no real money was used.",
};

export const ordersHandlers: RequestHandler[] = [
  http.get("/api/v1/ideas/:id/paper-preview", ({ params }) =>
    HttpResponse.json({ ...previewFixture, idea_id: String(params.id) }),
  ),
  http.post("/api/v1/approvals/:id/approve", () => HttpResponse.json(approvalFixture)),
];
