import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { createMemoryRouter, type RouteObject } from "react-router";
import { RouterProvider } from "react-router/dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setCsrfToken } from "../api/client";
import { makePreview } from "../mocks/orders";
import { server } from "../mocks/server";
import { DISCLAIMER } from "../shared/disclaimer";
import { renderApp } from "../test/render";
import { Providers, makeQueryClient } from "./providers";
import { AppShell } from "./AppShell";

// F13 T2 gap-filling: error route, no-dismiss, browser-zone independence, banned wording.
const BANNED =
  /\b(will|guaranteed|sure|guarantee[sd]?)\b|go live|live order|place order|placed|SEBI[- ]registered/i;
const stripDisclaimer = (t: string) => t.replace(DISCLAIMER, "");

describe("F13 T2 gaps", () => {
  afterEach(() => setCsrfToken(null));

  it("the disclaimer is a plain footer: no button, link or input inside it, on every route", async () => {
    for (const route of ["/", "/ideas", "/nope"]) {
      const { unmount } = renderApp({ route });
      const footer = (await screen.findByText(DISCLAIMER)).closest("footer")!;
      expect(within(footer).queryAllByRole("button")).toHaveLength(0);
      expect(footer.querySelector("button,a,input,[tabindex]")).toBeNull();
      unmount();
    }
  });

  it("session-ended screen (401) keeps the disclaimer", async () => {
    server.use(http.get("/api/v1/auth/me", () => new HttpResponse(null, { status: 401 })));
    renderApp({ route: "/ideas" });
    expect(await screen.findByText(DISCLAIMER)).toBeVisible();
  });

  it("a route that throws while rendering still shows the disclaimer (error route)", async () => {
    const Boom = () => {
      throw new Error("boom");
    };
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    const bad: RouteObject[] = [
      { path: "/", element: <AppShell />, children: [{ index: true, element: <Boom /> }] },
    ];
    const router = createMemoryRouter(bad, { initialEntries: ["/"] });
    render(
      <Providers client={makeQueryClient()}>
        <RouterProvider router={router} />
      </Providers>,
    );
    try {
      expect(await screen.findByText(DISCLAIMER)).toBeVisible();
    } finally {
      spy.mockRestore();
    }
  });

  it.each(["America/Los_Angeles", "Pacific/Kiritimati", "UTC"])(
    "times are IST whatever the process zone (%s) on home, ideas and the dialog",
    async (tz) => {
      const prev = process.env.TZ;
      process.env.TZ = tz;
      try {
        const home = renderApp({ route: "/" });
        expect(await screen.findByText("19 Nov, 10:00 IST")).toBeVisible();
        home.unmount();
        renderApp({ route: "/ideas" });
        expect((await screen.findAllByText(/19 Nov, 10:00 IST/)).length).toBeGreaterThan(0);
        const card = await screen.findByRole("region", { name: /RELIANCE/ });
        await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
        const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
        await within(dialog).findByRole("button", { name: "Approve paper order" });
        expect(within(dialog).getByText("19 Nov, 10:00 IST")).toBeVisible();
      } finally {
        if (prev === undefined) delete process.env.TZ;
        else process.env.TZ = prev;
      }
    },
  );

  it("no banned wording on home, ideas, not-found or either review dialog (disclaimer excluded)", async () => {
    for (const route of ["/", "/ideas", "/nope"]) {
      const { unmount } = renderApp({ route });
      await screen.findByText(DISCLAIMER);
      await screen.findAllByRole("link");
      if (route === "/ideas") await screen.findByRole("region", { name: /RELIANCE/ });
      if (route === "/") await screen.findByRole("region", { name: /Practice account/ });
      expect(stripDisclaimer(document.body.textContent ?? "")).not.toMatch(BANNED);
      unmount();
    }
    renderApp({ route: "/ideas" });
    for (const name of [/RELIANCE/, /TCS/]) {
      const card = await screen.findByRole("region", { name });
      await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
      const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
      await within(dialog).findByRole("button", { name: "Approve paper order" });
      expect(stripDisclaimer(dialog.textContent ?? "")).not.toMatch(BANNED);
      await userEvent.keyboard("{Escape}");
    }
  });

  it("approve sends {version}, a UUID Idempotency-Key and CSRF (preview version 7 sends 7)", async () => {
    let seen: { body: unknown; key: string | null; csrf: string | null } | null = null;
    server.use(
      http.post("/api/v1/ideas/:id/paper-preview", () =>
        HttpResponse.json(makePreview({ version: 7 })),
      ),
      http.post("/api/v1/approvals/:id/approve", async ({ request }) => {
        seen = {
          body: await request.json(),
          key: request.headers.get("Idempotency-Key"),
          csrf: request.headers.get("X-CSRF-Token"),
        };
        return HttpResponse.json({ status: "paper_filled", message: "ok" });
      }),
    );
    renderApp({ route: "/ideas" });
    const card = await screen.findByRole("region", { name: /RELIANCE/ });
    await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
    const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
    await userEvent.click(
      await within(dialog).findByRole("button", { name: "Approve paper order" }),
    );
    await within(dialog).findByRole("status");
    expect(seen).toEqual({
      body: { version: 7 },
      key: expect.stringMatching(/^[0-9a-f-]{36}$/),
      csrf: "test-csrf",
    });
  });

  it("a gate block shows WHICH rule (reason text) and keeps Approve disabled", async () => {
    server.use(
      http.post("/api/v1/ideas/:id/paper-preview", () =>
        HttpResponse.json(
          makePreview({
            gate: {
              verdict: "block",
              reasons: ["Kill switch is on.", "Stop would widen the risk."],
            },
          }),
        ),
      ),
    );
    renderApp({ route: "/ideas" });
    const card = await screen.findByRole("region", { name: /RELIANCE/ });
    await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
    const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
    expect(await within(dialog).findByText("Kill switch is on.")).toBeVisible();
    expect(within(dialog).getByText("Stop would widen the risk.")).toBeVisible();
    expect(within(dialog).getByText("not passed")).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Approve paper order" })).toBeDisabled();
  });
});
