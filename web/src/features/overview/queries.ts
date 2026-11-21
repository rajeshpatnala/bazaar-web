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

function isCount(v: unknown): boolean {
  return typeof v === "number" && Number.isInteger(v) && v >= 0;
}

function parseOverview(raw: unknown): Overview {
  const o = raw as Partial<Record<keyof Overview, unknown>> | null;
  if (
    !o || !validTime(o.as_of) ||
    typeof o.capital !== "string" || !MONEY.test(o.capital) ||
    typeof o.day_pnl !== "string" || !MONEY.test(o.day_pnl) ||
    !isCount(o.open_positions) || !isCount(o.ideas_today) ||
    typeof o.regime !== "string" || o.regime === "" ||
    typeof o.execution_mode !== "string" || o.execution_mode === "" ||
    typeof o.data_delayed !== "boolean"
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
