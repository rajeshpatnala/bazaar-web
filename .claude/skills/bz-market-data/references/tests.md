# 10. Tests (every PR needs unit tests)

(From `bz-market-data` SKILL.md; open when the task touches this area.)


- Fixture paths and tolerances come from the tree owned by `bz-testing` (`fixtures/kite/{rest,ws}/`, `fixtures/bhavcopy/`, `fixtures/config/`); do not invent other folders.
- **Binary parser:** hand-built frames as hex fixtures in `fixtures/kite/ws/` for each packet size (LTP, index, quote, full with depth), multi-packet frames, heartbeat, truncated and oversized lengths. `proptest`: arbitrary bytes never panic; encode-then-parse round trips for a test encoder.
- **Fake WebSocket server:** tokio listener on `127.0.0.1:0` that sends scripted frames, drops the connection, rejects auth, goes silent. Assert resubscription after reconnect, backoff growth (`tokio::time::pause`), `SessionExpired` on auth failure, conflation under a slow consumer.
- **REST:** `wiremock` serving recorded JSON/CSV from `fixtures/kite/rest/`: instruments gzip, historical with and without OI, 403 -> `SessionExpired`, 429 -> pacing, 5xx -> retry then `Unavailable`. Assert requests carry the version and auth headers without printing them.
- **Bhavcopy:** golden files (small real-shaped samples, one per layout, plus malformed, empty, BOM, CRLF); assert accepted and reject counts, idempotent re-import, snapshot (`insta`) of the normalised rows.
- **Corporate actions:** table-driven factor tests (split, bonus, dividend, stacked actions, action on the first bar), reference values computed by hand in comments.
- **Store:** `sqlx::test` ephemeral SQLite and a `tempfile` directory for Parquet; migration up-to-date test; DuckDB round trip with a `Decimal` price.
- **Fixtures** contain no tokens or account data; recording from the real service is a human task outside agent sessions, and agents otherwise craft minimal fixtures by hand.
