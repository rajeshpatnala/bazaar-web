import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { overviewFixture } from "../mocks/account";
import { server } from "../mocks/server";
import { DISCLAIMER } from "../shared/disclaimer";
import { renderApp } from "../test/render";

describe("Home overview", () => {
  it("shows the practice account figures in INR and IST, with badges", async () => {
    renderApp({ route: "/" });
    const card = await screen.findByRole("region", { name: /Practice account/ });
    expect(within(card).getByText("PAPER")).toBeInTheDocument();
    expect(within(card).getByText("₹1,00,000.00")).toBeInTheDocument();
    expect(within(card).getByText("-₹350.00")).toBeInTheDocument();
    expect(within(card).getByText("19 Nov, 10:00 IST")).toBeInTheDocument();
    expect(within(card).getByText(/Market: Sideways/)).toBeInTheDocument();
    expect(within(card).queryByText("Data delayed")).toBeNull();
    expect(screen.getByRole("main").textContent).not.toMatch(/\b(will|guaranteed|sure)\b/i);
    expect(screen.getByText(DISCLAIMER)).toBeVisible();
  });

  it("shows the data-delayed badge", async () => {
    server.use(http.get("/api/v1/overview", () => HttpResponse.json({ ...overviewFixture, data_delayed: true })));
    renderApp({ route: "/" });
    expect(await screen.findByText("Data delayed")).toBeInTheDocument();
  });

  it("shows the first-class empty state when there are no ideas", async () => {
    server.use(http.get("/api/v1/overview", () => HttpResponse.json({ ...overviewFixture, ideas_today: 0 })));
    renderApp({ route: "/" });
    expect(await screen.findByText("No high-quality setups today")).toBeInTheDocument();
  });

  it("shows an error with retry, and rejects a malformed overview", async () => {
    server.use(http.get("/api/v1/overview", () => HttpResponse.json({ nope: 1 })));
    renderApp({ route: "/" });
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load the overview.");
    server.use(http.get("/api/v1/overview", () => HttpResponse.json(overviewFixture)));
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("region", { name: /Practice account/ })).toBeInTheDocument();
  });

  it("does not call a non-paper mode a practice account", async () => {
    server.use(http.get("/api/v1/overview", () => HttpResponse.json({ ...overviewFixture, execution_mode: "semi_auto" })));
    renderApp({ route: "/" });
    const card = await screen.findByRole("region", { name: /Mode: semi_auto/ });
    expect(within(card).queryByText("PAPER")).toBeNull();
    expect(screen.queryByText(/Practice account/)).toBeNull();
  });

  it.each([
    ["a negative idea count", { ideas_today: -1 }],
    ["a negative position count", { open_positions: -2 }],
    ["an empty regime", { regime: "" }],
    ["a missing mode", { execution_mode: undefined }],
  ])("rejects %s", async (_name, patch) => {
    server.use(http.get("/api/v1/overview", () => HttpResponse.json({ ...overviewFixture, ...patch })));
    renderApp({ route: "/" });
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load the overview.");
  });

  it("loading state shows first", async () => {
    renderApp({ route: "/" });
    expect(await screen.findByText("Loading overview...")).toBeInTheDocument();
  });
});

describe("Navigation", () => {
  it("links Home and Ideas, marks the current page, and reaches the review flow", async () => {
    renderApp({ route: "/" });
    const nav = await screen.findByRole("navigation", { name: "Main" });
    expect(within(nav).getByRole("link", { name: "Home" })).toHaveAttribute("aria-current", "page");
    await userEvent.click(within(nav).getByRole("link", { name: "Ideas" }));
    expect(await screen.findByText("Ideas to consider today")).toBeInTheDocument();
    expect(within(nav).getByRole("link", { name: "Ideas" })).toHaveAttribute("aria-current", "page");
    const card = await screen.findByRole("region", { name: /RELIANCE/ });
    await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
    expect(await screen.findByRole("dialog", { name: "Review paper order" })).toBeInTheDocument();
    expect(screen.getByText(DISCLAIMER)).toBeVisible();
  });

  it("the overview link goes to ideas", async () => {
    renderApp({ route: "/" });
    await userEvent.click(await screen.findByRole("link", { name: /See ideas to consider/ }));
    expect(await screen.findByText("Ideas to consider today")).toBeInTheDocument();
  });
});
