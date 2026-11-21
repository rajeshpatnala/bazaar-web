import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { ApiError } from "../../api/client";
import { formatIstIso } from "../../shared/lib/format-ist";
import { Button } from "../../shared/ui/Button";
import { Modal } from "../../shared/ui/Modal";
import { Money } from "../../shared/ui/Money";
import { ErrorView, LoadingView } from "../../shared/ui/StateViews";
import { Tag } from "../../shared/ui/Tag";
import { approvePaperOrder, previewQuery } from "./queries";

/** Outcome unknown (timeout, network, 5xx): say so, never "nothing was recorded". */
const isAmbiguous = (e: unknown) =>
  !(e instanceof ApiError) || (e.status === 0 && e.code !== "csrf") || e.status >= 500;

/**
 * Practice order review. The Approve click is the only thing that calls the server.
 * Fails closed: no Approve button unless the server says execution_mode is "paper".
 */
export function ReviewOrderDialog({ ideaId, onClose }: { ideaId: string; onClose: () => void }) {
  const preview = useQuery(previewQuery(ideaId));
  const keys = useRef(new Map<string, string>()); // one idempotency key per approval_id, reused on retry
  const [unconfirmed, setUnconfirmed] = useState(false); // stays until an approval is confirmed
  const [now, setNow] = useState(() => Date.now());
  const p = preview.data;
  const approve = useMutation({
    mutationFn: () => {
      const id = p!.approval_id;
      if (!keys.current.has(id)) keys.current.set(id, crypto.randomUUID());
      return approvePaperOrder(p!, keys.current.get(id)!);
    },
    retry: false,
    onSuccess: () => setUnconfirmed(false),
    onError: (e) => {
      if (isAmbiguous(e)) setUnconfirmed(true);
    },
  });
  const expiresMs = p ? Date.parse(p.expires_at) : null;

  useEffect(() => {
    if (expiresMs === null) return;
    const t = setTimeout(() => setNow(Date.now()), Math.min(Math.max(expiresMs - Date.now(), 0), 2 ** 31 - 1));
    return () => clearTimeout(t);
  }, [expiresMs]);

  const expired = expiresMs !== null && now >= expiresMs;
  const paper = p?.execution_mode === "paper";
  const allowed = p?.gate.verdict === "allow";
  const err = approve.error;
  const definitive = approve.isError && !isAmbiguous(err); // the server answered: this draft is dead
  const canApprove = !!p && paper && allowed && !expired && !definitive && !approve.isPending;
  const needsRepreview = !!p && paper && (expired || !allowed || definitive);

  const repreview = () => {
    approve.reset();
    setNow(Date.now());
    void preview.refetch();
  };

  return (
    <Modal title="Review paper order" onClose={onClose}>
      {p && paper && (
        <p>
          <Tag tone="warn">PAPER</Tag> Practice order only. No real money is used.
        </p>
      )}
      {p && !paper && (
        <ErrorView message={`This screen only handles practice orders. The server reports mode "${p.execution_mode}", so nothing can be approved here.`} />
      )}
      {preview.isPending && <LoadingView label="Loading order details..." />}
      {preview.isError && <ErrorView message="Could not load the order details." onRetry={() => void preview.refetch()} />}
      {p && paper && (
        <>
          <dl className="idea-figures">
            <dt>Max loss</dt>
            <dd>
              <Money value={p.order.max_loss} />
            </dd>
            <dt>Instrument</dt>
            <dd>
              {p.order.symbol} ({p.order.side === "buy" ? "Buy" : "Sell"}){" "}
              {p.low_evidence && <Tag tone="warn">Low evidence</Tag>}
            </dd>
            <dt>Entry</dt>
            <dd>
              <Money value={p.order.entry} />
            </dd>
            <dt>Stop</dt>
            <dd>
              <Money value={p.order.stop} />
            </dd>
            <dt>Quantity</dt>
            <dd>
              {p.order.quantity} {p.order.quantity === 1 ? "share" : "shares"}
            </dd>
            <dt>Total</dt>
            <dd>
              <Money value={p.order.total} />
            </dd>
            <dt>Estimated charges</dt>
            <dd>
              <Money value={p.order.charges} />
            </dd>
            <dt>Prices as of</dt>
            <dd>{formatIstIso(p.as_of)}</dd>
            <dt>Valid until</dt>
            <dd>{formatIstIso(p.expires_at)}</dd>
          </dl>
          <p>
            Safety checks: <strong>{allowed ? "passed" : "not passed"}</strong>
          </p>
          {p.gate.reasons.length > 0 && (
            <ul>
              {p.gate.reasons.map((r, n) => (
                <li key={`${n}-${r}`}>{r}</li>
              ))}
            </ul>
          )}
          {expired && <p role="alert">These figures have expired. Review them again before approving.</p>}
        </>
      )}
      {approve.data && (
        <p role="status">
          {approve.data.status === "paper_filled" ? "Paper order recorded. " : "Not approved. "}
          {approve.data.message}
        </p>
      )}
      {unconfirmed && (
        <ErrorView message="Could not confirm the result of an earlier attempt. Check Orders before approving again." />
      )}
      {approve.isError && !isAmbiguous(err) && <ErrorView message={(err as ApiError).message} />}
      <div className="dialog-actions">
        <Button variant="ghost" onClick={onClose}>
          {approve.data ? "Close" : "Not now"}
        </Button>
        {needsRepreview && !approve.data && (
          <Button variant="ghost" onClick={repreview} disabled={preview.isFetching}>
            Review again
          </Button>
        )}
        {p && paper && !approve.data && (
          <Button disabled={!canApprove} onClick={() => approve.mutate()}>
            Approve paper order
          </Button>
        )}
      </div>
    </Modal>
  );
}
