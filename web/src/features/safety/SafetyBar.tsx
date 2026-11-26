import { AlarmsBanner } from "./AlarmsBanner";
import { KillSwitchPanel } from "./KillSwitchPanel";

/** Shown above every signed-in screen, outside the route error boundary. Alarms never push Engage down. */
export function SafetyBar() {
  return (
    <>
      <KillSwitchPanel />
      <AlarmsBanner />
    </>
  );
}
