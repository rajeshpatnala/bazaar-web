import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { ApiError } from "../api/client";
import { authMeQuery } from "../api/queries";
import { SessionEnded } from "./SessionEnded";

/** Renders children only with a live session; the disclaimer lives outside the gate. */
export function SessionGate({ children }: { children: ReactNode }) {
  const me = useQuery(authMeQuery);
  if (me.isPending) return <p>Checking session...</p>;
  if (me.error instanceof ApiError && me.error.status === 401) return <SessionEnded />;
  if (me.isError) {
    return <p role="alert">Cannot reach the Bazaar server. Is the app still running?</p>;
  }
  return <>{children}</>;
}
