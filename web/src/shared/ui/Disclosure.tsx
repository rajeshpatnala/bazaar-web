import { useId, useState, type ReactNode } from "react";

/** Plain-English "Why?" panel: a button controlling a region (aria-expanded). */
export function Disclosure({ summary, children }: { summary: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <div className="ui-disclosure">
      <button type="button" className="ui-disclosure-btn" aria-expanded={open} aria-controls={id} onClick={() => setOpen((o) => !o)}>
        {summary}
      </button>
      <div id={id} hidden={!open} className="ui-disclosure-body">
        {children}
      </div>
    </div>
  );
}
