import { queryOptions } from "@tanstack/react-query";
import { ApiError, apiJson } from "../../api/client";
import { formatIstIso } from "../../shared/lib/format-ist";
import type { Overview } from "./types";

const MONEY = /^-?\d+(\.\d+)?$/;

function validTime(v: unknown): boolean {
  try {
    return typeof v === "string" && formatIstIso(v) !== "";
  } catch {
    return false;
  }
}

function parseOverview(raw: unknown): Overview {
  const o = raw as Partial<Record<keyof Overview, unknown>> | null;
  if (
    !o || !validTime(o.as_of) ||
    typeof o.capital !== "string" || !MONEY.test(o.capital) ||
    typeof o.day_pnl !== "string" || !MONEY.test(o.day_pnl) ||
    !Number.isInteger(o.open_positions) || !Number.isInteger(o.ideas_today) ||
    typeof o.regime !== "string" || typeof o.data_delayed !== "boolean"
  ) {
    throw new ApiError(0, "bad_shape", "Unexpected overview from the server.");
  }
  return o as Overview;
}

// PROVISIONAL: `apiJson` until `/api/v1/overview` is in the generated schema (BZ-509).
export const overviewQuery = queryOptions({
  queryKey: ["overview"],
  queryFn: async (): Promise<Overview> => parseOverview(await apiJson("/api/v1/overview")),
});
