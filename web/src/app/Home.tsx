import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { overviewQuery } from "../features/overview/queries";
import { formatIstIso } from "../shared/lib/format-ist";
import { Card } from "../shared/ui/Card";
import { Money } from "../shared/ui/Money";
import { EmptyView, ErrorView, LoadingView } from "../shared/ui/StateViews";
import { Tag } from "../shared/ui/Tag";

export function Home() {
  const q = useQuery(overviewQuery);
  const o = q.data;
  return (
    <section>
      <h2>Market overview</h2>
      <p className="lbl">A practice account. Nothing here is a recommendation or a forecast.</p>
      {q.isPending && <LoadingView label="Loading overview..." />}
      {q.isError && <ErrorView message="Could not load the overview." onRetry={() => void q.refetch()} />}
      {o && (
        <Card labelledBy="overview-title">
          <h3 id="overview-title">
            Practice account <Tag tone="warn">PAPER</Tag>
          </h3>
          <div>
            <Tag>Market: {o.regime}</Tag> {o.data_delayed && <Tag tone="warn">Data delayed</Tag>}
          </div>
          <dl className="idea-figures">
            <dt>Capital</dt>
            <dd>
              <Money value={o.capital} />
            </dd>
            <dt>Today's profit or loss</dt>
            <dd>
              <Money value={o.day_pnl} />
            </dd>
            <dt>Open positions</dt>
            <dd>{o.open_positions}</dd>
            <dt>Ideas to review</dt>
            <dd>{o.ideas_today}</dd>
            <dt>As of</dt>
            <dd>{formatIstIso(o.as_of)}</dd>
          </dl>
          {o.ideas_today === 0 ? (
            <EmptyView title="No high-quality setups today">Waiting is a fine choice.</EmptyView>
          ) : (
            <p>
              <Link to="/ideas">See ideas to consider and review a paper order</Link>
            </p>
          )}
        </Card>
      )}
    </section>
  );
}
