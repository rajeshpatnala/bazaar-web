# Top 10: dev-data (from 54 findings, 2026-11-23)
1. No DuckDB, file I/O or fsync inline on the async executor; use `spawn_blocking` [CR-111-1].
2. Driver errors are not all mapped to `Corrupt`; I/O, permission and validation get their own variants (a bad symbol is not `TooLarge`) [CR-112-1, CR-111-2].
3. Root and symbol checks reject glob characters and quotes, and SQLite and Parquet apply the same symbol rule [SEC-112-2, TS-F12-1].
4. Symbol length, batch size and read row count are capped [SEC-108-1].
5. Epoch/sequence input accepts only current or next; migration columns get CHECK constraints; mutable single-row state also writes a history row [CR-128-1, CR-128-3, SEC-123-1].
6. Chain tests cover removed middle row, swapped rows and corrupt row text; a mid-batch failure test reads back to prove nothing landed [TS-128-1, TS-128-3, CR-104-2].
7. `unwrap_or(None)` on calendar results hides OutOfRange; propagate it, and test a special session (muhurat) [QUAN-40-2, QUAN-40-3].
8. Bar aggregation: trailing incomplete bucket counts its gap, `adjusted` covers every minute in the bucket, chrono arithmetic is checked [QUAN-40-4, QUAN-40-6, SEC-40-2].
9. Parsers reject a record cut mid-column and duplicate keys/tokens, and count them [QUAN-33-1, QUAN-33-2].
10. Corrupt-row tests include unknown interval labels and out-of-range numeric text [TS-108-1].
