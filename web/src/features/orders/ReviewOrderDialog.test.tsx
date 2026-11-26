import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";
import { onSessionEnded, setCsrfToken } from "../../api/client";
import { DISCLAIMER } from "../../shared/disclaimer";
import { makePreview } from "../../mocks/orders";
import { server } from "../../mocks/server";
import { renderApp } from "../../test/render";

const PREVIEW = "/api/v1/ideas/:id/paper-preview";
const APPROVE = "/api/v1/approvals/:id/approve";
const previewWith = (over: Parameters<typeof makePreview>[0]) =>
  server.use(http.post(PREVIEW, () => HttpResponse.json(makePreview(over))));

async function openDialog() {
  renderApp({ route: "/ideas" });
  const card = await screen.findByRole("region", { name: /RELIANCE/ });
  await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
  return screen.findByRole("dialog", { name: "Review paper order" });
}
const approveBtn = (d: HTMLElement) => within(d).findByRole("button", { name: "Approve paper order" });

describe("Review paper order dialog", () => {
  afterEach(() => vi.useRealTimers());

  it("shows mode-driven PAPER text, every figure in INR, IST times, gate reasons, safe wording", async () => {
    const d = await openDialog();
    expect(await within(d).findByText("₹57,000.00")).toBeInTheDocument();
    expect(within(d).getByText("PAPER")).toBeInTheDocument();
    expect(within(d).getByText(/Practice order only/)).toBeInTheDocument();
    expect(within(d).getByText("20 shares")).toBeInTheDocument();
    expect(within(d).getByText("₹42.50")).toBeInTheDocument();
    expect(within(d).getByText("₹1,000.00")).toBeInTheDocument();
    expect(within(d).getByText("₹2,850.00")).toBeInTheDocument();
    expect(within(d).getByText("₹2,800.00")).toBeInTheDocument();
    expect(within(d).getByText("19 Nov, 10:00 IST")).toBeInTheDocument();
    expect(within(d).getByText("Position size is within your limit.")).toBeInTheDocument();
    expect(within(d).getByText("passed")).toBeInTheDocument();
    expect(within(d).getByRole("button", { name: "Not now" })).toBeInTheDocument();
    expect(d.textContent).not.toMatch(/\b(will|guaranteed|sure|live)\b/i);
    expect(screen.getByText(DISCLAIMER)).toBeVisible();
  });

  it("shows the low-evidence label and singular share", async () => {
    previewWith({ low_evidence: true, order: { ...makePreview().order, quantity: 1 } });
    const d = await openDialog();
    expect(await within(d).findByText("Low evidence")).toBeInTheDocument();
    expect(within(d).getByText("1 share")).toBeInTheDocument();
  });

  it("approve sends CSRF, Idempotency-Key, the approval id from the preview and the version", async () => {
    let seen: { csrf: string | null; key: string | null; url: string; body: unknown } | null = null;
    server.use(
      http.post(APPROVE, async ({ request }) => {
        seen = {
          csrf: request.headers.get("X-CSRF-Token"),
          key: request.headers.get("Idempotency-Key"),
          url: new URL(request.url).pathname,
          body: await request.json(),
        };
        return HttpResponse.json({ status: "paper_filled", message: "Recorded as a practice order." });
      }),
    );
    previewWith({ approval_id: "appr-77", version: 4 });
    const d = await openDialog();
    await userEvent.click(await approveBtn(d));
    expect(await within(d).findByRole("status")).toHaveTextContent("Paper order recorded.");
    expect(seen).toMatchObject({ csrf: "test-csrf", url: "/api/v1/approvals/appr-77/approve", body: { version: 4 } });
    expect((seen as unknown as { key: string }).key).toMatch(/^[0-9a-f-]{36}$/);
    expect(within(d).queryByRole("button", { name: "Approve paper order" })).toBeNull();
  });

  it("a double click sends one request", async () => {
    let calls = 0;
    server.use(
      http.post(APPROVE, async () => {
        calls += 1;
        await new Promise((r) => setTimeout(r, 50));
        return HttpResponse.json({ status: "paper_filled", message: "x" });
      }),
    );
    const d = await openDialog();
    const b = await approveBtn(d);
    await userEvent.dblClick(b);
    await within(d).findByRole("status");
    expect(calls).toBe(1);
  });

  it("nothing is approved until the user clicks", async () => {
    let calls = 0;
    server.use(http.post(APPROVE, () => ((calls += 1), HttpResponse.json({ status: "paper_filled", message: "x" }))));
    const d = await openDialog();
    await within(d).findByText("₹57,000.00");
    expect(calls).toBe(0);
  });

  it("after a timeout-like failure says it could not confirm and retries with the same key", async () => {
    const keys: (string | null)[] = [];
    let n = 0;
    server.use(
      http.post(APPROVE, ({ request }) => {
        keys.push(request.headers.get("Idempotency-Key"));
        n += 1;
        return n === 1 ? new HttpResponse(null, { status: 500 }) : HttpResponse.json({ status: "paper_filled", message: "ok" });
      }),
    );
    const d = await openDialog();
    await userEvent.click(await approveBtn(d));
    expect(await within(d).findByRole("alert")).toHaveTextContent("Could not confirm the result");
    expect(d.textContent).not.toMatch(/Nothing was recorded/);
    await userEvent.click(await approveBtn(d));
    await within(d).findByRole("status");
    expect(keys[0]).toBe(keys[1]);
    expect(keys).toHaveLength(2);
  });

  it("a network failure is also reported as unconfirmed", async () => {
    server.use(http.post(APPROVE, () => HttpResponse.error()));
    const d = await openDialog();
    await userEvent.click(await approveBtn(d));
    expect(await within(d).findByRole("alert")).toHaveTextContent("Could not confirm the result");
  });

  it("shows a problem response body, disables Approve and offers a re-preview", async () => {
    server.use(
      http.post(APPROVE, () =>
        HttpResponse.json({ code: "gate_blocked", detail: "Position size is over your limit.", status: 422, errors: [] }, { status: 422 }),
      ),
    );
    const d = await openDialog();
    await userEvent.click(await approveBtn(d));
    expect(await within(d).findByRole("alert")).toHaveTextContent("Position size is over your limit.");
    expect(within(d).getByRole("button", { name: "Approve paper order" })).toBeDisabled();
    await userEvent.click(within(d).getByRole("button", { name: "Review again" }));
    await waitFor(() => expect(within(d).getByRole("button", { name: "Approve paper order" })).toBeEnabled());
  });

  it("shows a rejected result from the server", async () => {
    server.use(http.post(APPROVE, () => HttpResponse.json({ status: "rejected", message: "Rule G14 blocked it." })));
    const d = await openDialog();
    await userEvent.click(await approveBtn(d));
    expect(await within(d).findByRole("status")).toHaveTextContent("Not approved. Rule G14 blocked it.");
  });

  it("missing CSRF token: error shown, nothing sent", async () => {
    let calls = 0;
    server.use(http.post(APPROVE, () => ((calls += 1), HttpResponse.json({ status: "paper_filled", message: "x" }))));
    const d = await openDialog();
    const b = await approveBtn(d);
    setCsrfToken(null);
    await userEvent.click(b);
    expect(await within(d).findByRole("alert")).toHaveTextContent("Missing security token");
    expect(calls).toBe(0);
  });

  it("a 401 on approve ends the session", async () => {
    let ended = 0;
    onSessionEnded(() => (ended += 1));
    server.use(http.post(APPROVE, () => HttpResponse.json({ code: "unauthorized", detail: "Session ended.", status: 401, errors: [] }, { status: 401 })));
    const d = await openDialog();
    await userEvent.click(await approveBtn(d));
    await within(d).findByRole("alert");
    expect(ended).toBeGreaterThan(0);
    onSessionEnded(null);
  });

  it("fails closed when the mode is not paper: no Approve button, no practice wording", async () => {
    previewWith({ execution_mode: "semi_auto" });
    const d = await openDialog();
    expect(await within(d).findByRole("alert")).toHaveTextContent("only handles practice orders");
    expect(within(d).queryByRole("button", { name: "Approve paper order" })).toBeNull();
    expect(within(d).queryByText(/No real money/)).toBeNull();
    expect(within(d).queryByText("PAPER")).toBeNull();
  });

  it("gate verdict other than allow disables Approve and shows the reasons", async () => {
    previewWith({ gate: { verdict: "block", reasons: ["Daily loss limit reached."] } });
    const d = await openDialog();
    expect(await within(d).findByText("Daily loss limit reached.")).toBeInTheDocument();
    expect(within(d).getByText("not passed")).toBeInTheDocument();
    expect(within(d).getByRole("button", { name: "Approve paper order" })).toBeDisabled();
    expect(within(d).getByRole("button", { name: "Review again" })).toBeInTheDocument();
  });

  it("an expired preview disables Approve and offers a re-preview", async () => {
    previewWith({ expires_at: "2020-01-01T00:00:00Z" });
    const d = await openDialog();
    expect(await within(d).findByText(/have expired/)).toBeInTheDocument();
    expect(within(d).getByRole("button", { name: "Approve paper order" })).toBeDisabled();
    expect(within(d).getByRole("button", { name: "Review again" })).toBeInTheDocument();
  });

  it("a malformed preview is an error with no Approve button", async () => {
    previewWith({ order: { ...makePreview().order, quantity: 0 } });
    const d = await openDialog();
    expect(await within(d).findByRole("alert")).toHaveTextContent("Could not load the order details.");
    expect(within(d).queryByRole("button", { name: "Approve paper order" })).toBeNull();
  });

  it("a failed preview shows an error with retry", async () => {
    server.use(http.post(PREVIEW, () => new HttpResponse(null, { status: 500 })));
    const d = await openDialog();
    expect(await within(d).findByRole("alert")).toHaveTextContent("Could not load the order details.");
  });

  it("Not now and Esc close the dialog", async () => {
    const d = await openDialog();
    await userEvent.click(within(d).getByRole("button", { name: "Not now" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    const card = screen.getByRole("region", { name: /RELIANCE/ });
    await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
    await screen.findByRole("dialog");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("Review again uses a new key for the new approval and keeps the could-not-confirm warning", async () => {
    const seen: { id: string; idem: string | null }[] = [];
    let previews = 0;
    server.use(
      http.post(PREVIEW, () => {
        previews += 1;
        return HttpResponse.json(
          previews === 1
            ? makePreview({ approval_id: "appr-1", expires_at: new Date(Date.now() + 700).toISOString() })
            : makePreview({ approval_id: "appr-2" }),
        );
      }),
      http.post(APPROVE, ({ request, params }) => {
        seen.push({ id: String(params.id), idem: request.headers.get("Idempotency-Key") });
        return seen.length === 1 ? new HttpResponse(null, { status: 500 }) : HttpResponse.json({ status: "paper_filled", message: "ok" });
      }),
    );
    const d = await openDialog();
    await userEvent.click(await approveBtn(d));
    await within(d).findByText(/Could not confirm the result/);
    await userEvent.click(await within(d).findByRole("button", { name: "Review again" }, { timeout: 3000 }));
    await waitFor(() => expect(previews).toBe(2));
    expect(within(d).getByText(/Could not confirm the result/)).toBeInTheDocument();
    const again = await waitFor(() => {
      const b = within(d).getByRole("button", { name: "Approve paper order" });
      expect(b).toBeEnabled();
      return b;
    });
    await userEvent.click(again);
    await within(d).findByRole("status");
    expect(seen.map((s) => s.id)).toEqual(["appr-1", "appr-2"]);
    expect(seen[0]?.idem).not.toBe(seen[1]?.idem);
    expect(within(d).queryByText(/Could not confirm the result/)).toBeNull();
  });

  it("still expires when its timer fires before the wall clock gets there", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    previewWith({ expires_at: new Date(Date.now() + 60_000).toISOString() });
    const d = await openDialog();
    await approveBtn(d);
    vi.setSystemTime(Date.now() - 100); // the wall clock now lags the timer
    await vi.advanceTimersByTimeAsync(60_200);
    expect(await within(d).findByRole("button", { name: "Review again" })).toBeVisible();
  });
});
