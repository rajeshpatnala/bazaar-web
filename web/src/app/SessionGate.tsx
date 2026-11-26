import { useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { ApiError } from "../api/client";
import { authMeQuery } from "../api/queries";
import { SessionEnded } from "./SessionEnded";

/** Renders children only with a live session; the disclaimer lives outside the gate. */
export function SessionGate({ children }: { children: ReactNode }) {
  const me = useQuery(authMeQuery);
  // A 401 sticks until a probe succeeds: a later error or a pending retry replaces it, and old data stays cached.
  const [ended, setEnded] = useState(false);
  const unauthorised = me.error instanceof ApiError && me.error.status === 401;
  if (unauthorised && !ended) setEnded(true);
  if (me.isSuccess && ended) setEnded(false);
  if (unauthorised || (ended && !me.isSuccess)) return <SessionEnded />;
  if (me.isPending) return <p>Checking session...</p>;
  const unreachable = <p role="alert">Cannot reach the Bazaar server. Is the app still running?</p>;
  // Gate on data, not on the latest error: a failed later probe must not remove the kill switch.
  if (!me.data) return unreachable;
  return (
    <>
      {me.isError && unreachable}
      {children}
    </>
  );
}
