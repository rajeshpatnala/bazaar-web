# ## 6. API input validation

(From `bz-security` SKILL.md; open when the task touches this area.)


Every route is an API boundary exposed to code you do not fully control (npm dependencies, a future XSS, a hostile local page that passes the checks by luck).

- Request bodies are DTOs with `serde` `deny_unknown_fields`, converted by `TryFrom` into domain newtypes (`Symbol` resolved against the instrument master, `Qty` > 0 and a lot multiple for F&O, `Price` a tick multiple, `Decimal` not `f64`). Reject, never clamp silently.
- Routes accept no file paths, SQL, URLs, shell strings or mode flags (`execution_mode` is only set in `risk.toml`).
- Risk, money and limits are recomputed server-side from the store and `config/risk.toml`; a UI-supplied `risk_rupees` or `quantity` is a request validated by `DecisionGate`. `POST /api/v1/approvals/{id}/approve` re-loads the pending intent, re-runs the gate with fresh prices, checks expiry, kill switch, mode and the intent hash the user saw (`bz-execution`); a replayed or stale id fails.
- Errors are `application/problem+json` with a closed `code` and a safe message: no SQL, paths, tokens, stack traces, driver text. Detail goes to `tracing` after redaction. Paginate store reads.
- Never render broker or news text as HTML. SQL: `sqlx` bind parameters only; table and column names from enums. DuckDB: never interpolate user text into `read_parquet('...')` paths; build them from validated `Symbol` and year values. `ServeDir` serves `web/dist` only (never `data/` or the repo root).
