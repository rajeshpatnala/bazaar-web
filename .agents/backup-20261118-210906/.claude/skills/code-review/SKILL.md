---
name: code-review
description: Code review procedure for this repo - scope, red-flag scan, generic and stack-specific checklists, severity labels, and the findings file format sent to the tech lead. Use when reviewing a branch, PR, or diff.
---

# Code Review

Reviewers **do not edit source code**. Findings go to `.agents/reviews/<feature-id>/code-review.md`
(template `.agents/templates/review-findings.md`); a short summary goes to the manager, who passes it
to the tech lead for triage. Security issues you notice are recorded too, tagged `security`, and the
security reviewer owns the deep check.

Reviews run **per branch as soon as its developer hands off** (or on a draft PR at a checkpoint), in parallel with other developers still building. Review only what's on that branch.

## 1. Procedure

1. Read the feature plan (`.agents/plans/<feature-id>.md`) - acceptance criteria and slice scope.
2. Get the diff only: `git fetch -q && git diff --stat origin/$(just base)...<branch>` then `git diff origin/$(just base)...<branch> -- <files>`. Don't read unrelated files; use `.agents/codemap.md` to locate context.
3. Confirm the gate result from the developer's handoff; if missing, ask the manager for a build-runner check instead of running long builds yourself.
4. Read tests first, then code from the core outwards (domain/business logic first, integrations and entry points last).
5. Run the red-flag scan on changed files.
6. Walk the checklist for the areas touched.
7. Write findings; return a <= 10-line summary.

## 2. Red-flag scan

```bash
rg -n 'unwrap\(|expect\(|todo!|unimplemented!|println!|dbg!|#\[allow|unsafe|\bf(32|64)\b.*(price|pnl|amount|charge|capital|risk|qty|inr|rupee)|(price|pnl|amount|charge|capital|risk|inr|rupee)\w*\s*:\s*f(32|64)|lot_size\s*[:=]\s*\d|tick_size\s*[:=]\s*\d|Weekday::(Tue|Thu)|skip_gate|bypass|auto_approve|default\s*=\s*\[[^\]]*live-trading|access_token|api_secret|request_token|innerHTML|dangerouslySetInnerHTML|: any\b|as any|@ts-ignore|console\.log|SELECT \*|format!\(.*(SELECT|INSERT|UPDATE|DELETE)|std::thread::sleep|unbounded_channel|Utc::now\(\)|Local::now\(\)|reqwest::get\(|CorsLayer::permissive|allow_origin\(Any\)|0\.0\.0\.0|SameSite=None|danger_accept_invalid_certs|SameSite::None' <changed files>
```

Each hit must be justified or becomes a finding.

## 3. Checklist by area

- **Correctness:** meets acceptance criteria; edge cases (empty, max, negative, duplicate, null vs missing); time zones; external input.
- **Errors:** handled at the right layer; no swallowed errors; no leaked internals to callers; logged once.
- **Security basics:** no secrets in code/logs; parameterised queries; input validated at the edge (deep check: security reviewer).
- **Tests:** unit tests present for every new behaviour and error path (missing = blocker); deterministic; mocks only.
- **Records:** a feature PR adds or updates `docs/features/F<n>-*.md` with what it built; a `fix/B<n>` PR has a regression test that fails without the fix and a `docs/bugs/B<n>-*.md` with root cause (missing or wrong = major). Never accept secrets or real customer data in a record.
- **Architecture:** layering/dependency rules from `bz-architecture` respected; code in the right module; ADR for irreversible choices.
- **Readability:** names, small functions, docs on public items, comments explain why.

### Stack-specific checklist

- **Money and numbers:** `Decimal` for every ₹ value, quantity and charge; `f64` only inside indicator/pricing math with explicit conversion and tick rounding; no silent truncation.
- **Time:** IST via `Clock`; no `Utc::now()`/`Local::now()` in domain code; session and expiry logic from the calendar and instrument master, never hardcoded weekdays or dates.
- **Market facts and rates:** lot/tick sizes, expiries, ban list, holidays, circuits and charges loaded, not literals; config changes carry `source` + `verified_on`.
- **Look-ahead (`bz-backtest-risk`):** decisions see data up to the bar close only; fills at next bar/tick; look-ahead test present for new strategy or backtest logic.
- **Costs:** every P&L path applies the full `CostModel` and slippage.
- **Gate and execution (`bz-execution`, `bz-trading-discipline`):** no path to an `Executor` without the DecisionGate; gate re-run at approval; kill switch/circuit breaker checked before send; `live-trading` not default; audit log append-only.
- **Recommendations (`bz-quant`):** all hard-rule-6 fields; "low evidence" below ~30 trades; Why text in plain English; disclaimer visible.
- **Data (`bz-market-data`):** official sources only; reconnect with backoff and resubscribe; stale-data flag; rate limits respected; tokens never logged.
- **Async:** no blocking in async; no lock across `.await`; timeouts on outbound calls; bounded channels.
- **API (`bz-web-api`):** thin handlers, DTOs separate from domain, problem+json errors, Host/Origin guards, session required (except `/healthz`, `/auth`, Kite callback), CSRF on non-GET, no order-placing route, input validation and body limits, WebSocket throttled, OpenAPI updated and UI types regenerated.
- **UI (`bz-web-ui`):** generated API types only, no `innerHTML`, strict CSP respected, MSW in tests, disclaimer and every recommendation field visible.
- **Tests:** reference values for formulas; fixtures not live APIs; property tests for backtester invariants.

## 4. Severity

| Label | Meaning | Blocks merge |
|---|---|---|
| blocker | Bug, data loss, contract break, architecture violation, security issue | Yes |
| major | Missing important tests, poor error handling, likely prod performance issue | Yes unless tech lead defers |
| minor | Readability, idiom | No |
| nit | Preference | No |
| question | Need clarification | Until answered |

Each finding: id (`CR-<feature>-<n>`), severity, file:line, issue, why it matters, concrete fix (code if short).

## 5. Summary returned to the manager

```
Review <feature-id> <branch>: REQUEST CHANGES | APPROVE | APPROVE WITH MINORS
blockers: CR-12-1, CR-12-3 | major: CR-12-2 | minor: 4 | nit: 2
findings: .agents/reviews/<feature-id>/code-review.md
```

On re-review, update each finding's status (`fixed` / `not fixed` / `deferred by tech lead`) instead of writing a new file.
