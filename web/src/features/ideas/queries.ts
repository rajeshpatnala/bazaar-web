import { queryOptions } from "@tanstack/react-query";
import { ApiError } from "../../api/client";
import type { IdeasResponse } from "./types";

// PROVISIONAL: plain fetch until `/api/v1/ideas` is in the generated schema.
export const ideasQuery = queryOptions({
  queryKey: ["ideas"],
  queryFn: async (): Promise<IdeasResponse> => {
    const r = await fetch("/api/v1/ideas", { credentials: "same-origin" });
    if (!r.ok) throw new ApiError(r.status, "ideas", `Request failed (${r.status})`);
    return (await r.json()) as IdeasResponse;
  },
});
