import { useId, useState } from "react";
import { Button } from "../../shared/ui/Button";
import { Modal } from "../../shared/ui/Modal";

const MAX_BYTES = 500; // the server's cap, in bytes
const tooLong = (s: string) => new TextEncoder().encode(s).length > MAX_BYTES;

/** The phrase is sent exactly as typed; only the server decides whether it matches. */
export function ReleaseDialog({
  onCancel,
  onConfirm,
  reason,
  confirmable,
}: {
  onCancel: () => void;
  onConfirm: (phrase: string) => void;
  reason: string | null;
  confirmable: boolean;
}) {
  const [phrase, setPhrase] = useState("");
  const id = useId();
  const long = tooLong(phrase);
  return (
    <Modal title="Release the kill switch" onClose={onCancel}>
      <p>
        Releasing removes the emergency stop, so new orders can go ahead again. Each order still needs the safety checks
        and your approval.
      </p>
      {reason && (
        <p className="safety-reason">
          It was engaged for: <bdi>{reason}</bdi>
        </p>
      )}
      <label htmlFor={id}>Release phrase</label>
      <input
        id={id}
        className="safety-input"
        value={phrase}
        autoComplete="off"
        autoCapitalize="off"
        spellCheck={false}
        aria-describedby={`${id}-hint`}
        onChange={(e) => setPhrase(e.target.value)}
      />
      <p id={`${id}-hint`} className="lbl">
        Type it exactly as set in config/risk.toml (kill_switch_release_phrase).
      </p>
      {long && <p role="alert">That is too long to be the phrase.</p>}
      {!confirmable && (
        <p role="alert">The kill switch state changed or is no longer confirmed. Close this and check it again.</p>
      )}
      <div className="dialog-actions">
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button disabled={!confirmable || long || phrase.trim() === ""} onClick={() => onConfirm(phrase)}>
          Confirm release
        </Button>
      </div>
    </Modal>
  );
}
