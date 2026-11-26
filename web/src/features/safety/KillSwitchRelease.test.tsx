import { afterEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { setCsrfToken } from "../../api/client";
import { problemFixture } from "../../mocks/handlers";
import { killSwitchOff, killSwitchOn } from "../../mocks/safety";
import { server } from "../../mocks/server";
import { renderApp } from "../../test/render";

const KS = "/api/v1/killswitch";
const panel = () => screen.findByRole("region", { name: "Kill switch" });
const read = (body: object) => server.use(http.get(KS, () => HttpResponse.json(body)));
const shows = (p: HTMLElement, tag: string) => waitFor(() => expect(within(p).getByText(tag)).toBeVisible());

async function openRelease(p: HTMLElement) {
  await userEvent.click(await within(p).findByRole("button", { name: "Release..." }));
  const d = await screen.findByRole("dialog");
  return {
    d,
    input: within(d).getByLabelText("Release phrase"),
    confirm: within(d).getByRole("button", { name: "Confirm release" }),
  };
}
async function submitRelease(p: HTMLElement, phrase: string) {
  const { input, confirm } = await openRelease(p);
  await userEvent.type(input, phrase);
  await userEvent.click(confirm);
}
/** Counts release POSTs and answers each with "released". */
function countReleases() {
  const count = { posts: 0 };
  server.use(
    http.post(`${KS}/release`, () => {
      count.posts += 1;
      return HttpResponse.json(killSwitchOff);
    }),
  );
  return count;
}

afterEach(() => {
  setCsrfToken(null);
  vi.useRealTimers();
});

describe("release", () => {
  it("needs a typed phrase: no POST from opening, a blank phrase or Enter", async () => {
    read(killSwitchOn);
    const count = countReleases();
    renderApp();
    const { input, confirm } = await openRelease(await panel());
    expect(input).toHaveValue("");
    expect(confirm).toBeDisabled();
    await userEvent.type(input, "   ");
    expect(confirm).toBeDisabled();
    await userEvent.type(input, "words{Enter}");
    expect(count.posts).toBe(0);
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
    const { client } = renderApp();
    const p = await panel();
    await shows(p, "ENGAGED");
    await submitRelease(p, " My Phrase ");
    await within(p).findByText(/Sending your request/);
    expect(within(p).queryByText("OFF")).toBeNull();
    expect(within(p).getByRole("button", { name: "Engage kill switch" })).toBeEnabled();
    finish();
    await shows(p, "OFF");
    expect(sent).toEqual({ phrase: " My Phrase " });
    const kept = client.getMutationCache().getAll().map((m) => m.state.variables);
    expect(JSON.stringify(kept)).not.toContain("My Phrase");
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
    expect(await within(p).findByRole("alert")).toHaveTextContent("Release: no clear answer");
    await within(p).findByText(/Cannot confirm the state/);
    expect(within(p).queryByText("OFF")).toBeNull();
    expect(within(p).queryByRole("button", { name: "Release..." })).toBeNull();
  });

  it("shows a wrong-phrase refusal, stays engaged, and does not keep the phrase", async () => {
    read(killSwitchOn);
    const body = { ...problemFixture(403, "release_phrase_mismatch"), detail: "The phrase does not match." };
    server.use(http.post(`${KS}/release`, () => HttpResponse.json(body, { status: 403 })));
    renderApp();
    const p = await panel();
    await shows(p, "ENGAGED");
    await submitRelease(p, "nope");
    expect(await within(p).findByRole("alert")).toHaveTextContent("Release failed. The phrase does not match.");
    await shows(p, "ENGAGED");
    expect((await openRelease(p)).input).toHaveValue("");
  });

  it.each([
    ["the state stops being confirmed", () => HttpResponse.error()],
    ["the engage reason changes", () => HttpResponse.json({ ...killSwitchOn, reason: "daily loss" })],
  ])("disables confirm when %s under the open dialog", async (_n, later) => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    read(killSwitchOn);
    const count = countReleases();
    renderApp();
    const { d, input, confirm } = await openRelease(await panel());
    expect(d).toHaveTextContent("It was engaged for: manual stop");
    await userEvent.type(input, "some phrase");
    expect(confirm).toBeEnabled();
    server.use(http.get(KS, later));
    await vi.advanceTimersByTimeAsync(5_100);
    await waitFor(() => expect(confirm).toBeDisabled());
    expect(count.posts).toBe(0);
  });

  it("stays unconfirmed while a release is in flight, even after a later engage answers", async () => {
    read(killSwitchOn);
    let engaged = 0;
    let finish!: () => void;
    const gate = new Promise<void>((r) => (finish = r));
    server.use(
      http.post(`${KS}/release`, async () => {
        await gate;
        return HttpResponse.json(killSwitchOff);
      }),
      http.post(`${KS}/engage`, () => {
        engaged += 1;
        return HttpResponse.json(killSwitchOn);
      }),
    );
    renderApp();
    const p = await panel();
    await shows(p, "ENGAGED");
    await submitRelease(p, "some phrase");
    await userEvent.click(within(p).getByRole("button", { name: "Engage kill switch" }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Engage now" }));
    await waitFor(() => expect(engaged).toBe(1));
    await within(p).findByText(/Sending your request/);
    finish();
    await shows(p, "ENGAGED");
  });
});
