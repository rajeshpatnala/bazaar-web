import { queryOptions, useMutation, useQueryClient } from "@tanstack/react-query";
import { useRef } from "react";
import { api, ApiError, unwrap } from "../../api/client";
import { keys } from "../../api/keys";
import type { components } from "../../api/schema";

export type KillSwitch = components["schemas"]["KillSwitchDto"];

const TIMEOUT_MS = 10_000;
const POLL_MS = 5_000;

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;

const isKillSwitch = (v: unknown): v is KillSwitch =>
  isObj(v) &&
  typeof v.engaged === "boolean" &&
  typeof v.readable === "boolean" &&
  typeof v.latched === "boolean" &&
  (v.reason === undefined || v.reason === null || typeof v.reason === "string");

const loggedOut = (q: { state: { error: unknown } }) => q.state.error instanceof ApiError && q.state.error.status === 401;

export const killSwitchQuery = queryOptions({
  queryKey: keys.killSwitch,
  queryFn: async ({ signal }) => {
    const bounded = AbortSignal.any([signal, AbortSignal.timeout(TIMEOUT_MS)]);
    const data: unknown = unwrap(await api.GET("/api/v1/killswitch", { signal: bounded }));
    if (!isKillSwitch(data)) throw new ApiError(0, "bad_shape", "Unexpected answer from the server.");
    return data;
  },
  // A localhost app is never "paused offline". A hidden tab keeps polling, so an old OFF is not shown on return.
  networkMode: "always",
  staleTime: 0,
  refetchInterval: (q) => (loggedOut(q) ? false : POLL_MS),
  refetchIntervalInBackground: true,
  refetchOnWindowFocus: (q) => !loggedOut(q),
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

/**
 * The server runs the writes in the order they arrive, and two requests can arrive either way round. So an engage
 * clicked during a release is sent at once and again when it ends: this beats a release queued before the second engage.
 */
export function useKillSwitchWrites() {
  // Settles when the release request in flight ends, however it ends.
  const releasing = useRef<Promise<void> | null>(null);

  const engage = useKillSwitchWrite(async () => {
    const post = async () => {
      const body = { reason: "" }, signal = AbortSignal.timeout(TIMEOUT_MS); // blank is stored as "unspecified"
      return unwrap(await api.POST("/api/v1/killswitch/engage", { body, signal }));
    };
    const ended = releasing.current;
    if (!ended) return post();
    await Promise.allSettled([post(), ended]);
    return post();
  });

  // The phrase is taken out of its box as it is sent, so the mutation cache does not keep it.
  const release = useKillSwitchWrite((box: { phrase: string }) => {
    const body = { phrase: box.phrase };
    box.phrase = "";
    const signal = AbortSignal.timeout(TIMEOUT_MS);
    const sent = (async () => unwrap(await api.POST("/api/v1/killswitch/release", { body, signal })))();
    const done = () => {
      releasing.current = null;
    };
    releasing.current = sent.then(done, done);
    return sent;
  });

  return { engage, release };
}
