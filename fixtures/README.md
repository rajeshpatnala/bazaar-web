# fixtures

Test data. Nothing here is a market fact or a recording of a live service.

## config/

Eight twins of the shipped `config/*.toml` files, used by loader and calendar tests.

- Provenance: written by hand for tests; every `source` says "synthetic fixture".
- Values are fictional (session times, holidays, risk limits, watchlist symbols) and differ from
  `config/` on purpose, so a test cannot pass by reading the shipped file.
- `calendar.toml` covers: non-F&O and F&O sessions, an effective-dated closing-auction change, an
  `applies_to = "all"` row, a Thursday holiday, an equity-only holiday, a clearing holiday, a
  Saturday special session and a mock session. Covered year: 2026 only.
- Nothing was scrubbed because nothing was captured.
