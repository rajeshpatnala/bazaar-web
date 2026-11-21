import type { ReactNode } from "react";

/** White rounded panel. Pass `labelledBy` to expose it as a named region. */
export function Card({ children, labelledBy }: { children: ReactNode; labelledBy?: string }) {
  return (
    <section className="ui-card" aria-labelledby={labelledBy}>
      {children}
    </section>
  );
}
