import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it } from "vitest";
import { setCsrfToken } from "../api/client";
import { DISCLAIMER } from "../shared/disclaimer";
import { server } from "../mocks/server";
import { renderApp } from "../test/render";

// F13 T2 acceptance: cross-cutting checks over the real router on the MSW mock.
describe("F13 acceptance (T2)", () => {
  afterEach(() => setCsrfToken(null));

  it.each(["/", "/ideas", "/nope/deeper"])(
    "route %s shows the disclaimer text exactly once",
    async (route) => {
      renderApp({ route });
      await screen.findByText(DISCLAIMER);
      expect(screen.getAllByText(DISCLAIMER)).toHaveLength(1);
    },
  );

  it("no mode control anywhere: no switch, radio, checkbox, select or live-named button", async () => {
    for (const route of ["/", "/ideas"]) {
      const { unmount } = renderApp({ route });
      await screen.findByText(DISCLAIMER);
      await screen.findAllByRole("link");
      expect(screen.queryAllByRole("switch")).toHaveLength(0);
      expect(screen.queryAllByRole("radio")).toHaveLength(0);
      expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
      expect(screen.queryAllByRole("combobox")).toHaveLength(0);
      expect(screen.queryByRole("button", { name: /live|mode/i })).toBeNull();
      unmount();
    }
  });

  it("ideas -> review -> approve calls only preview and approve (no place-order route)", async () => {
    const seen: string[] = [];
    let approveCsrf: string | null = null;
    const onStart = ({ request }: { request: Request }) => {
      if (request.method === "GET") return;
      const path = new URL(request.url).pathname;
      seen.push(`${request.method} ${path}`);
      if (path.endsWith("/approve")) approveCsrf = request.headers.get("X-CSRF-Token");
    };
    server.events.on("request:start", onStart);
    try {
      renderApp({ route: "/ideas" });
      const card = await screen.findByRole("region", { name: /RELIANCE/ });
      await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
      const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
      const approveBtn = await within(dialog).findByRole("button", { name: "Approve paper order" });
      // Figures the user approves (preview fixture): mode, instrument, quantity, prices, charges.
      expect(within(dialog).getByText(/Practice order only/)).toBeVisible();
      expect(within(dialog).getByText("PAPER")).toBeVisible();
      expect(within(dialog).getByText(/RELIANCE/)).toBeVisible();
      expect(within(dialog).getByText(/20 shares/)).toBeVisible();
      for (const amt of ["2,850.00", "2,800.00", "1,000.00", "42.50", "57,000.00"]) {
        expect(within(dialog).getByText(new RegExp(amt.replace(".", "\\.")))).toBeVisible();
      }
      await userEvent.click(approveBtn);
      const status = await within(dialog).findByRole("status");
      expect(status).toHaveTextContent(
        "Paper order recorded. Recorded as a practice order; no real money was used.",
      );
    } finally {
      server.events.removeListener("request:start", onStart);
    }
    expect(seen).toHaveLength(2);
    expect(seen[0]).toMatch(/^POST \/api\/v1\/ideas\/[^/]+\/paper-preview$/);
    expect(seen[1]).toBe("POST /api/v1/approvals/appr-1/approve");
    expect(seen.join(" ")).not.toMatch(/place/i);
    expect(approveCsrf).toBe("test-csrf"); // session token from renderApp
  });

  it("review dialog: labelled modal, Tab and Shift+Tab stay inside, Esc closes and returns focus, disclaimer visible", async () => {
    renderApp({ route: "/ideas" });
    const card = await screen.findByRole("region", { name: /RELIANCE/ });
    const opener = within(card).getByRole("button", { name: "Review paper order" });
    await userEvent.click(opener);
    const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
    await within(dialog).findByRole("button", { name: "Approve paper order" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    for (let i = 0; i < 6; i++) {
      await userEvent.tab();
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
    for (let i = 0; i < 6; i++) {
      await userEvent.tab({ shift: true });
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
    expect(screen.getByText(DISCLAIMER)).toBeVisible();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(opener).toHaveFocus();
  });

  it("a preview refusal (403 gate rule) shows the fixed load error, offers no Approve and never calls approve", async () => {
    let approveCalls = 0;
    server.use(
      http.post("/api/v1/ideas/:id/paper-preview", () =>
        HttpResponse.json(
          {
            code: "gate_refused",
            title: "Refused",
            status: 403,
            detail: "Daily loss limit reached.",
            errors: [],
          },
          { status: 403, headers: { "Content-Type": "application/problem+json" } },
        ),
      ),
      http.post("/api/v1/approvals/:id/approve", () => {
        approveCalls++;
        return HttpResponse.json({ status: "paper_filled", message: "x" });
      }),
    );
    renderApp({ route: "/ideas" });
    const card = await screen.findByRole("region", { name: /RELIANCE/ });
    await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
    const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
    // The preview screen shows a fixed message (it does not surface the rule text).
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "Could not load the order details.",
    );
    expect(within(dialog).queryByRole("button", { name: "Approve paper order" })).toBeNull();
    expect(approveCalls).toBe(0);
  });

  it("an approve refusal shows WHICH rule refused (problem detail) and no success status", async () => {
    server.use(
      http.post("/api/v1/approvals/:id/approve", () =>
        HttpResponse.json(
          {
            code: "daily_loss_lockout",
            title: "Refused",
            status: 403,
            detail: "Daily loss limit reached.",
            errors: [],
          },
          { status: 403, headers: { "Content-Type": "application/problem+json" } },
        ),
      ),
    );
    renderApp({ route: "/ideas" });
    const card = await screen.findByRole("region", { name: /RELIANCE/ });
    await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
    const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
    await userEvent.click(
      await within(dialog).findByRole("button", { name: "Approve paper order" }),
    );
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Daily loss limit reached.");
    expect(within(dialog).queryByRole("status")).toBeNull();
  });
});
