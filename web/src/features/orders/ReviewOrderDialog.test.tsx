import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { DISCLAIMER } from "../../shared/disclaimer";
import { server } from "../../mocks/server";
import { renderApp } from "../../test/render";

async function openDialog() {
  renderApp({ route: "/ideas" });
  const card = await screen.findByRole("region", { name: /RELIANCE/ });
  await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
  return screen.findByRole("dialog", { name: "Review paper order" });
}

describe("Review paper order dialog", () => {
  it("shows PAPER, figures in INR and IST time, with safe wording", async () => {
    const d = await openDialog();
    expect(await within(d).findByText("₹57,000.00")).toBeInTheDocument();
    expect(within(d).getByText("PAPER")).toBeInTheDocument();
    expect(within(d).getByText("20 shares")).toBeInTheDocument();
    expect(within(d).getByText("₹42.50")).toBeInTheDocument();
    expect(within(d).getByText("₹1,000.00")).toBeInTheDocument();
    expect(within(d).getByText("19 Nov, 10:00 IST")).toBeInTheDocument();
    expect(within(d).getByRole("button", { name: "Not now" })).toBeInTheDocument();
    expect(d.textContent).not.toMatch(/\b(will|guaranteed|sure|live)\b/i);
    expect(screen.getByText(DISCLAIMER)).toBeVisible();
  });

  it("approve sends the CSRF header and shows the result", async () => {
    let header: string | null = null;
    server.use(
      http.post("/api/v1/approvals/:id/approve", ({ request }) => {
        header = request.headers.get("X-CSRF-Token");
        return HttpResponse.json({ status: "paper_filled", message: "Recorded as a practice order." });
      }),
    );
    const d = await openDialog();
    const approve = await within(d).findByRole("button", { name: "Approve paper order" });
    await userEvent.click(approve);
    expect(await within(d).findByRole("status")).toHaveTextContent("Paper order recorded.");
    expect(header).toBe("test-csrf");
    expect(within(d).queryByRole("button", { name: "Approve paper order" })).toBeNull();
  });

  it("nothing is sent until the user clicks", async () => {
    let calls = 0;
    server.use(
      http.post("/api/v1/approvals/:id/approve", () => {
        calls += 1;
        return HttpResponse.json({ status: "paper_filled", message: "x" });
      }),
    );
    const d = await openDialog();
    await within(d).findByText("₹57,000.00");
    expect(calls).toBe(0);
  });

  it("shows an error and records nothing when approval fails", async () => {
    server.use(http.post("/api/v1/approvals/:id/approve", () => new HttpResponse(null, { status: 500 })));
    const d = await openDialog();
    await userEvent.click(await within(d).findByRole("button", { name: "Approve paper order" }));
    expect(await within(d).findByRole("alert")).toHaveTextContent("Nothing was recorded.");
  });

  it("shows an error with a disabled Approve when the preview fails", async () => {
    server.use(http.get("/api/v1/ideas/:id/paper-preview", () => new HttpResponse(null, { status: 500 })));
    const d = await openDialog();
    expect(await within(d).findByRole("alert")).toHaveTextContent("Could not load the order details.");
    expect(within(d).getByRole("button", { name: "Approve paper order" })).toBeDisabled();
  });

  it("Not now and Esc close the dialog", async () => {
    const d = await openDialog();
    await userEvent.click(within(d).getByRole("button", { name: "Not now" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(screen.getAllByRole("button", { name: "Review paper order" })[0]!);
    await screen.findByRole("dialog");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
