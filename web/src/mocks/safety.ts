import { http, HttpResponse, type RequestHandler } from "msw";
import type { components } from "../api/schema";

type S = components["schemas"];

export const killSwitchOff: S["KillSwitchDto"] = { engaged: false, readable: true, latched: false, reason: null };
export const killSwitchOn: S["KillSwitchDto"] = { ...killSwitchOff, engaged: true, latched: true, reason: "manual stop" };

/** Stateless defaults: switch off. Tests override per case. */
export const safetyHandlers: RequestHandler[] = [
  http.get("/api/v1/killswitch", () => HttpResponse.json(killSwitchOff)),
  http.post("/api/v1/killswitch/engage", () => HttpResponse.json(killSwitchOn)),
];
