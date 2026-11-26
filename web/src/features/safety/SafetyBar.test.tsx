import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { delay, http, HttpResponse } from "msw";
import { createMemoryRouter, type RouteObject } from "react-router";
import { RouterProvider } from "react-router/dom";
import { Providers, makeQueryClient } from "../../app/providers";
import { routes } from "../../app/routes";
import { setCsrfToken } from "../../api/client";
import { DISCLAIMER } from "../../shared/disclaimer";
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

async function submitRelease(p: HTMLElement, phrase: string) {
  await userEvent.click(within(p).getByRole("button", { name: "Release..." }));
  const d = await screen.findByRole("dialog");
  await userEvent.type(within(d).getByLabelText("Release phrase"), phrase);
  await userEvent.click(within(d).getByRole("button", { name: "Confirm release" }));
}
async function submitEngage(p: HTMLElement, reason = "") {
  await userEvent.click(engageBtn(p));
  const d = await screen.findByRole("dialog");
  if (reason) await userEvent.type(within(d).getByLabelText("Reason (optional)"), reason);
  await userEvent.click(within(d).getByRole("button", { name: "Engage now" }));
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
    expect(within(p).queryByRole("button", { name: "Release..." })).toBeNull();
  });

  it("keeps Engage when the page below it crashes", async () => {
    const quiet = [
      vi.spyOn(console, "error").mockImplementation(() => {}),
      vi.spyOn(console, "warn").mockImplementation(() => {}),
    ];
    try {
      const Crash = (): never => {
        throw new Error("boom");
      };
      const [shell] = routes;
      const [boundary] = shell!.children!;
      const crashing: RouteObject = {
        path: "/",
        element: shell!.element,
        children: [{ errorElement: boundary!.errorElement, children: [{ index: true, element: <Crash /> }] }],
      };
      render(
        <Providers client={makeQueryClient()}>
          <RouterProvider router={createMemoryRouter([crashing])} />
        </Providers>,
      );
      await screen.findByText(/Something went wrong showing this page/);
      expect(engageBtn(await panel())).toBeEnabled();
    } finally {
      quiet.forEach((s) => s.mockRestore());
    }
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

  it("offers no release when the state is unreadable", async () => {
    read({ ...killSwitchOn, readable: false });
    renderApp();
    const p = await panel();
    await within(p).findByText(/cannot read the saved state/);
    expect(within(p).queryByRole("button", { name: "Release..." })).toBeNull();
  });

  it("is unknown while loading, with Engage already there", async () => {
    server.use(http.get(KS, () => delay("infinite")));
    renderApp();
    const p = await panel();
    expect(within(p).getByText("UNKNOWN")).toBeVisible();
    expect(engageBtn(p)).toBeEnabled();
  });

  it("shows unknown on a failed read and recovers with Check again", async () => {
    server.use(http.get(KS, () => HttpResponse.error()));
    renderApp();
    const p = await panel();
    await within(p).findByText(/Cannot confirm the state/);
    expect(engageBtn(p)).toBeEnabled();
    read(killSwitchOff);
    await userEvent.click(within(p).getByRole("button", { name: "Check again" }));
    await shows(p, "OFF");
  });

  it("drops a cached OFF when a later poll fails", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    renderApp();
    const p = await panel();
    await shows(p, "OFF");
    server.use(http.get(KS, () => HttpResponse.error()));
    await vi.advanceTimersByTimeAsync(5_100);
    await shows(p, "UNKNOWN");
    expect(within(p).queryByText("OFF")).toBeNull();
  });

  it("stops polling after a 401 and says the session ended", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    let reads = 0;
    server.use(
      http.get(KS, () => {
        reads += 1;
        return problem(401, "unauthenticated", "No session.");
      }),
    );
    renderApp();
    const p = await panel();
    await within(p).findByText(/session has ended/);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(reads).toBe(1);
  });

  it("renders a hostile reason as text only", async () => {
    const reason = '<img src=x onerror="alert(1)"><b>released</b>';
    read({ ...killSwitchOn, reason });
    renderApp();
    const p = await panel();
    expect(await within(p).findByText(reason)).toBeVisible();
    expect(p.querySelector("img, b")).toBeNull();
  });

  it("hides the stored reason when off", async () => {
    read({ ...killSwitchOff, reason: "released" });
    renderApp();
    const p = await panel();
    await shows(p, "OFF");
    expect(within(p).queryByText(/Reason:/)).toBeNull();
  });
});

