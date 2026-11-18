# Project profile

The manager fills this in with the user at the start (keep it under 60 lines). Every agent reads it.

## Purpose
<!-- one paragraph: what the service does and for whom -->

## Consumers / API style
<!-- e.g. internal web app, partners; REST/JSON; auth method (OAuth/JWT issuer) -->

## Data stores
<!-- list only what this project uses; remove the rest -->
| store | role | access | notes |
|---|---|---|---|
| SQLite (`data/bazaar.db`) | app state, approvals, kill switch, job runs, journal, paper account, audit log (sessions are in memory only) | sqlx 0.9, migrations in `crates/store/migrations` | |
| Parquet + DuckDB (`data/candles/`) | candles, bhavcopy history, backtest data | duckdb crate, partitioned by symbol/year | |
| Zerodha Kite Connect | live ticks (WebSocket), historical candles, instruments, margins; orders later | REST + WebSocket, daily login redirect to `/api/v1/kite/callback` | |
| NSE/BSE bhavcopy archives | end-of-day universe, delivery % | official downloads only | |

## Integrations
<!-- external APIs, queues, object storage -->

## Input data / parsing
<!-- formats (JSON/CSV/NDJSON/Parquet/XML), sources, volumes, reject policy -->

## Non-functional targets
<!-- latency p95, throughput, availability, data freshness, cost limits -->

## Deployment and CI
<!-- e.g. Kubernetes/ECS, GitHub Actions, environments -->

## Constraints and conventions
<!-- org rules, compliance, naming, anything agents must respect -->
