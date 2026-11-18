import { Outlet } from "react-router";
import { Disclaimer } from "../shared/ui/Disclaimer";

/** The disclaimer is a sibling of the outlet, outside every route and boundary. */
export function AppShell() {
  return (
    <div className="shell">
      <header className="shell-header">
        <h1>Bazaar</h1>
      </header>
      <main className="shell-main">
        <Outlet />
      </main>
      <Disclaimer />
    </div>
  );
}
