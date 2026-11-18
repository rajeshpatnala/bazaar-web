import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw/http";
import { server } from "../mocks/server";
import { problemFixture } from "../mocks/handlers";
import { api, ApiError, onSessionEnded, unwrap } from "./client";

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
