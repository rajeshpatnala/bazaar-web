import { queryOptions } from "@tanstack/react-query";
import { ApiError } from "../../api/client";
import type { IdeasResponse } from "./types";

// PROVISIONAL: plain fetch until `/api/v1/ideas` is in the generated schema.
export const ideasQuery = queryOptions({
  queryKey: ["ideas"],
  queryFn: async (): Promise<IdeasResponse> => {
    const r = await fetch("/api/v1/ideas", { credentials: "same-origin", signal: AbortSignal.timeout(10_000) });
    if (!r.ok) throw new ApiError(r.status, "ideas", `Request failed (${r.status})`);
    const body = (await r.json()) as Partial<IdeasResponse> | null;
    if (!body || !Array.isArray(body.ideas) || typeof body.as_of !== "string") {
      throw new ApiError(r.status, "bad_shape", "Unexpected ideas response");
    }
    return body as IdeasResponse;
  },
});
