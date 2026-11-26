import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import { http, HttpResponse } from "msw/http";
import { DISCLAIMER } from "../shared/disclaimer";
import { problemFixture } from "../mocks/handlers";
import { server } from "../mocks/server";
import { renderApp } from "../test/render";

describe("AppShell", () => {
  it.each(["/", "/ideas", "/scanner", "/anything/else"])(
    "renders the disclaimer on %s, also when the API is down",
    async (path) => {
      server.use(http.all("/api/*", () => HttpResponse.error()));
      renderApp({ route: path });
      expect(await screen.findByText(DISCLAIMER)).toBeVisible();
      expect(screen.getByRole("contentinfo")).toHaveTextContent(DISCLAIMER);
    },
  );

  it("keeps the disclaimer on the session-ended screen", async () => {
    server.use(
      http.get("/api/v1/auth/me", () =>
        HttpResponse.json(problemFixture(401, "unauthenticated"), { status: 401 }),
      ),
    );
    renderApp({ route: "/ideas" });
    expect(await screen.findByText("Session ended")).toBeVisible();
    expect(screen.getByRole("contentinfo")).toHaveTextContent(DISCLAIMER);
  });

  it("renders the home screen", async () => {
    renderApp();
    expect(await screen.findByRole("heading", { name: "Market overview" })).toBeVisible();
  });

  it("renders the ideas placeholder route", async () => {
    renderApp({ route: "/ideas" });
    expect(await screen.findByRole("heading", { name: "Ideas to consider today" })).toBeVisible();
  });

  it("renders a not-found page inside the shell", async () => {
    renderApp({ route: "/nope" });
    expect(await screen.findByText("Page not found.")).toBeVisible();
  });

  it("shows PAPER, a read-only live note and the server time in IST, with no mode control", async () => {
    renderApp();
    expect(await screen.findByText("PAPER")).toBeVisible();
    expect(screen.getByText("Live: off (set on the server)")).toBeVisible();
    expect(await screen.findByText("19 Nov, 10:00 IST")).toBeVisible();
    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
    expect(screen.queryByRole("button", { name: /live|mode|paper/i })).toBeNull();
  });

  it("states live is on when the server says so", async () => {
    server.use(
      http.get("/api/v1/status", () =>
        HttpResponse.json({
          server_time: "2026-11-19T04:30:00Z",
          version: "0.1.0",
          mode: "semi_auto",
          live_enabled: true,
        }),
      ),
    );
    renderApp();
    expect(await screen.findByText("Live: on (set on the server)")).toBeVisible();
  });

  it("survives a bad server_time and keeps the disclaimer", async () => {
    server.use(
      http.get("/api/v1/status", () =>
        HttpResponse.json({
          server_time: "garbage",
          version: "0.1.0",
          mode: "paper",
          live_enabled: false,
        }),
      ),
    );
    renderApp();
    expect(await screen.findByText("time unavailable")).toBeVisible();
    expect(screen.getByRole("contentinfo")).toHaveTextContent(DISCLAIMER);
  });
});
