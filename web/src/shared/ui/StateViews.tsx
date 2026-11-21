import type { ReactNode } from "react";

export function LoadingView({ label = "Loading..." }: { label?: string }) {
  return (
    <p role="status" className="ui-state">
      {label}
    </p>
  );
}

export function EmptyView({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="ui-state">
      <p className="ui-state-title">{title}</p>
      {children}
    </div>
  );
}

export function ErrorView({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return (
    <div role="alert" className="ui-state ui-state-error">
      <p>{message}</p>
      {onRetry && (
        <button type="button" className="ui-btn ui-btn-ghost" onClick={onRetry}>
          Try again
        </button>
      )}
    </div>
  );
}
