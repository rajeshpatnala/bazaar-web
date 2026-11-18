# 10. NFR template (fill for every feature)

(From `bz-architecture` SKILL.md; open when the task touches this area.)


| NFR | Starting target | How verified |
|---|---|---|
| Tick to screen latency | under 250 ms p95 from tick receipt to WebSocket send on the watchlist at the push cadence | replay fixture benchmark |
| API latency | read endpoints under 100 ms p95 and writes (intent, approve, kill switch) under 300 ms p95, excluding broker round trips; heavy reads paginated | in-process `oneshot` timing tests |
| Concurrent tabs | one owner, a few tabs with no pipeline slowdown; slow tabs isolated; double-approve safe | multi-client WebSocket test; approval race test |
| Bar close to recommendation refresh | under 2 s for the F&O universe | criterion bench on fixture universe |
| EOD scan, full NSE equity universe | under 60 s on the owner's Mac | bench, scan progress |
| Backtest throughput | 1 strategy, 1 instrument, 10 years of daily bars in under 1 s | criterion bench; golden regression |
| Cold start to ready | under 5 s without network, including reconcile and catch-up checks | manual + smoke test |
| Memory | under 1 GB resident with a full-universe EOD scan (it shares the Mac with a browser) | profile run |
| Data freshness | live tick staleness alert at the configured threshold; instrument master dated today; everything marked stale after wake until fresh | fake clock tests |
| Correctness | no look-ahead; Decimal for money; reference-value tests green | `bz-testing` suites |
| Safety | no order without Gate + approval; kill switch and lockout effective in one tick, across restarts and wake, with no UI open | pipeline property tests; restart and wake tests |
| Availability | runs while the Mac is awake; restart or wake keeps state and leaves no missed job silent; broker outage degrades analysis and paper, never crashes | scheduler, recovery and failing-adapter tests |
| Disk | Parquet size per symbol-year and SQLite growth recorded; retention and backup policy stated (backups of `data/` are the owner's, for example Time Machine) | `du` in notes |

These numbers are starting points for the first session; measure, then record real targets in `DECISIONS.md`.
