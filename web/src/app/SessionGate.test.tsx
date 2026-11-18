import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import { http, HttpResponse } from "msw/http";
import { DISCLAIMER } from "../shared/disclaimer";
import { problemFixture, statusFixture } from "../mocks/handlers";
import { server } from "../mocks/server";
import { renderApp } from "../test/render";

describe("SessionGate and ModeBadge", () => {
  it("shows the PAPER badge and the screen on 200", async () => {
    renderApp();
    expect(await screen.findByText("PAPER")).toBeVisible();
    expect(await screen.findByRole("heading", { name: "Market overview" })).toBeVisible();
  });

  it("shows a non-paper mode distinctly", async () => {
    server.use(
      http.get("/api/v1/status", () => HttpResponse.json({ ...statusFixture, mode: "semi_auto" })),
    );
    renderApp();
    expect(await screen.findByText("SEMI-AUTO")).toHaveAttribute("data-mode", "semi_auto");
  });

  it("shows the session-ended screen on 401 with the disclaimer and no retry", async () => {
    let calls = 0;
    server.use(
      http.get("/api/v1/auth/me", () => {
        calls += 1;
        return HttpResponse.json(problemFixture(401, "unauthenticated"), { status: 401 });
      }),
    );
    renderApp();
    expect(await screen.findByText("Session ended")).toBeVisible();
    expect(screen.queryByRole("heading", { name: "Market overview" })).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByRole("contentinfo")).toHaveTextContent(DISCLAIMER);
    expect(calls).toBe(1);
  });

  it("shows an unreachable message on a network error, keeping the disclaimer", async () => {
    server.use(http.get("/api/v1/auth/me", () => HttpResponse.error()));
    renderApp();
    expect(await screen.findByText(/Cannot reach the Bazaar server/)).toBeVisible();
    expect(screen.getByRole("contentinfo")).toHaveTextContent(DISCLAIMER);
  });
});