describe("engage", () => {
  it("sends the reason with the CSRF header, then shows what the re-read says", async () => {
    let state = killSwitchOff;
    let sent: unknown;
    let csrf: string | null = null;
    server.use(
      http.get(KS, () => HttpResponse.json(state)),
      http.post(`${KS}/engage`, async ({ request }) => {
        sent = await request.json();
        csrf = request.headers.get("X-CSRF-Token");
        state = { ...killSwitchOn, reason: "bad prices" };
        return HttpResponse.json(state);
      }),
    );
    renderApp();
    const p = await panel();
    await shows(p, "OFF");
    await submitEngage(p, "bad prices");
    await shows(p, "ENGAGED");
    expect(sent).toEqual({ reason: "bad prices" });
    expect(csrf).toBe("test-csrf");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("sends nothing on Cancel", async () => {
    let posts = 0;
    server.use(
      http.post(`${KS}/engage`, () => {
        posts += 1;
        return HttpResponse.json(killSwitchOn);
      }),
    );
    renderApp();
    const p = await panel();
    await userEvent.click(engageBtn(p));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Cancel" }));
    expect(posts).toBe(0);
  });

  it("limits the reason by bytes, not characters", async () => {
    renderApp();
    const p = await panel();
    await userEvent.click(engageBtn(p));
    const d = await screen.findByRole("dialog");
    await userEvent.click(within(d).getByLabelText("Reason (optional)"));
    await userEvent.paste("é".repeat(251));
    expect(within(d).getByRole("button", { name: "Engage now" })).toBeDisabled();
    expect(within(d).getByRole("alert")).toHaveTextContent("too long");
  });

  it("shows the server's not-saved answer and re-reads the latched state", async () => {
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
    await submitEngage(p);
    expect(await within(p).findByRole("alert")).toHaveTextContent("Engage failed. Latched but not saved.");
    await shows(p, "ENGAGED");
  });

  it("is unknown, never the old OFF, when engage and the re-read both fail", async () => {
    renderApp();
    const p = await panel();
    await shows(p, "OFF");
    server.use(
      http.get(KS, () => HttpResponse.error()),
      http.post(`${KS}/engage`, () => HttpResponse.error()),
    );
    await submitEngage(p);
    expect(await within(p).findByRole("alert")).toHaveTextContent("result is unknown");
    await within(p).findByText(/Cannot confirm the state/);
    expect(within(p).queryByText("OFF")).toBeNull();
    expect(engageBtn(p)).toBeEnabled();
  });
});

describe("release", () => {
  it("needs a typed phrase: no POST from opening, a blank phrase or Enter", async () => {
    let posts = 0;
    read(killSwitchOn);
    server.use(
      http.post(`${KS}/release`, () => {
        posts += 1;
        return HttpResponse.json(killSwitchOff);
      }),
    );
    renderApp();
    const p = await panel();
    await userEvent.click(await within(p).findByRole("button", { name: "Release..." }));
    const d = await screen.findByRole("dialog");
    const confirm = within(d).getByRole("button", { name: "Confirm release" });
    const input = within(d).getByLabelText("Release phrase");
    expect(input).toHaveValue("");
    expect(confirm).toBeDisabled();
    await userEvent.type(input, "   ");
    expect(confirm).toBeDisabled();
    await userEvent.type(input, "words{Enter}");
    expect(posts).toBe(0);
  });

  it("sends the phrase exactly as typed and shows no OFF until the re-read says so", async () => {
    let sent: unknown;
    let state = killSwitchOn;
    let finish!: () => void;
    const gate = new Promise<void>((r) => (finish = r));
    server.use(
      http.get(KS, () => HttpResponse.json(state)),
      http.post(`${KS}/release`, async ({ request }) => {
        sent = await request.json();
        await gate;
        state = killSwitchOff;
        return HttpResponse.json(killSwitchOff);
      }),
    );
    renderApp();
    const p = await panel();
    await shows(p, "ENGAGED");
    await submitRelease(p, " My Phrase ");
    await within(p).findByText(/Sending your request/);
    expect(within(p).queryByText("OFF")).toBeNull();
    expect(engageBtn(p)).toBeEnabled();
    finish();
    await shows(p, "OFF");
    expect(sent).toEqual({ phrase: " My Phrase " });
  });

  it("does not trust a 200 'released' reply when the re-read still says engaged", async () => {
    let reads = 0;
    server.use(
      http.get(KS, () => {
        reads += 1;
        return HttpResponse.json(killSwitchOn);
      }),
    );
    renderApp();
    const p = await panel();
    await shows(p, "ENGAGED");
    await submitRelease(p, "some phrase");
    await waitFor(() => expect(reads).toBeGreaterThanOrEqual(2));
    await shows(p, "ENGAGED");
    expect(within(p).queryByText("OFF")).toBeNull();
  });

  it("is unknown, never released, when the reply is lost and the re-read fails", async () => {
    read(killSwitchOn);
    renderApp();
    const p = await panel();
    await shows(p, "ENGAGED");
    server.use(
      http.get(KS, () => HttpResponse.error()),
      http.post(`${KS}/release`, () => HttpResponse.error()),
    );
    await submitRelease(p, "some phrase");
    expect(await within(p).findByRole("alert")).toHaveTextContent("result is unknown");
    await within(p).findByText(/Cannot confirm the state/);
    expect(within(p).queryByText("OFF")).toBeNull();
    expect(within(p).queryByRole("button", { name: "Release..." })).toBeNull();
  });

  it("shows a wrong-phrase refusal, stays engaged, and does not keep the phrase", async () => {
    read(killSwitchOn);
    server.use(
      http.post(`${KS}/release`, () => problem(403, "release_phrase_mismatch", "The phrase does not match.")),
    );
    renderApp();
    const p = await panel();
    await shows(p, "ENGAGED");
    await submitRelease(p, "nope");
    expect(await within(p).findByRole("alert")).toHaveTextContent("Release failed. The phrase does not match.");
    await shows(p, "ENGAGED");
    await userEvent.click(within(p).getByRole("button", { name: "Release..." }));
    expect(within(await screen.findByRole("dialog")).getByLabelText("Release phrase")).toHaveValue("");
  });

  it("disables confirm if the state stops being confirmed while the dialog is open", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    read(killSwitchOn);
    renderApp();
    const p = await panel();
    await userEvent.click(await within(p).findByRole("button", { name: "Release..." }));
    const d = await screen.findByRole("dialog");
    await userEvent.type(within(d).getByLabelText("Release phrase"), "some phrase");
    server.use(http.get(KS, () => HttpResponse.error()));
    await vi.advanceTimersByTimeAsync(5_100);
    await waitFor(() => expect(within(d).getByRole("button", { name: "Confirm release" })).toBeDisabled());
  });
});

describe("alarms banner", () => {
  const alarm = { code: "square_off_blocked", message: "<b>Square-off</b> is blocked", since: "2026-11-19T04:30:00Z" };
  const alarms = (body: object) => server.use(http.get("/api/v1/alarms", () => HttpResponse.json(body)));

  it("shows nothing when the list is empty", async () => {
    renderApp();
    await shows(await panel(), "OFF");
    expect(screen.queryByText(/alarm/i)).toBeNull();
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
