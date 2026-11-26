import { queryOptions } from "@tanstack/react-query";
import { api, setCsrfToken, unwrap } from "./client";
import { keys } from "./keys";

export const authMeQuery = queryOptions({
  queryKey: keys.authMe,
  queryFn: async () => {
    const me = unwrap(await api.GET("/api/v1/auth/me"));
    setCsrfToken(me.csrf_token);
    return me;
  },
  staleTime: 60_000,
  networkMode: "always", // a localhost app is never "paused offline"
});

export const statusQuery = queryOptions({
  queryKey: keys.status,
  queryFn: async () => unwrap(await api.GET("/api/v1/status")),
  refetchInterval: 30_000,
});
