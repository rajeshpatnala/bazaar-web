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

export const getCsrfToken = (): string | null => csrf;

let sessionEndedListener: (() => void) | null = null;
export const onSessionEnded = (fn: (() => void) | null) => {
  sessionEndedListener = fn;
};

function attachCsrf(request: Request): Request {
  if (!["GET", "HEAD", "OPTIONS"].includes(request.method)) {
    if (!csrf) throw new Error("missing CSRF token");
    request.headers.set(CSRF_HEADER, csrf);
  }
  return request;
}
function noteResponse(response: Response): Response {
  if (response.status === 401) sessionEndedListener?.();
  return response;
}

const security: Middleware = {
  onRequest: ({ request }) => attachCsrf(request),
  onResponse: ({ response }) => noteResponse(response),
};

/**
 * JSON call for routes not yet in the generated schema (PROVISIONAL, BZ-509 swaps these for `api`).
 * Same CSRF header and 401 handling as `api`; 10 s timeout; problem+json becomes an `ApiError`.
 * Network failures and timeouts become `ApiError` with status 0 (outcome unknown).
 */
export async function apiJson<T>(
  path: string,
  opts: { method?: string; body?: unknown; headers?: Record<string, string> } = {},
): Promise<T> {
  const headers = new Headers(opts.headers);
  if (opts.body !== undefined) headers.set("Content-Type", "application/json");
  let request = new Request(new URL(path, location.origin), {
    method: opts.method ?? "GET",
    headers,
    body: opts.body === undefined ? null : JSON.stringify(opts.body),
    credentials: "same-origin",
    signal: AbortSignal.timeout(10_000),
  });
  try {
    request = attachCsrf(request);
  } catch {
    throw new ApiError(0, "csrf", "Missing security token. Reload the page.");
  }
  let response: Response;
  try {
    response = noteResponse(await globalThis.fetch(request));
  } catch {
    throw new ApiError(0, "network", "No answer from the server.");
  }
  if (!response.ok) {
    const p = (await response.json().catch(() => null)) as Partial<Problem> | null;
    throw new ApiError(response.status, p?.code ?? "unknown", p?.detail ?? `Request failed (${response.status})`);
  }
  try {
    return (await response.json()) as T;
  } catch {
    throw new ApiError(0, "bad_body", "Unreadable answer from the server.");
  }
}

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
