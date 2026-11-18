---
name: fe-audit
description: Front-end audit procedure - confirm the target, threat-model, coverage manifest, automated checks, the security, commerce, architecture, quality, accessibility, performance, testing and playbook-gap lenses, verification with confidence levels, severity, completion gate and the audit report. Use when auditing a front-end repo or part of one, rather than reviewing a single PR.
---

# Front-end audit

Run a full or scoped audit of a front-end repo and write `docs/audits/YYYY-MM-DD/review-log.md`. PR reviews follow `code-review` instead; fixes are made by developers after the tech lead triages the findings.

**Reads.** An audit judges code, so read every in-scope file in full; the kit's "read narrowly" rule is for everyday tasks. Say once: "This audit needs full reads, so I'm reading in-scope files completely." Never read `.env*` (except `.env.example`), key files or `secrets/`: list them and flag any that git tracks.

**Evidence over opinion.** Every finding cites `file:line` and a concrete failure or exploit scenario. Never invent paths, line numbers, command results or tool availability. Reading code is not testing: say what was and was not tested.

**Safe execution.** Read what a script does before running it. Never run deploy, publish, migrate, seed or reset scripts, anything that sends messages, creates payments or touches customer data, anything with live keys or production endpoints, destructive git commands, or dependency changes.

### 1. Confirm the target
- Run `pwd` and `git rev-parse --show-toplevel` (skip if not a git repo). A user-named path is the target; otherwise the repo root.
- Confirm it is a front-end repo (HTML/JS/CSS or `package.json`).
- Ask **only** if the target is genuinely ambiguous (a folder of several repos, a similarly named sibling, an empty folder, a path mismatch). This is the only point where an audit stops to ask.
- State `Auditing: /abs/path`.

### 2. Orient and threat-model
- Identify stack, hosting, build/deploy config (`netlify.toml`, `vercel.json`, `_headers`, `.htaccess`, `nginx.conf`, CI), where data lives, how auth and checkout work, third-party scripts, demo vs production code.
- Write a 3-line threat model: likely attackers, what is worth stealing or breaking, entry points. Use it to rank severity.
- List the critical user flows (browse, cart, checkout, payment failure, confirmation, login, admin…). The report tracks each.

### 3. Coverage
Inventory with `git ls-files` plus `git ls-files -o --exclude-standard` (or `find . -type f`), excluding `.git`, `node_modules`, build output and `docs/audits/`.

| Depth | Applies to | How |
|---|---|---|
| Full read | Hand-written source, markup, styles, config, CI, scripts, `.env.example`, headers files, service workers, manifests, SVG, docs | First line to last. |
| Never read | `.env*` (except `.env.example`), key files, `secrets/` | List them; flag any that git tracks. Never open or print contents. |
| Skim | Minified, generated or vendored files | Head and tail; grep inline `<script>`, URLs and keys. Flag committed generated code as debt. |
| List | Binaries | `ls -la` and `file`. Flag oversized assets and disguised files. |
| Exclude | `node_modules`, lockfile bodies | Only check the lockfile exists and matches `package.json`. |

- **Manifest:** above ~40 full-read files, write the inventory to `docs/audits/YYYY-MM-DD/coverage.md` (path, depth, status, notes) before reading, and update it after each group with statuses **and** findings. After compaction or interruption, resume from it.
- **Delegation (agent kit):** you can't launch agents yourself; only the manager can. Above ~40 files or ~8,000 lines, write the group plan into the manifest (groups of 15–20 files, with the threat model and lenses) and return it to the manager, asking for one `fe-auditor` per group in parallel. Each group run reads its files, applies the lenses and appends per-file status and findings (`file:line`, scenario, severity guess) to the manifest. A final `fe-auditor` run then verifies (step 6) and reports (step 7): before accepting a group, re-open at least two of its cited findings; if one fails, re-read that group.
- **Gate:** don't report while any full-read file is unread. Statuses: `read`, `skimmed`, `listed`, `excluded`, `secret (not read)`, `unreadable (reason)`. Print `N files = read + skimmed + listed + excluded + secret + unreadable` and make it add up.

