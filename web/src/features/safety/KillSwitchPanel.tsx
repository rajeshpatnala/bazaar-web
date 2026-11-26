import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ApiError } from "../../api/client";
import { Button } from "../../shared/ui/Button";
import { Disclosure } from "../../shared/ui/Disclosure";
import { ErrorView } from "../../shared/ui/StateViews";
import { Tag, type TagTone } from "../../shared/ui/Tag";
import { EngageDialog, ReleaseDialog } from "./KillSwitchDialogs";
import { killSwitchQuery, useEngage, useRelease, type KillSwitch } from "./queries";

type State = "checking" | "sending" | "unknown" | "unreadable" | "engaged" | "off";

const VIEW: Record<State, { tag: string; tone: TagTone; text: string }> = {
  checking: { tag: "UNKNOWN", tone: "warn", text: "Checking the kill switch. Do not assume it is off." },
  sending: { tag: "UNKNOWN", tone: "warn", text: "Sending your request. The state is not confirmed yet." },
  unknown: {
    tag: "UNKNOWN",
    tone: "warn",
    text: "Cannot confirm the state. Do not assume trading is stopped; engage the switch if you want it stopped.",
  },
  unreadable: {
    tag: "UNKNOWN",
    tone: "warn",
    text: "The server cannot read the saved state, so it treats the switch as engaged.",
  },
  engaged: { tag: "ENGAGED", tone: "sell", text: "New orders are blocked." },
  off: { tag: "OFF", tone: "neutral", text: "The kill switch is not blocking orders. Other safety checks still apply." },
};

/** "off" needs every flag to agree; any doubt reads as engaged or unknown. */
function stateOf(d: KillSwitch): State {
  if (!d.readable) return "unreadable";
  return d.engaged || d.latched ? "engaged" : "off";
}

/** A problem+json answer is definite; silence or a bare failure means the outcome is unknown. */
function failureText(e: unknown, action: "Engage" | "Release"): string {
  if (e instanceof ApiError && e.status > 0 && e.code !== "unknown") {
    return `${action} failed. ${e.message}`;
  }
  return `${action}: no clear answer from the server, so the result is unknown. Check the state above.`;
}

export function KillSwitchPanel() {
  const query = useQuery(killSwitchQuery);
  const engage = useEngage();
  const release = useRelease();
  const [dialog, setDialog] = useState<"engage" | "release" | null>(null);
  // A write in progress, then a failed read, win over cached data: old state is never shown as current.
  let state: State = "checking";
  if (engage.isPending || release.isPending) state = "sending";
  else if (query.isError) state = "unknown";
  else if (query.data) state = stateOf(query.data);
  const view = VIEW[state];
  const reason = state === "engaged" ? query.data?.reason : null;
  const sessionEnded = query.error instanceof ApiError && query.error.status === 401;
  const close = () => setDialog(null);

  return (
    <section className={`safety-panel safety-${state}`} aria-label="Kill switch">
      <div className="safety-row">
        <p className="safety-status" aria-live="polite">
          <strong>Kill switch</strong> <Tag tone={view.tone}>{view.tag}</Tag> {view.text}
        </p>
        <div className="safety-actions">
          {state === "unknown" && (
            <Button variant="ghost" onClick={() => void query.refetch()} disabled={query.isFetching}>
              Check again
            </Button>
          )}
          {state === "engaged" && (
            <Button variant="ghost" onClick={() => setDialog("release")}>
              Release...
            </Button>
          )}
          {/* Offered in every state: engaging is the safe direction and safe to repeat. */}
          <Button className="safety-stop" disabled={engage.isPending} onClick={() => setDialog("engage")}>
            Engage kill switch
          </Button>
        </div>
      </div>
      {reason && (
        <p className="safety-reason">
          Reason: <bdi>{reason}</bdi>
        </p>
      )}
      {sessionEnded && <p>Your session has ended. Open Bazaar again from its login link.</p>}
      {engage.isError && <ErrorView message={failureText(engage.error, "Engage")} />}
      {release.isError && <ErrorView message={failureText(release.error, "Release")} />}
      <Disclosure summary="Why is there a kill switch?">
        It is your emergency stop. Use it if prices, orders or the app look wrong. It stays on after a restart, and you
        turn it off by typing the release phrase.
      </Disclosure>
      {dialog === "engage" && (
        <EngageDialog
          onCancel={close}
          onConfirm={(text) => {
            close();
            release.reset();
            engage.mutate(text);
          }}
        />
      )}
      {dialog === "release" && (
        <ReleaseDialog
          onCancel={close}
          confirmable={state === "engaged"}
          onConfirm={(text) => {
            close();
            engage.reset();
            release.mutate(text);
          }}
        />
      )}
    </section>
  );
}
