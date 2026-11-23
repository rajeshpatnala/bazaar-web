---
name: project-trade-lifecycle-expectation
description: Owner expects Bazaar to manage open positions like a trader (daily monitoring, hold/sell/add decisions), not just emit one entry/target/stop suggestion
metadata:
  type: project
---

Owner said (2026-11-23): once the web app is complete he does NOT want "buy HDFC at 1000, sell at 1120, stop 950" and then silence. The app must track each open position daily, watch the stock's behaviour (and events/results/announcements), and keep making hold / sell / reduce / add decisions like a human trader who watches the stock.

**Why:** the owner wants a trader-style assistant, not a one-shot idea generator. Current build only has entry ideas (F11, fixtures), gate, approval and reduce-only exits; no position monitor, no daily health check, no event warnings.

**How to apply:** treat as a new L feature (position monitor + daily review + event-aware exit/trim/add suggestions). Plan with market-strategist first. Keep hard rules: every order still needs owner approval via DecisionGate, paper default, legal data only (no news scraping; events from events.toml / official sources), never imply certainty. Do not describe the app as protecting a position until this exists. Not started yet; owner had also said to stop after BZ-205 S5.
