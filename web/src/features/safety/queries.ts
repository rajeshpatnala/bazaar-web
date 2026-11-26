import { queryOptions, useMutation, useQueryClient } from "@tanstack/react-query";
import { api, ApiError, unwrap } from "../../api/client";
import { keys } from "../../api/keys";
import type { components } from "../../api/schema";

export type KillSwitch = components["schemas"]["KillSwitchDto"];
export type Alarm = components["schemas"]["AlarmDto"];

const TIMEOUT_MS = 10_000;
const POLL_MS = 5_000;

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;

const isKillSwitch = (v: unknown): v is KillSwitch =>
  isObj(v) &&
  typeof v.engaged === "boolean" &&
  typeof v.readable === "boolean" &&
  typeof v.latched === "boolean" &&
  (v.reason === undefined || v.reason === null || typeof v.reason === "string");

const isAlarm = (v: unknown): v is Alarm =>
  isObj(v) && typeof v.code === "string" && typeof v.message === "string" && typeof v.since === "string";

type Polled = { state: { error: unknown } };
const loggedOut = (q: Polled) => q.state.error instanceof ApiError && q.state.error.status === 401;

/** A localhost app is never "paused offline". A hidden tab keeps polling, so an old answer is not shown on return. */
const live = {
  networkMode: "always",
  staleTime: 0,
  refetchInterval: (q: Polled) => (loggedOut(q) ? false : POLL_MS),
  refetchIntervalInBackground: true,
  refetchOnWindowFocus: (q: Polled) => !loggedOut(q),
} as const;

const bounded = (signal: AbortSignal) => AbortSignal.any([signal, AbortSignal.timeout(TIMEOUT_MS)]);
const badShape = () => new ApiError(0, "bad_shape", "Unexpected answer from the server.");

export const killSwitchQuery = queryOptions({
  queryKey: keys.killSwitch,
  queryFn: async ({ signal }) => {
    const data: unknown = unwrap(await api.GET("/api/v1/killswitch", { signal: bounded(signal) }));
    if (!isKillSwitch(data)) throw badShape();
    return data;
  },
  ...live,
});

export const alarmsQuery = queryOptions({
  queryKey: keys.alarms,
  queryFn: async ({ signal }) => {
    const data: unknown = unwrap(await api.GET("/api/v1/alarms", { signal: bounded(signal) }));
    if (!Array.isArray(data) || !data.every(isAlarm)) throw badShape();
    return data;
  },
  ...live,
});

/** The POST reply is never shown as the state: success or failure, the cache is cleared and read again. */
function useKillSwitchWrite<T>(send: (input: T) => Promise<unknown>) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: send,
    retry: false,
    networkMode: "always",
    onMutate: () => client.cancelQueries({ queryKey: keys.killSwitch }),
    onSettled: async () => {
      await client.cancelQueries({ queryKey: keys.killSwitch });
      void client.resetQueries({ queryKey: keys.killSwitch });
    },
  });
}

export const useEngage = () =>
  useKillSwitchWrite(async (reason: string) => {
    const signal = AbortSignal.timeout(TIMEOUT_MS);
    return unwrap(await api.POST("/api/v1/killswitch/engage", { body: { reason }, signal }));
  });

/** The phrase is taken out of its box as it is sent, so the mutation cache does not keep it. */
export const useRelease = () =>
  useKillSwitchWrite(async (box: { phrase: string }) => {
    const body = { phrase: box.phrase };
    box.phrase = "";
    const signal = AbortSignal.timeout(TIMEOUT_MS);
    return unwrap(await api.POST("/api/v1/killswitch/release", { body, signal }));
  });
