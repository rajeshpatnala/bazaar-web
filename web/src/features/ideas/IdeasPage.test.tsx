import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { DISCLAIMER } from "../../shared/disclaimer";
import { server } from "../../mocks/server";
import { renderApp } from "../../test/render";

describe("Ideas page", () => {
  it("shows cards with every field, badges and a Why panel", async () => {
    renderApp({ route: "/ideas" });
    const card = await screen.findByRole("region", { name: /RELIANCE/ });
    expect(within(card).getByText("Buy")).toBeInTheDocument();
    expect(within(card).getByText("₹2,850.00")).toBeInTheDocument();
    expect(within(card).getByText("₹2,800.00")).toBeInTheDocument();
    expect(within(card).getByText("₹2,950.00")).toBeInTheDocument();
    expect(within(card).getByText("₹1,000.00")).toBeInTheDocument();
    const why = within(card).getByRole("button", { name: "Why this idea?" });
    await userEvent.click(why);
    expect(within(card).getByText(/price has been moving up/)).toBeVisible();
    const low = screen.getByRole("region", { name: /TCS/ });
    expect(within(low).getByText("Sell")).toBeInTheDocument();
    expect(within(low).getByText("Low evidence")).toBeInTheDocument();
    expect(within(low).getByText("Data delayed")).toBeInTheDocument();
    expect(within(low).getByText(/Market: Sideways/)).toBeInTheDocument();
  });

  it("never uses certainty words and keeps the disclaimer", async () => {
    renderApp({ route: "/ideas" });
    await screen.findByRole("region", { name: /RELIANCE/ });
    const main = screen.getByRole("main");
    expect(main.textContent).not.toMatch(/\b(will|guaranteed|sure)\b/i);
    expect(screen.getByText(DISCLAIMER)).toBeVisible();
  });

  it("shows the empty state", async () => {
    server.use(http.get("/api/v1/ideas", () => HttpResponse.json({ ideas: [], as_of: "2026-11-19T10:00:00+05:30" })));
    renderApp({ route: "/ideas" });
    expect(await screen.findByText("No high-quality setups today")).toBeInTheDocument();
  });

  it("shows loading then an error with retry", async () => {
    server.use(http.get("/api/v1/ideas", () => new HttpResponse(null, { status: 500 })));
    renderApp({ route: "/ideas" });
    expect(await screen.findByText("Loading ideas...")).toBeInTheDocument();
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load ideas.");
    server.use(http.get("/api/v1/ideas", () => HttpResponse.json({ ideas: [], as_of: "2026-11-19T10:00:00+05:30" })));
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("No high-quality setups today")).toBeInTheDocument();
  });

  it("shows the as-of time in IST", async () => {
    renderApp({ route: "/ideas" });
    expect(await screen.findByText(/As of 19 Nov, 10:00 IST/)).toBeInTheDocument();
  });

  it("treats a wrongly shaped response as an error", async () => {
    server.use(http.get("/api/v1/ideas", () => HttpResponse.json({ nope: true })));
    renderApp({ route: "/ideas" });
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load ideas.");
  });
});
