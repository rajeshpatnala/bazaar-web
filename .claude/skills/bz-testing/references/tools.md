# 2. Tools

(From `bz-testing` SKILL.md; open when the task touches this area.)


| Tool | Use |
|---|---|
| `cargo-nextest` | Default runner (`.config/nextest.toml`); doc tests via `cargo test --doc --workspace`. |
| `proptest`, `insta`, `wiremock`, `tempfile` | Generators (commit `proptest-regressions/`); snapshots (review diffs, CI never updates); Kite REST stand-in; temp dirs. |
| `tower` (`util`), `http-body-util` | Dev-dependencies of `bazaar-server`: `ServiceExt::oneshot` and body collection for router tests. |
| `tokio-tungstenite` | Server side of the Kite ticker fake, and the client in server WebSocket tests. Keep the same minor as the one `axum` pulls in (`cargo tree -i tungstenite`) so there is one `Message` type. |
| `cargo-llvm-cov`, `cargo-mutants`, `criterion` | Coverage with nextest; periodic mutation run on `risk`, cost model, order state machine; benchmarks only. |
| Vitest 5, `@testing-library/react`, `user-event`, `jest-dom`, `@vitest/coverage-v8` | UI tests under jsdom. Config in `web/vitest.config.ts` (Vitest 5 does not search parent folders; `clearMocks` now defaults to true). |
| MSW 3 | REST and WebSocket mocks for Vitest (`msw/node`), `dev:mock` (`msw/browser`) and Playwright. Needs Node 22+. Details: `references/ui-mocked-api.md`. |
| Playwright (1.65 line) | Browser smoke against `dev:mock`, CI only. |

`.config/nextest.toml` (flaky tests are fixed, not retried):

```toml
[profile.default]
fail-fast = true

[profile.ci]
retries = 0
fail-fast = false
slow-timeout = { period = "30s", terminate-after = 4 }

[profile.ci.junit]
path = "junit.xml"
```
