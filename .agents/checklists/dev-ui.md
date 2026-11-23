# Top 10: dev-ui (from 25 findings, 2026-11-23)
1. The PAPER/mode label and "practice" wording come from server state, never hardcoded text [RISK-126-1, CR-127-1].
2. Approve carries the preview id/hash it was shown; a refetch on focus cannot swap the figures under an open dialog [RISK-126-2, CR-126-3].
3. Retry keeps one idempotency key per draft and a new key for a new draft; a timeout says "outcome unknown", not "failed" [RISK-126-3, RISK-126-9].
4. Calls go through `api/client.ts` (401, CSRF, problem+json), not a raw `fetch` [CR-126-4, RISK-126-6].
5. Responses are checked at runtime for every field and list item used: enums, non-negative ints, strings [RISK-126-7, CR-124-4, CR-127-2].
6. Every fetch has a timeout/abort; polling stops after a 401 [CR-124-1, SEC-25-1].
7. Timestamp formatting never throws in render and requires an offset; tests force `TZ` [CR-101-1, CR-101-4, CR-101-2].
8. The fixed disclaimer keeps room (no fixed padding), dialogs do not cover it, colours come from tokens only [CR-101-6, RISK-126-5, CR-117-1].
9. The approval dialog shows every figure being approved (qty, side, price, max loss, total, charges, time) [RISK-126-4].
10. Test state (CSRF token, TZ) is reset in `finally`; a contrast claim has a recorded ratio [CR-67-1, CR-101-5].
