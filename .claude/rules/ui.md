---
paths:
  - "web/**"
---
# Web UI

- Load `bz-web-ui`. API types come only from the generated `web/src/api/schema.d.ts` (`just openapi`); never hand-write DTOs.
- Components are tested with MSW (`web/src/mocks/handlers.ts`), never the real server or network.
- Every recommendation shows all required fields, the low-evidence badge and a plain-English Why panel; the disclaimer is always visible; ₹ in en-IN grouping, times in IST.
- No `innerHTML`/`dangerouslySetInnerHTML`; no tokens or secrets in `localStorage`; send the CSRF header on non-GET requests.
