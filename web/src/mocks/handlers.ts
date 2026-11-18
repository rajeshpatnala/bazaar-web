import { http, HttpResponse, type RequestHandler } from "msw";
import type { components } from "../api/schema";

type S = components["schemas"];

export const authMeFixture: S["AuthMeDto"] = {
  csrf_token: "test-csrf",
  mode: "paper",
  live_enabled: false,
};
export const statusFixture: S["StatusDto"] = {
  server_time: "2026-11-19T10:00:00+05:30",
  version: "0.1.0",
  mode: "paper",
  live_enabled: false,
};
export const problemFixture = (status: number, code: string): S["Problem"] => ({
  type: `urn:bazaar:problem:${code}`,
  title: code,
  status,
  code,
  detail: code,
  errors: [],
});

/** Shared by the browser worker and the Vitest server; bodies are typed from the generated schema. */
export const handlers: RequestHandler[] = [
  http.get("/api/v1/auth/me", () => HttpResponse.json(authMeFixture)),
  http.get("/api/v1/status", () => HttpResponse.json(statusFixture)),
];
