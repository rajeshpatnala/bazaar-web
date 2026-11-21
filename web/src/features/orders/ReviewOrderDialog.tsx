import { useMutation, useQuery } from "@tanstack/react-query";
import { formatIstIso } from "../../shared/lib/format-ist";
import { Button } from "../../shared/ui/Button";
import { Modal } from "../../shared/ui/Modal";
import { Money } from "../../shared/ui/Money";
import { ErrorView, LoadingView } from "../../shared/ui/StateViews";
import { Tag } from "../../shared/ui/Tag";
import { approvePaperOrder, previewQuery } from "./queries";

/** Practice order review. The Approve click is the only thing that calls the server. */
export function ReviewOrderDialog({ ideaId, onClose }: { ideaId: string; onClose: () => void }) {
  const preview = useQuery(previewQuery(ideaId));
  const approve = useMutation({ mutationFn: () => approvePaperOrder(ideaId) });
  const p = preview.data;
  return (
    <Modal title="Review paper order" onClose={onClose}>
      <p>
        <Tag tone="warn">PAPER</Tag> Practice order only. No real money is used.
      </p>
      {preview.isPending && <LoadingView label="Loading order details..." />}
      {preview.isError && <ErrorView message="Could not load the order details." onRetry={() => void preview.refetch()} />}
      {p && (
        <dl className="idea-figures">
          <dt>Instrument</dt>
          <dd>
            {p.symbol} ({p.side === "buy" ? "Buy" : "Sell"})
          </dd>
          <dt>Quantity</dt>
          <dd>{p.quantity} shares</dd>
          <dt>Max loss</dt>
          <dd>
            <Money value={p.max_loss} />
          </dd>
          <dt>Total</dt>
          <dd>
            <Money value={p.total} />
          </dd>
          <dt>Estimated charges</dt>
          <dd>
            <Money value={p.charges} />
          </dd>
          <dt>Time</dt>
          <dd>{formatIstIso(p.as_of)}</dd>
        </dl>
      )}
      {approve.data && (
        <p role="status">
          {approve.data.status === "paper_filled" ? "Paper order recorded. " : "Not approved. "}
          {approve.data.message}
        </p>
      )}
      {approve.isError && <ErrorView message="The paper order could not be approved. Nothing was recorded." />}
      <div className="dialog-actions">
        <Button variant="ghost" onClick={onClose}>
          {approve.data ? "Close" : "Not now"}
        </Button>
        {!approve.data && (
          <Button disabled={!p || approve.isPending} onClick={() => approve.mutate()}>
            Approve paper order
          </Button>
        )}
      </div>
    </Modal>
  );
}
