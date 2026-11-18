import type { RequestHandler } from "msw";

/** Shared by the browser worker and the Vitest server. Typed handlers arrive with the API client slice. */
export const handlers: RequestHandler[] = [];
