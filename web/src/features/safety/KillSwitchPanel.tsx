import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { ApiError } from "../../api/client";
import { Button } from "../../shared/ui/Button";
import { Disclosure } from "../../shared/ui/Disclosure";
import { Modal } from "../../shared/ui/Modal";
import { ErrorView } from "../../shared/ui/StateViews";
import { Tag, type TagTone } from "../../shared/ui/Tag";
import { killSwitchQuery, useEngage, type KillSwitch } from "./queries";

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

/** Only a problem+json refusal is definite. Silence, a timeout (408) or a server fault may follow a saved write. */
function failureText(e: unknown): string {
  if (e instanceof ApiError && e.status > 0 && e.status !== 408 && !["unknown", "internal"].includes(e.code)) {
    return `Engage failed. ${e.message}`;
  }
  return "Engage: no clear answer from the server, so the result is unknown. Check the state above.";
}

export function KillSwitchPanel() {
  const query = useQuery(killSwitchQuery);
  const engage = useEngage();
  const [confirming, setConfirming] = useState(false);
  // A write in progress, then a failed read, win over cached data: old state is never shown as current.
  let state: State = "checking";
  if (engage.isPending) state = "sending";
  else if (query.isError) state = "unknown";
  else if (query.data) state = stateOf(query.data);
  const view = VIEW[state];
  const reason = state === "engaged" ? query.data?.reason : null;

  return (
    <section className={`safety-panel safety-${state}`} aria-label="Kill switch">
      <div className="safety-row">
        <p className="safety-status" aria-live="polite">
          <strong>Kill switch</strong> <Tag tone={view.tone}>{view.tag}</Tag> {view.text}
        </p>
        <div className="safety-actions">
          {/* Offered in every state: engaging is the safe direction and safe to repeat. */}
          <Button className="safety-stop" disabled={engage.isPending} onClick={() => setConfirming(true)}>
            Engage kill switch
          </Button>
        </div>
      </div>
      {reason && (
        <p className="safety-reason">
          Reason: <bdi>{reason}</bdi>
        </p>
      )}
      {query.error instanceof ApiError && query.error.status === 401 && <p>Your session has ended. Open Bazaar again from its login link.</p>}
      {engage.isError && <ErrorView message={failureText(engage.error)} />}
      <Disclosure summary="Why is there a kill switch?">
        It is your emergency stop. Use it if prices, orders or the app look wrong. It stays on after a restart. This screen cannot
        release it yet.
      </Disclosure>
      {confirming && (
        <Modal title="Engage the kill switch" onClose={() => setConfirming(false)}>
          <p>This is the emergency stop: new orders are blocked. It never places an order.</p>
          <p>This screen cannot release it yet.</p>
          <div className="dialog-actions">
            <Button variant="ghost" onClick={() => setConfirming(false)}>
              Cancel
            </Button>
            <Button
              className="safety-stop"
              onClick={() => {
                setConfirming(false);
                engage.mutate();
              }}
            >
              Engage now
            </Button>
          </div>
        </Modal>
      )}
    </section>
  );
}
