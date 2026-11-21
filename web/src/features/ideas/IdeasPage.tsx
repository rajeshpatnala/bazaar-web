import { useQuery } from "@tanstack/react-query";
import { Card } from "../../shared/ui/Card";
import { Disclosure } from "../../shared/ui/Disclosure";
import { Money } from "../../shared/ui/Money";
import { EmptyView, ErrorView, LoadingView } from "../../shared/ui/StateViews";
import { formatIstIso } from "../../shared/lib/format-ist";
import { Tag } from "../../shared/ui/Tag";
import { ideasQuery } from "./queries";
import type { Idea } from "./types";

export function IdeaCard({ idea, action }: { idea: Idea; action?: React.ReactNode }) {
  const titleId = `idea-${idea.id}`;
  return (
    <Card labelledBy={titleId}>
      <h3 id={titleId}>
        {idea.symbol} <Tag tone={idea.side === "buy" ? "buy" : "sell"}>{idea.side === "buy" ? "Buy" : "Sell"}</Tag>
      </h3>
      <div>
        {idea.low_evidence && <Tag tone="warn">Low evidence</Tag>} <Tag>Market: {idea.regime}</Tag>{" "}
        {idea.data_delayed && <Tag tone="warn">Data delayed</Tag>}
      </div>
      <dl className="idea-figures">
        <dt>Max loss</dt>
        <dd>
          <Money value={idea.max_loss} />
        </dd>
        <dt>Entry</dt>
        <dd>
          <Money value={idea.entry} />
        </dd>
        <dt>Stop</dt>
        <dd>
          <Money value={idea.stop} />
        </dd>
        <dt>Target</dt>
        <dd>
          <Money value={idea.target} />
        </dd>
        <dt>Quantity</dt>
        <dd>{idea.quantity} shares</dd>
      </dl>
      <Disclosure summary="Why this idea?">
        <ul>
          {idea.why.map((w, n) => (
            <li key={`${idea.id}-${n}`}>{w}</li>
          ))}
        </ul>
      </Disclosure>
      {action}
    </Card>
  );
}

export function IdeasPage() {
  const q = useQuery(ideasQuery);
  return (
    <section>
      <h2>Ideas to consider today</h2>
      <p className="lbl">These are suggestions to review, not forecasts. Any of them can lose money.</p>
      {q.data && <p className="lbl">As of {formatIstIso(q.data.as_of)}</p>}
      {q.isPending && <LoadingView label="Loading ideas..." />}
      {q.isError && <ErrorView message="Could not load ideas." onRetry={() => void q.refetch()} />}
      {q.data && q.data.ideas.length === 0 && (
        <EmptyView title="No high-quality setups today">Nothing met the checks. Waiting is a fine choice.</EmptyView>
      )}
      {q.data?.ideas.map((i) => <IdeaCard key={i.id} idea={i} />)}
    </section>
  );
}
