import { NavLink, Outlet } from "react-router";
import { SafetyBar } from "../features/safety/SafetyBar";
import { Disclaimer } from "../shared/ui/Disclaimer";
import { ModeBadge } from "./ModeBadge";
import { SessionGate } from "./SessionGate";

/** The disclaimer is a sibling of the outlet, outside every route and boundary. */
export function AppShell() {
  return (
    <div className="shell">
      <header className="shell-header">
        <h1>Bazaar</h1>
        <nav aria-label="Main" className="shell-nav">
          <NavLink to="/" end>
            Home
          </NavLink>
          <NavLink to="/ideas">Ideas</NavLink>
        </nav>
        <ModeBadge />
      </header>
      <main className="shell-main">
        <SessionGate>
          <SafetyBar />
          <Outlet />
        </SessionGate>
      </main>
      <Disclaimer />
    </div>
  );
}
