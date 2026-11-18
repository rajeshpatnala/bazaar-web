import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import { http, HttpResponse } from "msw/http";
import { DISCLAIMER } from "../shared/disclaimer";
import { server } from "../mocks/server";
import { renderApp } from "../test/render";

describe("AppShell", () => {
  it.each(["/", "/scanner", "/anything/else"])(
    "renders the disclaimer on %s, also when the API is down",
    async (path) => {
      server.use(http.all("/api/*", () => HttpResponse.error()));
      renderApp({ route: path });
      expect(await screen.findByText(DISCLAIMER)).toBeVisible();
      expect(screen.getByRole("contentinfo")).toHaveTextContent(DISCLAIMER);
    },
  );

  it("renders the home screen", async () => {
    renderApp();
    expect(await screen.findByRole("heading", { name: "Market overview" })).toBeVisible();
  });

  it("renders a not-found page inside the shell", async () => {
    renderApp({ route: "/nope" });
    expect(await screen.findByText("Page not found.")).toBeVisible();
  });
});