### 4. Automated checks (when available and safe)
- **Dependencies:** `npm audit` / `pnpm audit` / `yarn npm audit`. Per hit: installed version, path, browser-shipped or dev-only, compatible fix or not. Dev-only ranks at most Low unless it affects the build pipeline.
- **Secrets:** `gitleaks` or `trufflehog` if installed (on tracked files; redact output); otherwise grep tracked source for the patterns (`sk_live_`, `rk_live_`, `rzp_live_`, `shpat_`, `AKIA`, `-----BEGIN`, `password=`, `secret`, `token`, `api_key`) with `git grep -l` so only file names are printed. Never grep inside `.env*` files.
- **Lint, typecheck, tests, build:** the repo's own scripts, if safe and offline.
- **Performance baseline:** if the build runs offline, record total bundle size, largest JS/CSS chunks and heaviest images. Core Web Vitals need a live browser — "Not checked" unless measured.
- Tool output is a lead, not a finding: confirm every hit in code. **Record every command** with exit status and a one-line result; a failed or timed-out check is "not done", never "passed".

### 5. Lenses
Run every lens for a full audit. Load the reference when the lens applies; if a reference is missing, use the checklist here.

| Lens | Reference | Minimum checklist |
|---|---|---|
| Security | `.claude/skills/fe-security/references/security.md`, `.claude/skills/fe-auth-realtime/references/testing-and-security.md` (checklist) | DOM sinks (`innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval`, `new Function`, string `setTimeout`, `dangerouslySetInnerHTML`, `v-html`, unescaped template output); URL sinks (`javascript:` hrefs, open redirects, `location` from params); `postMessage` without origin checks; secrets in bundles incl. public env prefixes (`NEXT_PUBLIC_`, `VITE_`, `PUBLIC_`); tokens in web storage; CSRF on cookie-authenticated writes; CSP, HSTS, `X-Content-Type-Options`, `frame-ancestors`, `Referrer-Policy`, `Permissions-Policy`; third-party scripts without SRI or pinning; client-only validation. |
| Commerce | `.claude/skills/fe-security/references/commerce.md` | Prices, discounts or totals computed or trusted on the client; payment success trusted from a redirect without server-side signature/webhook verification; client-set amount; no idempotency or duplicate-submit protection; live keys in repo or test keys in prod. |
| Architecture | `.claude/skills/fe-audit/references/architecture.md`, `.claude/skills/fe-app-architecture/SKILL.md` | Duplicated or hardcoded values; data mixed into layout; state in the wrong home (server data copied into global stores, filters not in the URL); unvalidated API responses; copy-pasted components; missing tokens; non-reproducible builds. |
| Quality | `.claude/skills/fe-audit/references/quality-modernization.md` | Error, empty and loading states; broken links; SEO basics; browser support; legacy patterns worth modernizing. |
| Accessibility | `.claude/skills/fe-a11y-i18n/references/checklist.md` | Labels, names and roles; keyboard and focus on core flows; contrast; reduced motion; hard-coded strings if the product is multilingual. |
| Performance | `.claude/skills/fe-performance/references/third-party-and-enforcement.md` (PR checklist) | Image weight and dimensions, render-blocking resources, bundle size vs 170 KB JS / 30 KB CSS budgets, no lazy loading for heavy widgets, long tasks on input. |
| Testing & delivery | `.claude/skills/fe-testing/references/contracts-and-policy.md`, `.claude/skills/fe-cicd-observability/SKILL.md` | Critical flows without tests; tests that mock internals; flaky waits; no CI gate; no error monitoring; secrets in CI. |
| Playbook gap | every `.claude/skills/fe-*/SKILL.md` **Done when** | Only for React/TypeScript repos, or when the user asks to modernize. For each area: `meets` / `partial` / `missing` / `n/a`, with the evidence (`file:line`) and the single highest-value step to close the gap. |

