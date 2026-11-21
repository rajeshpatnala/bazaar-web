import { useEffect, useId, useRef, type ReactNode } from "react";

const FOCUSABLE = 'a[href],button:not([disabled]),input:not([disabled]),select,textarea,[tabindex]:not([tabindex="-1"])';

/** Accessible dialog: labelled, Esc closes, Tab is trapped, focus returns to the opener. */
export function Modal({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  const titleId = useId();
  const ref = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  useEffect(() => {
    closeRef.current = onClose;
  });

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    const node = ref.current;
    (node?.querySelector<HTMLElement>(FOCUSABLE) ?? node)?.focus();
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        closeRef.current();
        return;
      }
      if (e.key !== "Tab" || !node) return;
      const items = [...node.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (items.length === 0) {
        e.preventDefault();
        return;
      }
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const active = document.activeElement;
      if (e.shiftKey && (active === first || !node.contains(active))) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (active === last || !node.contains(active))) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      opener?.focus?.();
    };
  }, []);

  return (
    <div className="ui-backdrop">
      <div ref={ref} role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1} className="ui-modal">
        <h2 id={titleId}>{title}</h2>
        {children}
      </div>
    </div>
  );
}
