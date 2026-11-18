import { Outlet } from "react-router";
import { Disclaimer } from "../shared/ui/Disclaimer";
import { ModeBadge } from "./ModeBadge";
import { SessionGate } from "./SessionGate";

/** The disclaimer is a sibling of the outlet, outside every route and boundary. */
export function AppShell() {
  return (
    <div className="shell">
      <header className="shell-header">
        <h1>Bazaar</h1>
        <ModeBadge />
      </header>
      <main className="shell-main">
        <SessionGate>
          <Outlet />
        </SessionGate>
      </main>
      <Disclaimer />
    </div>
  );
}
