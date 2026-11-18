# ## 12. Tests (all with fakes, temp dirs and in-process requests)

(From `bz-security` SKILL.md; open when the task touches this area.)


Build the router with a test constructor (fixed launch token, `dev` flag, `FakeClock`), drive it with `tower::ServiceExt::oneshot`, and always set `Host` (oneshot adds none).

- HTTP security tests (bind, launch token, session, Host, Origin, CSRF, CORS absence, headers, limits, Kite callback nonce, no secret in responses): the single table in `bz-testing` section 5.6, run against every OpenAPI route. Keep them green; add new cases there.
- Secrets: `SecretString` redaction; `resolve` prefers the store over env; a token from day D is rejected on D+1 IST (`FakeClock`); a 403 from the fake clears the token and blocks order sends.
- Validation: table tests per DTO (bad symbol, zero qty, off-tick price, unknown field, oversize list) return the closed error. Gate bypass, kill switch and audit chain tests as in `bz-execution`.
- Playwright smoke (`bz-testing`): chart screen renders with the production CSP and zero `securitypolicyviolation` events.
