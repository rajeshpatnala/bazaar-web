import { queryOptions } from "@tanstack/react-query";
import { ApiError, apiJson } from "../../api/client";
import { formatIstIso } from "../../shared/lib/format-ist";
import type { ApprovalResult, PaperPreview } from "./types";

const MONEY = /^-?\d+(\.\d+)?$/;
const bad = () => new ApiError(0, "bad_shape", "Unexpected order details from the server.");

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}
function validTime(v: unknown): v is string {
  if (typeof v !== "string") return false;
  try {
    formatIstIso(v);
    return true;
  } catch {
    return false;
  }
}

/** Check every field before it can reach the screen; anything off is an error, not a guess. */
export function parsePreview(raw: unknown): PaperPreview {
  if (!isObj(raw) || !isObj(raw.order) || !isObj(raw.gate)) throw bad();
  const o = raw.order;
  const money = [o.entry, o.stop, o.max_loss, o.charges, o.total];
  if (
    typeof raw.approval_id !== "string" || raw.approval_id === "" ||
    typeof raw.execution_mode !== "string" ||
    !validTime(raw.as_of) || !validTime(raw.expires_at) ||
    !Number.isInteger(raw.version) ||
    typeof raw.gate.verdict !== "string" ||
    !Array.isArray(raw.gate.reasons) || !raw.gate.reasons.every((r) => typeof r === "string") ||
    typeof raw.low_evidence !== "boolean" ||
    typeof o.symbol !== "string" || o.symbol === "" ||
    (o.side !== "buy" && o.side !== "sell") ||
    !Number.isInteger(o.quantity) || (o.quantity as number) < 1 ||
    !money.every((m) => typeof m === "string" && MONEY.test(m))
  ) {
    throw bad();
  }
  return raw as unknown as PaperPreview;
}

/** Server creates a gate-checked draft. Fetched once per dialog open; never refetched on focus. */
export const previewQuery = (ideaId: string) =>
  queryOptions({
    queryKey: ["paper-preview", ideaId],
    queryFn: async (): Promise<PaperPreview> =>
      parsePreview(await apiJson(`/api/v1/ideas/${encodeURIComponent(ideaId)}/paper-preview`, { method: "POST", body: {} })),
    staleTime: Infinity,
    gcTime: 0,
    networkMode: "always", // never pause a re-preview: the old draft would look current
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });

/** Only ever called from a user click in the review dialog. Same key on retry: a repeat returns the first outcome. */
export function approvePaperOrder(p: PaperPreview, idempotencyKey: string): Promise<ApprovalResult> {
  return apiJson<ApprovalResult>(`/api/v1/approvals/${encodeURIComponent(p.approval_id)}/approve`, {
    method: "POST",
    body: { version: p.version },
    headers: { "Idempotency-Key": idempotencyKey },
  });
}
