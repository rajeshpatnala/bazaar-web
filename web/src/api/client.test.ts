import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw/http";
import { server } from "../mocks/server";
import { problemFixture } from "../mocks/handlers";
import { api, ApiError, CSRF_HEADER, onSessionEnded, setCsrfToken, unwrap } from "./client";

const fail = (e: unknown) => e;

describe("api client", () => {
  it("returns data on success", async () => {
    const d = unwrap(await api.GET("/api/v1/status"));
    expect(d.mode).toBe("paper");
  });

  it("throws ApiError with status and code, and signals session end on 401", async () => {
    server.use(
      http.get("/api/v1/status", () =>
        HttpResponse.json(problemFixture(401, "unauthenticated"), { status: 401 }),
      ),
    );
    const ended = vi.fn();
    onSessionEnded(ended);
    const err = await api.GET("/api/v1/status").then(unwrap).catch(fail);
    onSessionEnded(null);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 401, code: "unauthenticated" });
    expect(ended).toHaveBeenCalledOnce();
  });

  it("falls back to an unknown code when the body is not a problem", async () => {
    server.use(http.get("/api/v1/status", () => new HttpResponse("boom", { status: 500 })));
    const err = await api.GET("/api/v1/status").then(unwrap).catch(fail);
    expect(err).toMatchObject({ status: 500, code: "unknown" });
  });
});

describe("CSRF header", () => {
  const verbs = ["POST", "PUT", "PATCH", "DELETE"] as const;
  const call = (m: string) => {
    const f = (api as unknown as Record<string, ((p: string) => Promise<unknown>) | undefined>)[m];
    if (!f) throw new Error(`no ${m}`);
    return f("/api/v1/status");
  };

  it("is sent on every state-changing method and not on GET", async () => {
    setCsrfToken("tok-123");
    const seen: Record<string, string | null> = {};
    const rec =
      (m: string) =>
      ({ request }: { request: Request }) => {
        seen[m] = request.headers.get(CSRF_HEADER);
        return HttpResponse.json({});
      };
    server.use(
      http.get("/api/v1/status", rec("GET")),
      http.post("/api/v1/status", rec("POST")),
      http.put("/api/v1/status", rec("PUT")),
      http.patch("/api/v1/status", rec("PATCH")),
      http.delete("/api/v1/status", rec("DELETE")),
    );
    for (const m of verbs) await call(m);
    await call("GET");
    setCsrfToken(null);
    expect(seen).toEqual({
      POST: "tok-123",
      PUT: "tok-123",
      PATCH: "tok-123",
      DELETE: "tok-123",
      GET: null,
    });
  });

  it("refuses a state-changing request when no token is set", async () => {
    setCsrfToken(null);
    await expect(call("POST")).rejects.toThrow("missing CSRF token");
  });
});
