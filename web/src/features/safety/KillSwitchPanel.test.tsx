import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { delay, http, HttpResponse } from "msw";
import { createMemoryRouter } from "react-router";
import { RouterProvider } from "react-router/dom";
import { Providers, makeQueryClient } from "../../app/providers";
import { routes } from "../../app/routes";
import { setCsrfToken } from "../../api/client";
import { problemFixture } from "../../mocks/handlers";
import { killSwitchOff, killSwitchOn } from "../../mocks/safety";
import { server } from "../../mocks/server";
import { renderApp } from "../../test/render";

const KS = "/api/v1/killswitch";
const panel = () => screen.findByRole("region", { name: "Kill switch" });
const engageBtn = (p: HTMLElement) => within(p).getByRole("button", { name: "Engage kill switch" });
const read = (body: object) => server.use(http.get(KS, () => HttpResponse.json(body)));
const problem = (status: number, code: string, detail: string) =>
  HttpResponse.json({ ...problemFixture(status, code), detail }, { status });
const shows = (p: HTMLElement, tag: string) => waitFor(() => expect(within(p).getByText(tag)).toBeVisible());

const clickInDialog = async (name: string) =>
  userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name }));
async function engage(p: HTMLElement) {
  await userEvent.click(engageBtn(p));
  await clickInDialog("Engage now");
}

afterEach(() => {
  setCsrfToken(null);
  vi.useRealTimers();
});

describe("kill switch state", () => {
  it.each(["/", "/ideas", "/nope"])("offers Engage on %s", async (route) => {
    renderApp({ route });
    const p = await panel();
    await shows(p, "OFF");
    expect(engageBtn(p)).toBeEnabled();
  });

  it("keeps Engage when the page below it crashes", async () => {
    const Crash = (): never => {
      throw new Error("boom");
    };
    const [shell] = routes;
    const children = [{ errorElement: shell!.children![0]!.errorElement, children: [{ index: true, element: <Crash /> }] }];
    const router = createMemoryRouter([{ path: "/", element: shell!.element, children }]);
    render(
      <Providers client={makeQueryClient()}>
        <RouterProvider router={router} />
      </Providers>,
    );
    await screen.findByText(/Something went wrong showing this page/);
    expect(engageBtn(await panel())).toBeEnabled();
  });

  it.each([
    ["engaged", killSwitchOn, "ENGAGED"],
    ["latched only", { ...killSwitchOff, latched: true }, "ENGAGED"],
    ["unreadable", { ...killSwitchOff, readable: false }, "UNKNOWN"],
    ["a wrong-typed flag", { ...killSwitchOff, engaged: "no" }, "UNKNOWN"],
    ["a missing flag", { engaged: false, readable: true }, "UNKNOWN"],
    ["a non-string reason", { ...killSwitchOff, reason: 4 }, "UNKNOWN"],
  ])("never shows OFF when %s, and keeps Engage", async (_n, body, tag) => {
    read(body);
    renderApp();
    const p = await panel();
    await waitFor(() => expect(within(p).queryByText(/Checking/)).toBeNull());
    expect(within(p).getByText(tag)).toBeVisible();
    expect(within(p).queryByText("OFF")).toBeNull();
    expect(engageBtn(p)).toBeEnabled();
  });

  it("is unknown while loading, with Engage already there", async () => {
    server.use(http.get(KS, () => delay("infinite")));
    renderApp();
    const p = await panel();
    expect(within(p).getByText("UNKNOWN")).toBeVisible();
    expect(engageBtn(p)).toBeEnabled();
  });

  it("drops a cached OFF when a later poll fails, and recovers on a later poll", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderApp();
    const p = await panel();
    await shows(p, "OFF");
    server.use(http.get(KS, () => HttpResponse.error()));
    await vi.advanceTimersByTimeAsync(5_100);
    await shows(p, "UNKNOWN");
    expect(within(p).queryByText("OFF")).toBeNull();
    read(killSwitchOff);
    await vi.advanceTimersByTimeAsync(5_100);
    await shows(p, "OFF");
  });

  it("keeps polling in a hidden tab and stops after a 401", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const hidden = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    try {
      renderApp();
      const p = await panel();
      await shows(p, "OFF");
      let reads = 0;
      server.use(
        http.get(KS, () => {
          reads += 1;
          return problem(401, "unauthenticated", "No session.");
        }),
      );
      await vi.advanceTimersByTimeAsync(5_100);
      await within(p).findByText(/session has ended/);
      await vi.advanceTimersByTimeAsync(20_000);
      expect(reads).toBe(1);
    } finally {
      hidden.mockRestore();
    }
  });

  it("renders a hostile reason as text only, and hides it when off", async () => {
    const reason = '<img src=x onerror="alert(1)"><b>released</b>';
    read({ ...killSwitchOn, reason });
    const { unmount } = renderApp();
    const p = await panel();
    expect(await within(p).findByText(reason)).toBeVisible();
    expect(p.querySelector("img, b")).toBeNull();
    unmount();
    read({ ...killSwitchOff, reason });
    renderApp();
    const off = await panel();
    await shows(off, "OFF");
    expect(within(off).queryByText(reason)).toBeNull();
  });
});

