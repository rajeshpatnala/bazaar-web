import { afterEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { setCsrfToken } from "../../api/client";
import { DISCLAIMER } from "../../shared/disclaimer";
import { server } from "../../mocks/server";
import { renderApp } from "../../test/render";

afterEach(() => {
  setCsrfToken(null);
  vi.useRealTimers();
});

describe("alarms banner", () => {
  const alarm = { code: "square_off_blocked", message: "<b>Square-off</b> is blocked", since: "2026-11-19T04:30:00Z" };
  const alarms = (body: object) => server.use(http.get("/api/v1/alarms", () => HttpResponse.json(body)));

  it("says it is checking until the first answer, then shows nothing for an empty list", async () => {
    let answer!: () => void;
    const gate = new Promise<void>((r) => (answer = r));
    server.use(
      http.get("/api/v1/alarms", async () => {
        await gate;
        return HttpResponse.json([]);
      }),
    );
    renderApp();
    await screen.findByText("Checking alarms...");
    answer();
    await waitFor(() => expect(screen.queryByText(/alarm/i)).toBeNull());
  });

  it("lists alarms as text with IST time", async () => {
    alarms([alarm, { ...alarm, since: "garbage" }]);
    renderApp();
    const banner = await screen.findByRole("alert");
    expect(banner).toHaveTextContent("2 alarms need attention");
    expect(banner).toHaveTextContent("<b>Square-off</b> is blocked");
    expect(banner).toHaveTextContent("Since 19 Nov, 10:00 IST (square_off_blocked)");
    expect(banner).toHaveTextContent("time unavailable");
    expect(banner.querySelector("b")).toBeNull();
    expect(screen.getByRole("contentinfo")).toHaveTextContent(DISCLAIMER);
  });

  it.each([
    ["a non-list", { alarms: [] }],
    ["a bad item", [alarm, { code: "x", message: 1, since: "y" }]],
  ])("says alarms are unknown for %s", async (_n, body) => {
    alarms(body);
    renderApp();
    expect(await screen.findByRole("alert")).toHaveTextContent("Alarms unknown");
  });

  it("keeps the last list, labelled out of date, when a later read fails", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    alarms([alarm]);
    renderApp();
    await screen.findByText("1 alarm needs attention");
    server.use(http.get("/api/v1/alarms", () => HttpResponse.error()));
    await vi.advanceTimersByTimeAsync(5_100);
    await screen.findByText("Alarms unknown.");
    expect(screen.getByRole("alert")).toHaveTextContent("may be out of date");
    expect(screen.getByRole("alert")).toHaveTextContent("is blocked");
  });
});
