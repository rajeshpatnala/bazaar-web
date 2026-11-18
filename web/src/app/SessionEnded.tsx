/** Shown on a 401. No form and no retry: the user restarts Bazaar to get a fresh launch link. */
export function SessionEnded() {
  return (
    <section role="alert" className="session-ended">
      <h2>Session ended</h2>
      <p>
        Your Bazaar session is no longer valid. Close this tab and open the launch link printed by
        the app again.
      </p>
    </section>
  );
}
