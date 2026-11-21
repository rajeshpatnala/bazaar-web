# Compliance, Disclaimer and Wording

Open this file for any user-facing copy (recommendations, Why? panels, tooltips, README, marketing text). This is the **only** place for the disclaimer text and the no-advice wording rules; other skills link here and never retype them. The SEBI retail algo framework and the pre-live checklist are **not** here: see `bz-security` `references/sebi-algo-compliance.md` (open it before any live-order work). This is engineering guidance, not legal advice.

## 1. Permanent disclaimer (exact text)

> For education and personal analysis. Not investment advice; not SEBI registered; markets carry risk. Past performance does not guarantee future results.

Rules:
- One shared constant in the UI (`shared/disclaimer.ts`) and one in `crates/core` for exports and notifications. Do not retype it; do not paraphrase it.
- Rendered by the app shell on every screen, always visible (not behind a menu, not dismissible, not collapsed on small screens). Included in every CSV/PDF export, every browser-notification/Telegram alert that carries a trade idea, and the README top section.
- A component test asserts the shell renders it on each route (`bz-web-ui`, `bz-testing`). Removing or hiding it is a blocking review finding.
- The app and its docs must not say or imply the author or the app is SEBI-registered, regulated, or an adviser.

## 2. Wording rules

| Do | Do not |
|---|---|
| "Setup", "idea for analysis", "the system flags", "historically this pattern won 41% of 63 trades" | "Buy now", "you should", "recommended for you", "tip", "sure-shot", "guaranteed", "risk-free" |
| Show risk in rupees first, then reward:risk | Lead with potential profit |
| "Low evidence (12 trades)" under about 30 trades | Hide the sample size |
| "No high-quality setups today" as a normal outcome | Force a pick to fill the screen |
| Probabilities as ranges with the method named ("backtest, costs included, out of sample") | Single-point certainty ("90% accurate") |
| "Paper trade" and "Live (real money)" labelled distinctly | Same styling for both |
| Past results labelled as past | Annualising a short window as a promise |

Naked option selling and ratio spreads: warning with margin and worst-case loss figures in rupees, never a one-click path.

## 3. SEBI adviser rules in plain terms

Primary sources: the SEBI (Investment Advisers) Regulations, 2013 and the SEBI (Research Analysts) Regulations, 2014, both under https://www.sebi.gov.in/legal/regulations.html; recent SEBI circulars on finfluencers and unregistered advice are at https://www.sebi.gov.in/legal/circulars.html (circular numbers: unverified here, confirm there).

- Giving personalised buy/sell advice for a fee or other consideration requires registration as an Investment Adviser or Research Analyst. Registered advisers also have qualification, disclosure, risk-profiling and record-keeping duties.
- A personal analysis tool used by its owner on their own money is not advice to anyone. The moment the app is shared, sold, or its signals are published, forwarded, or sold as a channel, the adviser question arises. Ask the user before adding sharing, social, subscription, or multi-user features and record the decision in `DECISIONS.md`.
- Unregistered persons are restricted in presenting performance claims or tips to the public as advice (check the current SEBI rules on promotion). Do not add "share your trade" features without a compliance review.
- Registered status, if ever claimed, is verified at SEBI's intermediary search (https://www.sebi.gov.in); do not print a registration number you have not verified.

## 4. Tests for wording and compliance

- Vitest: shell renders the exact disclaimer constant on every route; a deny-list regex test over Why-panel and alert templates ("buy now", "guaranteed", "sure-shot", "risk-free", "tip").
- insta snapshots for Why? text and low-evidence labels (`bz-testing`).
- A Rust test that the export and notification builders append the disclaimer.
- Feature-flag test: `live-trading` is not in default features.
