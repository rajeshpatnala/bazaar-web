import { AlarmsBanner } from "./AlarmsBanner";
import { KillSwitchPanel } from "./KillSwitchPanel";

/** Shown above every signed-in screen, outside the route error boundary. */
export function SafetyBar() {
  return (
    <>
      <AlarmsBanner />
      <KillSwitchPanel />
    </>
  );
}
