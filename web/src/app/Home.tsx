import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { overviewQuery } from "../features/overview/queries";
import { formatIstIso } from "../shared/lib/format-ist";
import { Card } from "../shared/ui/Card";
import { Money } from "../shared/ui/Money";
import { EmptyView, ErrorView, LoadingView } from "../shared/ui/StateViews";
import { Tag } from "../shared/ui/Tag";

/** Home screen: the account overview from the server, with loading, error, empty and data-delayed states. */
export function Home() {
  const query = useQuery(overviewQuery);
  const overview = query.data;
  // PROVISIONAL until BZ-509: the practice label shows only when the server says "paper".
  const isPaper = overview?.execution_mode === "paper";
  return (
    <section>
      <h2>Market overview</h2>
      <p className="lbl">Nothing here is a recommendation or a forecast.</p>
      {query.isPending && <LoadingView label="Loading overview..." />}
      {query.isError && <ErrorView message="Could not load the overview." onRetry={() => void query.refetch()} />}
      {overview && (
        <Card labelledBy="overview-title">
          <h3 id="overview-title">
            {isPaper ? (
              <>
                Practice account <Tag tone="warn">PAPER</Tag>
              </>
            ) : (
              <>Account (Mode: {overview.execution_mode})</>
            )}
          </h3>
          <div>
            <Tag>Market: {overview.regime}</Tag> {overview.data_delayed && <Tag tone="warn">Data delayed</Tag>}
          </div>
          <dl className="idea-figures">
            <dt>Capital</dt>
            <dd>
              <Money value={overview.capital} />
            </dd>
            <dt>Today's profit or loss</dt>
            <dd>
              <Money value={overview.day_pnl} />
            </dd>
            <dt>Open positions</dt>
            <dd>{overview.open_positions}</dd>
            <dt>Ideas to review</dt>
            <dd>{overview.ideas_today}</dd>
            <dt>As of</dt>
            <dd>{formatIstIso(overview.as_of)}</dd>
          </dl>
          {overview.ideas_today === 0 ? (
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
