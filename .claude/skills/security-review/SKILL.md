---
name: security-review
description: Stack-independent security review procedure - secrets handling (agents never read .env), threat-modelling a change, baseline checklist (authn/authz, injection, input limits, data stores, transport, supply chain, logging/privacy), severity scale and the findings format reported to the tech lead. Use when reviewing a branch or design for security.
---

# Security review (procedure)

Stack-specific rules live in the stack's security skill (listed in the agent's skills). Findings go to `.agents/reviews/<feature-id>/<pr-N|branch>/security.md` (the exact path `just review-pack` prints) (template `.agents/templates/security-findings.md`); a short summary goes to the manager; the tech lead triages; developers fix; you verify.

## 1. Secrets (all stacks)

- Secrets live only in `.env` (local) or a secret manager (deployed). Agents **never** read, grep, print, copy or edit `.env*` (except `.env.example`), key files or `secrets/`; hooks block it. Connectivity is checked only with `just db-check`.
- In code: secret wrapper types where the language has them, never logged, never in URLs, errors, metrics, snapshots, fixtures, docs, commits or PR text.

## 2. Procedure

1. Read the plan and the diff (`git diff origin/$(just base)...<branch>`) plus directly related code (use `.agents/codemap.md`).
2. Run the stack's red-flag scan (security skill) on changed files.
3. Threat-model new entry points briefly: who can call it, what it can reach, malicious input, high volume.
4. Walk the baseline checklist (section 3) and the stack checklist.
5. Write findings; return <= 10 lines: `security | <feature-id> | <branch> | BLOCK | PASS`, counts by severity, blocker ids, findings path.
6. On re-check, mark each finding `fixed`, `not fixed`, or `accepted risk (by tech lead)`.

## 3. Baseline checklist

- **AuthN/AuthZ:** every entry point authenticated (except health); authorization per action and per object (no IDOR); deny by default.
- **Injection:** queries parameterised; no shell/eval/template strings built from input; dynamic identifiers from allowlists.
- **Input:** size/length/range/format limits at the edge; safe parsers (no external entities, no unsafe deserialisation); path traversal blocked; SSRF allowlist for outbound URLs from input.
- **Data stores:** least-privilege roles; TLS; PII minimised and masked; destructive migrations flagged.
- **Transport/HTTP:** TLS verified; CORS allowlist; security headers; rate limits and timeouts; errors reveal no internals.
- **Supply chain:** dependency/advisory checks clean; new dependencies justified; pinned base images; non-root containers.
- **Logging/privacy:** no bodies, tokens, keys, passwords or personal data in logs; audit log for security-relevant actions.

## 4. Severity

| Severity | Meaning | Blocks merge |
|---|---|---|
| critical | Exploitable now: auth bypass, injection, secret exposure | Yes |
| high | Likely exploitable or serious data exposure | Yes |
| medium | Defence-in-depth gap | Tech lead decides |
| low / info | Hardening | No |

Each finding: id `SEC-<pr>-<n>` (prefix from `review-pack`), severity (`question` when a person must decide), category, file:line, issue, impact, concrete fix, how to verify.
