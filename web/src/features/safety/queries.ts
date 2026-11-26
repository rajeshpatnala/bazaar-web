import { queryOptions, useMutation, useQueryClient } from "@tanstack/react-query";
import { api, ApiError, unwrap } from "../../api/client";
import { keys } from "../../api/keys";
import type { components } from "../../api/schema";

export type KillSwitch = components["schemas"]["KillSwitchDto"];
export type Alarm = components["schemas"]["AlarmDto"];

const TIMEOUT_MS = 10_000;
const POLL_MS = 5_000;

const bad = () => new ApiError(0, "bad_shape", "Unexpected answer from the server.");
const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;

const isKillSwitch = (v: unknown): v is KillSwitch =>
  isObj(v) &&
  typeof v.engaged === "boolean" &&
  typeof v.readable === "boolean" &&
  typeof v.latched === "boolean" &&
  (v.reason === undefined || v.reason === null || typeof v.reason === "string");

const isAlarm = (v: unknown): v is Alarm =>
  isObj(v) && typeof v.code === "string" && typeof v.message === "string" && typeof v.since === "string";

const is401 = (e: unknown) => e instanceof ApiError && e.status === 401;

type Errored = { state: { error: unknown } };

/** Shared polling rules: localhost app (never "paused offline"), no polling or refetch after a 401. */
const live = {
  networkMode: "always",
  staleTime: 0,
  refetchInterval: (q: Errored) => (is401(q.state.error) ? false : POLL_MS),
  refetchOnWindowFocus: (q: Errored) => !is401(q.state.error),
} as const;

const bounded = (signal: AbortSignal) => AbortSignal.any([signal, AbortSignal.timeout(TIMEOUT_MS)]);

export const killSwitchQuery = queryOptions({
  queryKey: keys.killSwitch,
  queryFn: async ({ signal }) => {
    const data: unknown = unwrap(await api.GET("/api/v1/killswitch", { signal: bounded(signal) }));
    if (!isKillSwitch(data)) throw bad();
    return data;
  },
  ...live,
});

export const alarmsQuery = queryOptions({
  queryKey: keys.alarms,
  queryFn: async ({ signal }) => {
    const data: unknown = unwrap(await api.GET("/api/v1/alarms", { signal: bounded(signal) }));
    if (!Array.isArray(data) || !data.every(isAlarm)) throw bad();
    return data;
  },
  ...live,
});

/**
 * A kill-switch write. The POST reply is never shown as the state: any read in flight is dropped
 * and, success or failure, the cached state is cleared and read again (unknown until then).
 */
function useKillSwitchWrite(send: (text: string) => Promise<unknown>) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: send,
    retry: false,
    gcTime: 0, // the typed release phrase is not kept in the mutation cache
    networkMode: "always",
    onMutate: () => client.cancelQueries({ queryKey: keys.killSwitch }),
    onSettled: async () => {
      await client.cancelQueries({ queryKey: keys.killSwitch });
      void client.resetQueries({ queryKey: keys.killSwitch });
    },
  });
}

export const useEngage = () =>
  useKillSwitchWrite(async (reason) =>
    unwrap(
      await api.POST("/api/v1/killswitch/engage", {
        body: { reason },
        signal: AbortSignal.timeout(TIMEOUT_MS),
      }),
    ),
  );

export const useRelease = () =>
  useKillSwitchWrite(async (phrase) =>
    unwrap(
      await api.POST("/api/v1/killswitch/release", {
        body: { phrase },
        signal: AbortSignal.timeout(TIMEOUT_MS),
      }),
    ),
  );
