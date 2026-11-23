import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { setCsrfToken } from "../api/client";
import { DISCLAIMER } from "../shared/disclaimer";
import { server } from "../mocks/server";
import { renderApp } from "../test/render";

// F13 T2 acceptance: cross-cutting checks over the real router on the MSW mock.
describe("F13 acceptance (T2)", () => {
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
    setCsrfToken("tok");
    const seen: string[] = [];
    const onStart = ({ request }: { request: Request }) => {
      if (request.method !== "GET") seen.push(`${request.method} ${new URL(request.url).pathname}`);
    };
    server.events.on("request:start", onStart);
    renderApp({ route: "/ideas" });
    const card = await screen.findByRole("region", { name: /RELIANCE/ });
    await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
    const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
    await userEvent.click(
      await within(dialog).findByRole("button", { name: "Approve paper order" }),
    );
    await within(dialog).findByRole("status");
    server.events.removeListener("request:start", onStart);
    expect(seen).toHaveLength(2);
    expect(seen[0]).toMatch(/^POST \/api\/v1\/ideas\/[^/]+\/paper-preview$/);
    expect(seen[1]).toMatch(/^POST \/api\/v1\/approvals\/[^/]+\/approve$/);
    expect(seen.join(" ")).not.toMatch(/place/i);
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

  it("a preview refusal (HTTP 403) leaves Approve unavailable", async () => {
    server.use(
      http.post("/api/v1/ideas/:id/paper-preview", () =>
        HttpResponse.json({ title: "Refused" }, { status: 403 }),
      ),
    );
    renderApp({ route: "/ideas" });
    const card = await screen.findByRole("region", { name: /RELIANCE/ });
    await userEvent.click(within(card).getByRole("button", { name: "Review paper order" }));
    const dialog = await screen.findByRole("dialog", { name: "Review paper order" });
    await within(dialog).findByRole("alert");
    expect(within(dialog).queryByRole("button", { name: "Approve paper order" })).toBeNull();
  });
});
