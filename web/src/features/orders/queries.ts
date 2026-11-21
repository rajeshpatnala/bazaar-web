import { queryOptions } from "@tanstack/react-query";
import { ApiError, CSRF_HEADER, getCsrfToken } from "../../api/client";
import type { ApprovalResult, PaperPreview } from "./types";

// PROVISIONAL: plain fetch until these routes are in the generated schema.
export const previewQuery = (ideaId: string) =>
  queryOptions({
    queryKey: ["paper-preview", ideaId],
    queryFn: async (): Promise<PaperPreview> => {
      const r = await fetch(`/api/v1/ideas/${encodeURIComponent(ideaId)}/paper-preview`, {
        credentials: "same-origin",
        signal: AbortSignal.timeout(10_000),
      });
      if (!r.ok) throw new ApiError(r.status, "preview", `Request failed (${r.status})`);
      const body = (await r.json()) as Partial<PaperPreview> | null;
      if (!body || typeof body.total !== "string" || typeof body.charges !== "string" || typeof body.max_loss !== "string") {
        throw new ApiError(r.status, "bad_shape", "Unexpected preview response");
      }
      return body as PaperPreview;
    },
  });

/** Only ever called from a user click in the review dialog. */
export async function approvePaperOrder(approvalId: string): Promise<ApprovalResult> {
  const token = getCsrfToken();
  if (!token) throw new ApiError(0, "csrf", "Missing security token");
  const r = await fetch(`/api/v1/approvals/${encodeURIComponent(approvalId)}/approve`, {
    method: "POST",
    credentials: "same-origin",
    headers: { [CSRF_HEADER]: token },
    signal: AbortSignal.timeout(10_000),
  });
  if (!r.ok) throw new ApiError(r.status, "approve", `Request failed (${r.status})`);
  return (await r.json()) as ApprovalResult;
}