**Headers: source vs deployed.** A header missing from repo config doesn't prove the live site lacks it. Say which layer you checked. If the user gives the URL of their own site, one `curl -sI` is allowed. Without it, header findings are at most `Suspected` with "deployed headers unverified".

### 6. Verify
Re-open every cited `file:line`, trace source to sink, confirm the failure is real; drop what doesn't hold. Never report a sub-agent's claim unchecked.
- **Confidence** (separate from severity): `Confirmed` (reproduced by a rerunnable command or test — log command and redacted output), `High` (complete code path, not reproduced), `Suspected` (depends on unverified environment or backend), `Info` (hardening, no demonstrated vulnerability).
- **Verification level:** `static`, `automated`, `browser` or `environment`. One level never implies another. If blocked, say so.
- **Backend-dependent:** if exploitability depends on server behaviour you can't see, tag it, state exactly what the server must do, and set Confidence to Suspected.

### 7. Report and log
- First line: **Status** `complete` / `partial` / `blocked` (completion gate).
- **Verdict** (2–3 lines): overall health, biggest risk, biggest opportunity.
- **Findings table**, by severity then confidence: `ID | Status | Severity | Confidence | Verified (level) | Category | file:line | What & scenario | Prerequisites | Fix | Effort (S/M/L)`.
- **Critical flows:** each with `tested (how)` or `not tested`, plus finding IDs.
- **Playbook gap table** (when that lens ran).
- **Quick wins** (today) vs **structural work** (plan it), each structural item naming the playbook that describes the target.
- **Commands run** with exit statuses. **Not checked**, explicitly.
- **Compare with the last review** in `docs/audits/`: mark findings `new`, `still open`, `regressed`; list `fixed` ones.
- Write to `docs/audits/YYYY-MM-DD/review-log.md` (date from `date +%F`; if it exists, `review-log-HHMM.md`; never overwrite) with header (repo, date/time, scope, lenses, tools) and the final coverage table. **Redact** secrets, tokens and personal data — cite the location instead. `docs/audits/` is inside the repo; the user decides whether to commit it.
- Chat reply: status, verdict, top 3–5 findings, log path.

## 8. Severity guide
Severity = business impact × ease of exploit. Note prerequisites (anonymous visitor, logged-in customer, admin, victim must click, network position).
- **Critical:** low-effort exploit leading to script execution in a victim's browser, secret or credential exposure, payment or price tampering, auth bypass, customer data exposure.
- **High:** stored or reflected XSS sinks, missing CSRF on state changes, client-trusted prices or discounts, unpinned third-party script with page access; or Critical impact needing privileged access or unlikely conditions.
- **Medium:** missing CSP or security headers, tokens in web storage, no SRI, open redirects, verbose errors, a11y blockers on core flows, budgets blown on core pages.
- **Low:** hygiene, hardcoding, dead code, inconsistency, dev-only dependency issues, playbook gaps without user impact.
Adjust one level with a stated reason. Anything touching money, checkout or customer accounts ranks at least one level higher than the same flaw on a static page.

## 9. Completion gate
Report `complete` only when:
1. Scope and coverage are fully accounted for (the count adds up).
2. Every finding is re-verified with `file:line`, confidence and verification level.
3. Every critical flow is marked tested or not tested.
4. Changes are baselined, diff-reviewed and re-verified.
5. Logs contain no secrets or personal data.
6. Limits and unverified assumptions are under "Not checked".

Otherwise report `partial` or `blocked`, name the unmet item and what remains. "Complete" never certifies the app as secure, bug-free or production-ready.

## 10. Hard rules
- Never print or log real secrets or personal data; report the location and advise rotation.
- Only test the user's own repo or environment. Never scan third-party hosts; the only network check allowed is a header request to the user's own site when they give the URL.
- Never delete, overwrite or restructure without asking; back up first if the repo isn't versioned.
- Never delete, rename, force-push or reset `develop` or `main`, and never merge into `main` without the user's approval.
- Separate demo placeholders (`DUMMY`, fake payment) from production holes, and flag demo code that must not ship.
