import createClient, { type Middleware } from "openapi-fetch";
import type { components, paths } from "./schema";

export type Problem = components["schemas"]["Problem"];

export const CSRF_HEADER = "X-CSRF-Token";

/** Error thrown by query functions; carries the HTTP status and the problem+json `code`. */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

let csrf: string | null = null; // memory only, never storage
export const setCsrfToken = (t: string | null) => {
  csrf = t;
};

let sessionEndedListener: (() => void) | null = null;
export const onSessionEnded = (fn: (() => void) | null) => {
  sessionEndedListener = fn;
};

const security: Middleware = {
  onRequest({ request }) {
    if (!["GET", "HEAD", "OPTIONS"].includes(request.method)) {
      if (!csrf) throw new Error("missing CSRF token");
      request.headers.set(CSRF_HEADER, csrf);
    }
    return request;
  },
  onResponse({ response }) {
    if (response.status === 401) sessionEndedListener?.();
    return response;
  },
};

export const api = createClient<paths>({
  baseUrl: location.origin,
  credentials: "same-origin",
  // Resolve fetch per call so test/dev mocks installed after import are honoured.
  fetch: (request) => globalThis.fetch(request),
});
api.use(security);

/** Turn an openapi-fetch `{ data, error, response }` into data or a thrown `ApiError`. */
export function unwrap<T>(r: { data?: T; error?: unknown; response: Response }): T {
  if (r.data !== undefined) return r.data;
  const p = r.error as Partial<Problem> | undefined;
  throw new ApiError(
    r.response.status,
    p?.code ?? "unknown",
    p?.detail ?? `Request failed (${r.response.status})`,
  );
}