describe("engage", () => {
  it("posts once, only after the confirm step, with CSRF, and shows only what the re-read says", async () => {
    let state = killSwitchOff;
    let posts = 0;
    let sent: unknown;
    let csrf: string | null = null;
    let finish!: () => void;
    const gate = new Promise<void>((r) => (finish = r));
    server.use(
      http.get(KS, () => HttpResponse.json(state)),
      http.post(`${KS}/engage`, async ({ request }) => {
        posts += 1;
        sent = await request.json();
        csrf = request.headers.get("X-CSRF-Token");
        await gate;
        state = killSwitchOn;
        return HttpResponse.json(state);
      }),
    );
    renderApp();
    const p = await panel();
    await shows(p, "OFF");
    await userEvent.click(engageBtn(p));
    await clickInDialog("Cancel");
    expect(screen.queryByRole("dialog")).toBeNull();
    await engage(p);
    await within(p).findByText(/Sending your request/);
    expect(within(p).queryByText("OFF")).toBeNull();
    finish();
    await shows(p, "ENGAGED");
    expect(sent).toEqual({ reason: "" });
    expect(posts).toBe(1);
    expect(csrf).toBe("test-csrf");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows the server's not-saved refusal and re-reads the latched state", async () => {
    let state = killSwitchOff;
    server.use(
      http.get(KS, () => HttpResponse.json(state)),
      http.post(`${KS}/engage`, () => {
        state = killSwitchOn;
        return problem(503, "kill_switch_not_saved", "Latched but not saved. Retry engaging.");
      }),
    );
    renderApp();
    const p = await panel();
    await shows(p, "OFF");
    await engage(p);
    expect(await within(p).findByRole("alert")).toHaveTextContent("Engage failed. Latched but not saved.");
    await shows(p, "ENGAGED");
  });

  it.each([
    ["no answer", () => HttpResponse.error()],
    ["a server timeout", () => problem(408, "request_timeout", "The request took too long.")],
    ["a server fault", () => problem(500, "internal", "Something went wrong.")],
  ])("says the result is unknown after %s, never the old OFF", async (_n, reply) => {
    renderApp();
    const p = await panel();
    await shows(p, "OFF");
    server.use(
      http.get(KS, () => HttpResponse.error()),
      http.post(`${KS}/engage`, reply),
    );
    await engage(p);
    expect(await within(p).findByRole("alert")).toHaveTextContent("result is unknown");
    await within(p).findByText(/Cannot confirm the state/);
    expect(within(p).queryByText("OFF")).toBeNull();
    expect(engageBtn(p)).toBeEnabled();
  });
});
